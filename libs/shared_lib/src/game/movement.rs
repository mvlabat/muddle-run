use crate::{
    game::components::{PlayerDirection, Position},
    messages::PlayerNetId,
    player::PlayerUpdates,
    registry::EntityRegistry,
    GameTime,
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

pub fn read_movement_updates(
    time: Res<GameTime>,
    player_updates: Res<PlayerUpdates>,
    player_registry: Res<EntityRegistry<PlayerNetId>>,
    mut players: Query<(Entity, &mut PlayerDirection)>,
) {
    for (entity, mut player_direction) in players.iter_mut() {
        let player_net_id = player_registry
            .get_id(entity)
            .expect("Expected a registered player");

        // TODO: we miss all updates if a player is at least 1 frame behind. We need to implement lag compensation.
        let direction = player_updates
            .updates
            .get(&player_net_id)
            .unwrap()
            .get(time.game_frame);
        // TODO: make sure that we don't leave all buffer filled with `None` (i.e. disconnect a player earlier).
        //  Document the implemented guarantees.
        player_direction
            .buffer
            .insert(time.game_frame, direction.and_then(|direction| *direction));
    }
}

pub fn player_movement(
    time: Res<GameTime>,
    mut rigid_body_set: ResMut<RigidBodySet>,
    players: Query<(&RigidBodyHandleComponent, &PlayerDirection, &Position)>,
) {
    log::trace!("Moving players (frame {})", time.game_frame);
    for (rigid_body, player_direction, position) in players.iter() {
        let rigid_body = rigid_body_set
            .get_mut(rigid_body.handle())
            .expect("expected a rigid body");

        let mut body_position = *rigid_body.position();
        let current_position = position
            .buffer
            .get(time.game_frame)
            .unwrap_or_else(|| panic!("Expected position for frame {}", time.game_frame));
        let wake_up = (body_position.translation.x - current_position.x).abs() > f32::EPSILON
            || (body_position.translation.z - current_position.y).abs() > f32::EPSILON;
        body_position.translation.x = current_position.x;
        body_position.translation.z = current_position.y;
        rigid_body.set_position(body_position, wake_up);

        let (_, current_direction) = player_direction
            .buffer
            .get_with_extrapolation(time.game_frame)
            .unwrap_or_else(|| panic!("Expected player direction for frame {}", time.game_frame));
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
    mut simulated_entities: Query<(&RigidBodyHandleComponent, &mut Position)>,
) {
    log::trace!("Syncing positions (frame {})", time.game_frame);
    for (rigid_body, mut position) in simulated_entities.iter_mut() {
        let rigid_body = rigid_body_set
            .get(rigid_body.handle())
            .expect("expected a rigid body");

        let body_position = *rigid_body.position();
        position.buffer.push(Vec2::new(
            body_position.translation.x,
            body_position.translation.z,
        ));
    }
}
