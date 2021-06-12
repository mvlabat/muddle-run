use crate::{
    framebuffer::FrameNumber,
    game::components::{
        PlayerDirection, PlayerFrameSimulated, Position, PredictedPosition, Spawned,
    },
    messages::PlayerNetId,
    player::PlayerUpdates,
    registry::EntityRegistry,
    GameTime, SimulationTime, COMPONENT_FRAMEBUFFER_LIMIT, SIMULATIONS_PER_SECOND,
};
use bevy::{
    ecs::{
        entity::Entity,
        system::{Query, Res, ResMut},
    },
    log,
    math::Vec2,
    transform::components::Transform,
};
use bevy_rapier3d::{
    physics::RigidBodyHandleComponent,
    rapier::{dynamics::RigidBodySet, math::Vector},
};

/// Positions should align in half a second.
const LERP_FACTOR: f32 = 1.0 / SIMULATIONS_PER_SECOND as f32 * 2.0;

// The scaling factor for the player's linear velocity
const PLAYER_MOVEMENT_SPEED: f32 = 1.0;

pub fn read_movement_updates(
    time: Res<GameTime>,
    simulation_time: Res<SimulationTime>,
    mut player_updates: ResMut<PlayerUpdates>,
    player_registry: Res<EntityRegistry<PlayerNetId>>,
    mut players: Query<(
        Entity,
        &mut Position,
        &mut PlayerDirection,
        &Spawned,
        Option<&PlayerFrameSimulated>,
    )>,
) {
    for (entity, mut position, mut player_direction, spawned, player_frame_simulated) in
        players.iter_mut()
    {
        let player_net_id = player_registry
            .get_id(entity)
            .expect("Expected a registered player");

        let range = if player_frame_simulated.is_some() {
            simulation_time.player_frame..=time.frame_number
        } else {
            simulation_time.server_frame..=time.frame_number
        };
        log::trace!(
            "Reading updates for player {}: {:?}",
            player_net_id.0,
            range
        );
        for frame_number in range {
            if !spawned.is_spawned(frame_number) {
                continue;
            }

            if let Some(position_update) = player_updates
                .position
                .get(&player_net_id)
                .and_then(|buffer| buffer.get(frame_number))
                .and_then(|p| *p)
            {
                log::trace!(
                    "Position update for player {} (frame_number: {}): {:?}",
                    player_net_id.0,
                    frame_number,
                    position_update
                );
                position.buffer.insert(frame_number, position_update);
            } else {
                log::trace!(
                    "No updates for player {} (frame_number: {})",
                    player_net_id.0,
                    frame_number
                );
            }

            let direction_update = player_updates
                .get_direction_mut(player_net_id, frame_number, COMPONENT_FRAMEBUFFER_LIMIT)
                .get_mut(frame_number);
            // TODO: make sure that we don't leave all buffer filled with `None` (i.e. disconnect a player earlier).
            //  Document the implemented guarantees.
            let current_direction = player_direction
                .buffer
                .get(frame_number)
                .and_then(|update| *update);
            player_direction.buffer.insert(
                frame_number,
                direction_update
                    .and_then(|direction_update| {
                        direction_update.as_mut().map(|direction_update| {
                            if cfg!(feature = "client") {
                                direction_update.is_processed_client_input = Some(true);
                            }
                            direction_update.direction
                        })
                    })
                    // Avoid replacing initial updates with None.
                    .or(current_direction),
            );
        }
    }
}

type PlayersQuery<'a> = (
    Entity,
    &'a RigidBodyHandleComponent,
    &'a PlayerDirection,
    &'a Position,
    Option<&'a PlayerFrameSimulated>,
    &'a Spawned,
);

