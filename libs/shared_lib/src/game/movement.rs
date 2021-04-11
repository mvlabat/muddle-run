use crate::{
    framebuffer::FrameNumber,
    game::components::{PlayerDirection, PlayerFrameSimulated, Position, Spawned},
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
};
use bevy_rapier3d::{
    physics::RigidBodyHandleComponent,
    rapier::{dynamics::RigidBodySet, math::Vector},
};

/// Positions should align in half a second.
const LERP_FACTOR: f32 = 1.0 / SIMULATIONS_PER_SECOND as f32 * 2.0;

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

        let frames_ahead = simulation_time.player_frame - simulation_time.server_frame;
        let range = if player_frame_simulated.is_some() {
            simulation_time.player_frame..=time.frame_number
        } else {
            simulation_time.server_frame..=time.frame_number - frames_ahead
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
                let current_position = match position.buffer.get(frame_number) {
                    Some(position) => *position,
                    None => {
                        // There might be an edge-case that a client is slowing down
                        // `SimulationTime::player_frame` so that subtracting `frames_ahead` from
                        // `player_frame` (see how we calculate `range`, also see
                        // `control_ticking_speed`) results in an end frame number that we don't
                        // have a start position for.
                        break;
                    }
                };
                let lerp_position =
                    current_position + (position_update - current_position) * LERP_FACTOR;
                position.buffer.insert(frame_number, lerp_position);
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
                            if cfg!(feature = "render") {
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

pub fn player_movement(
    time: Res<SimulationTime>,
    mut rigid_body_set: ResMut<RigidBodySet>,
    players: Query<(
        &RigidBodyHandleComponent,
        &PlayerDirection,
        &Position,
        Option<&PlayerFrameSimulated>,
        &Spawned,
    )>,
) {
    log::trace!(
        "Moving players (frame {}, {})",
        time.server_frame,
        time.player_frame
    );
    for (rigid_body, player_direction, position, player_frame_simulated, _) in players
        .iter()
        .filter(|(_, _, _, player_frame_simulated, spawned)| {
            spawned.is_spawned(time.entity_simulation_frame(*player_frame_simulated))
        })
    {
        let frame_number = time.entity_simulation_frame(player_frame_simulated);
        let rigid_body = rigid_body_set
            .get_mut(rigid_body.handle())
            .expect("expected a rigid body");

        let mut body_position = *rigid_body.position();
        let current_position = position.buffer.get(frame_number).unwrap_or_else(|| {
            panic!(
                "Expected position for frame {} (start frame: {}, len: {})",
                time.entity_simulation_frame(player_frame_simulated),
                position.buffer.start_frame(),
                position.buffer.len()
            );
        });
        let wake_up = (body_position.translation.x - current_position.x).abs() > f32::EPSILON
            || (body_position.translation.z - current_position.y).abs() > f32::EPSILON;
        body_position.translation.x = current_position.x;
        body_position.translation.z = current_position.y;
        rigid_body.set_position(body_position, wake_up);

        let (_, current_direction) = player_direction
            .buffer
            .get_with_extrapolation(frame_number)
            .unwrap_or_else(|| panic!("Expected player direction for frame {}", frame_number));
        let wake_up = current_direction.length_squared() > 0.0;
        rigid_body.set_linvel(
            Vector::new(current_direction.x, 0.0, current_direction.y),
            wake_up,
        );
    }
}

pub fn sync_position(
    time: Res<SimulationTime>,
    rigid_body_set: Res<RigidBodySet>,
    mut simulated_entities: Query<(
        &RigidBodyHandleComponent,
        &mut Position,
        Option<&PlayerFrameSimulated>,
        &Spawned,
    )>,
) {
    log::trace!(
        "Syncing positions (frame {}, {})",
        time.server_frame,
        time.player_frame
    );
    for (rigid_body, mut position, player_frame_simulated, _) in simulated_entities
        .iter_mut()
        .filter(|(_, _, player_frame_simulated, spawned)| {
            spawned.is_spawned(time.entity_simulation_frame(*player_frame_simulated))
        })
    {
        let frame_number = time.entity_simulation_frame(player_frame_simulated);
        let rigid_body = rigid_body_set
            .get(rigid_body.handle())
            .expect("expected a rigid body");

        let body_position = *rigid_body.position();
        // Positions buffer represents start positions before moving entities, so this is why
        // we save the new position in the next frame.
        position.buffer.insert(
            frame_number + FrameNumber::new(1),
            Vec2::new(body_position.translation.x, body_position.translation.z),
        );
    }
}
