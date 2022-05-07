use crate::{
    framebuffer::FrameNumber,
    game::{
        components::{LevelObjectTag, Position},
        level::LevelState,
    },
    messages::EntityNetId,
    registry::EntityRegistry,
    util::random_point_inside_shape,
    PLAYER_RADIUS,
};
use bevy::{
    ecs::{
        query::With,
        system::{Query, Res, SystemParam},
    },
    math::Vec2,
};
use bevy_rapier2d::geometry::Collider;
use rand::seq::SliceRandom;

#[derive(SystemParam)]
pub struct LevelSpawnLocationService<'w, 's> {
    level_state: Res<'w, LevelState>,
    level_objects: Query<'w, 's, (&'static Position, &'static Collider), With<LevelObjectTag>>,
    entity_registry: Res<'w, EntityRegistry<EntityNetId>>,
}

impl<'w, 's> LevelSpawnLocationService<'w, 's> {
    pub fn spawn_position(&self, frame_number: FrameNumber) -> Vec2 {
        let available_shapes = self
            .level_state
            .spawn_areas
            .iter()
            .copied()
            .filter_map(|net_id| {
                self.entity_registry
                    .get_entity(net_id)
                    .and_then(|entity| self.level_objects.get(entity).ok())
            })
            .collect::<Vec<_>>();

        let (position, random_spawn_area) = match available_shapes.choose(&mut rand::thread_rng()) {
            Some((position, area)) => (*position, *area),
            None => return Vec2::ZERO,
        };

        *position
            .buffer
            .get(frame_number)
            .expect("Expected a position for existing level object")
            + random_point_inside_shape(random_spawn_area.as_typed_shape(), PLAYER_RADIUS)
    }
}