pub fn player_movement(
    time: Res<SimulationTime>,
    mut rigid_body_set: ResMut<RigidBodySet>,
    players: Query<PlayersQuery>,
) {
    log::trace!(
        "Moving players (frame {}, {})",
        time.server_frame,
        time.player_frame
    );
    for (entity, rigid_body, player_direction, position, player_frame_simulated, _) in players
        .iter()
        .filter(|(_, _, _, _, player_frame_simulated, spawned)| {
            spawned.is_spawned(time.entity_simulation_frame(*player_frame_simulated))
        })
    {
        let frame_number = time.entity_simulation_frame(player_frame_simulated);
        let rigid_body = rigid_body_set
            .get_mut(rigid_body.handle())
            .expect("expected a rigid body");

        let mut body_position = *rigid_body.position();
        let current_position = position.buffer.get(frame_number).unwrap_or_else(|| {
            // This can happen only if our `sync_position` haven't created a new position for
            // the current frame. If we are catching this, it's definitely a bug.
            panic!(
                "Expected player (entity: {:?}) position for frame {} (start frame: {}, len: {})",
                entity,
                time.entity_simulation_frame(player_frame_simulated),
                position.buffer.start_frame(),
                position.buffer.len()
            );
        });
        let wake_up = (body_position.translation.x - current_position.x).abs() > f32::EPSILON
            || (body_position.translation.y - current_position.y).abs() > f32::EPSILON;
        body_position.translation.x = current_position.x;
        body_position.translation.y = current_position.y;
        rigid_body.set_position(body_position, wake_up);

        let zero_vec = Vec2::new(0.0, 0.0);
        let (_, current_direction) = player_direction
            .buffer
            .get_with_extrapolation(frame_number)
            .unwrap_or_else(|| {
                if cfg!(debug_assertions) {
                    // We might have an edge-case when a client had been frozen for several seconds,
                    // didn't get any updates from a server, but failed to pause the game or
                    // disconnect. We want to avoid such cases (i.e. we want our clients to
                    // disconnect), but it's very difficult to catch every single one of them.
                    // In debug this scenario is unlikely, so we're probably catching some real
                    // bug, but in production we don't want our clients to panic.
                    panic!(
                        "Expected player (entity: {:?}) direction for frame {}",
                        entity, frame_number
                    )
                } else {
                    (FrameNumber::new(0), &zero_vec)
                }
            });
        let current_direction_norm = current_direction.normalize_or_zero() * PLAYER_MOVEMENT_SPEED;
        let wake_up = current_direction_norm.length_squared() > 0.0;
        rigid_body.set_linvel(
            Vector::new(current_direction_norm.x, current_direction_norm.y, 0.0),
            wake_up,
        );
    }
}

type SimulatedEntitiesQuery<'a> = (
    &'a RigidBodyHandleComponent,
    &'a mut Position,
    Option<&'a mut Transform>,
    Option<&'a mut PredictedPosition>,
    Option<&'a PlayerFrameSimulated>,
    &'a Spawned,
);

pub fn sync_position(
    game_time: Res<GameTime>,
    time: Res<SimulationTime>,
    mut rigid_body_set: ResMut<RigidBodySet>,
    mut simulated_entities: Query<SimulatedEntitiesQuery>,
) {
    log::trace!(
        "Syncing positions (frame {}, {})",
        time.server_frame,
        time.player_frame
    );
    for (
        rigid_body,
        mut position,
        mut transform,
        mut predicted_position,
        player_frame_simulated,
        _,
    ) in
        simulated_entities
            .iter_mut()
            .filter(|(_, _, _, _, player_frame_simulated, spawned)| {
                spawned.is_spawned(time.entity_simulation_frame(*player_frame_simulated))
            })
    {
        let frame_number = time.entity_simulation_frame(player_frame_simulated);
        let rigid_body = rigid_body_set
            .get_mut(rigid_body.handle())
            .expect("expected a rigid body");

        let body_position = *rigid_body.position();
        let new_position = Vec2::new(body_position.translation.x, body_position.translation.y);
        if let Some(predicted_position) = predicted_position.as_mut() {
            let current_position = *position
                .buffer
                .get(frame_number)
                .expect("Expected the current position");

            let needs_lerping_predicted_position = time.player_frame == game_time.frame_number;
            if needs_lerping_predicted_position {
                let real_diff = new_position - current_position;
                let new_predicted_position = predicted_position.value + real_diff;
                let lerp =
                    new_predicted_position + (new_position - new_predicted_position) * LERP_FACTOR;

                predicted_position.value = lerp;
                let transform = transform.as_mut().expect("Expected a Transform component if entity has PredictedPosition (is supposed to be a client)");
                transform.translation.x = lerp.x;
                transform.translation.y = lerp.y;
            }
        }

        // Positions buffer represents start positions before moving entities, so this is why
        // we save the new position in the next frame.
        position.buffer.insert(
            frame_number + FrameNumber::new(1),
            Vec2::new(body_position.translation.x, body_position.translation.y),
        );
    }
}
