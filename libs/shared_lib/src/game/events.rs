use crate::game::level::CollisionLogic;
use bevy::ecs::entity::Entity;

pub struct CollisionLogicChanged {
    pub level_object_entity: Entity,
    pub collision_logic: CollisionLogic,
}
