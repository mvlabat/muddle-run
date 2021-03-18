use crate::{
    framebuffer::FrameNumber,
    game::components::{PlayerDirection, Position, Spawned},
    messages::PlayerNetId,
    player::PlayerUpdates,
    registry::EntityRegistry,
    GameTime, COMPONENT_FRAMEBUFFER_LIMIT, SIMULATIONS_PER_SECOND,
};
use bevy::{
    ecs::{Entity, Query, Res, ResMut},
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
    mut player_updates: ResMut<PlayerUpdates>,
    player_registry: Res<EntityRegistry<PlayerNetId>>,
    mut players: Query<(Entity, &mut Position, &mut PlayerDirection)>,
) {
    for (entity, mut position, mut player_direction) in players.iter_mut() {
        let player_net_id = player_registry
            .get_id(entity)
            .expect("Expected a registered player");

        for frame_number in time.simulation_frame..=time.game_frame {
            if let Some(position_update) = player_updates
                .position
                .get(&player_net_id)
                .and_then(|buffer| buffer.get(time.game_frame))
                .and_then(|p| p.clone())
            {
                let current_position = *position
                    .buffer
                    .get(time.game_frame)
                    .expect("We always expect a start position for a current frame to exist");
                let lerp_position =
                    current_position + (position_update - current_position) * LERP_FACTOR;
                position.buffer.insert(time.game_frame, lerp_position);
            }

            let direction_update = player_updates
                .get_direction_mut(player_net_id, time.game_frame, COMPONENT_FRAMEBUFFER_LIMIT)
                .get_mut(frame_number);
            // TODO: make sure that we don't leave all buffer filled with `None` (i.e. disconnect a player earlier).
            //  Document the implemented guarantees.
            player_direction.buffer.insert(
                frame_number,
                direction_update.and_then(|direction_update| {
                    direction_update.as_mut().map(|direction_update| {
                        direction_update.is_processed_client_input = Some(true);
                        direction_update.direction
                    })
                }),
            );
        }
    }
}

pub fn player_movement(
    time: Res<GameTime>,
    mut rigid_body_set: ResMut<RigidBodySet>,
    players: Query<(
        &RigidBodyHandleComponent,
        &PlayerDirection,
        &Position,
        &Spawned,
    )>,
) {
    log::trace!("Moving players (frame {})", time.simulation_frame);
    for (rigid_body, player_direction, position, _) in players
        .iter()
        .filter(|(_, _, _, spawned)| spawned.is_spawned(time.simulation_frame))
    {
        let rigid_body = rigid_body_set
            .get_mut(rigid_body.handle())
            .expect("expected a rigid body");

        let mut body_position = *rigid_body.position();
        let current_position = position
            .buffer
            .get(time.simulation_frame)
            .unwrap_or_else(|| {
                panic!(
                    "Expected position for frame {} (start frame: {}, len: {})",
                    time.simulation_frame,
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
            .get_with_extrapolation(time.simulation_frame)
            .unwrap_or_else(|| {
                panic!(
                    "Expected player direction for frame {}",
                    time.simulation_frame
                )
            });
        let wake_up = current_direction.length_squared() > 0.0;
        rigid_body.set_linvel(
            Vector::new(current_direction.x, 0.0, current_direction.y),
            wake_up,
        );
    }
}

pub fn sync_position(
    time: Res<GameTime>,
    rigid_body_set: Res<RigidBodySet>,
    mut simulated_entities: Query<(&RigidBodyHandleComponent, &mut Position, &Spawned)>,
) {
    log::trace!("Syncing positions (frame {})", time.simulation_frame);
    for (rigid_body, mut position, _) in simulated_entities
        .iter_mut()
        .filter(|(_, _, spawned)| spawned.is_spawned(time.simulation_frame))
    {
        let rigid_body = rigid_body_set
            .get(rigid_body.handle())
            .expect("expected a rigid body");

        let body_position = *rigid_body.position();
        // Positions buffer represents start positions before moving entities, so this is why
        // we save the new position in the next frame.
        position.buffer.insert(
            time.simulation_frame + FrameNumber::new(1),
            Vec2::new(body_position.translation.x, body_position.translation.z),
        );
    }
}
