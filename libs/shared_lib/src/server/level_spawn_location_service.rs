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
use bevy_rapier2d::rapier::geometry::SharedShape;
use rand::seq::SliceRandom;

#[derive(SystemParam)]
pub struct LevelSpawnLocationService<'a> {
    level_state: Res<'a, LevelState>,
    level_objects: Query<'a, (&'static Position, &'static SharedShape), With<LevelObjectTag>>,
    entity_registry: Res<'a, EntityRegistry<EntityNetId>>,
}

impl<'a> LevelSpawnLocationService<'a> {
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
