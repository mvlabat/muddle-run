use crate::game::level::CollisionLogic;
use bevy::ecs::entity::Entity;

pub struct CollisionLogicChanged {
    pub level_object_entity: Entity,
    pub collision_logic: CollisionLogic,
}

/// Triggered for both the client and the server. Server should send a command
/// to respawn a player. Client may only provide visual feedback, such as
/// animations; respawning the player happens only on receiving `DeltaUpdate`
/// message that reflects that.
pub struct PlayerDeath(pub Entity);

/// Triggered for both the client and the server. Server should send a command
/// to respawn a player. Client may only provide visual feedback, such as
/// animations; respawning the player happens only on receiving `DeltaUpdate`
/// message that reflects that.
pub struct PlayerFinish(pub Entity);
