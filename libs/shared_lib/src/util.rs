use crate::{
    framebuffer::FrameNumber, game::components::rotate, PLAYER_RADIUS, PLAYER_SENSOR_RADIUS,
    SIMULATIONS_PER_SECOND,
};
use bevy::{
    ecs::{
        entity::Entity,
        query::{Fetch, FilterFetch, QueryEntityError, WorldQuery},
        system::Query,
    },
    math::Vec2,
};
use bevy_rapier2d::geometry::ColliderView;
use rand::Rng;

pub const PLAYER_RESPAWN_TIME: FrameNumber = FrameNumber::new(SIMULATIONS_PER_SECOND * 3);

pub fn player_sensor_outline() -> Vec<Vec2> {
    let sensors_count = 8;
    let step = std::f32::consts::PI * 2.0 / sensors_count as f32;
    (0..sensors_count)
        .map(|i| {
            rotate(
                Vec2::new(PLAYER_RADIUS - PLAYER_SENSOR_RADIUS, 0.0),
                step * i as f32,
            )
        })
        .collect()
}

pub fn random_point_inside_shape(shape: ColliderView, object_radius: f32) -> Vec2 {
    let mut rng = rand::thread_rng();
    match shape {
        ColliderView::Ball(ball) => rotate(
            Vec2::new(
                rng.gen::<f32>() * (ball.radius() - object_radius).max(0.0),
                0.0,
            ),
            rng.gen_range(0.0..std::f32::consts::PI * 2.0),
        ),
        ColliderView::Cuboid(cuboid) => {
            Vec2::new(
                rng.gen::<f32>() * (cuboid.half_extents().x * 2.0 - object_radius).max(0.0),
                rng.gen::<f32>() * (cuboid.half_extents().y * 2.0 - object_radius).max(0.0),
            ) - cuboid.half_extents()
        }
        _ => unimplemented!(),
    }
}

pub fn dedup_by_key_unsorted<T, F, K>(vec: &mut Vec<T>, mut key: F)
where
    F: FnMut(&T) -> K,
    K: PartialEq,
{
    let mut new = Vec::new();
    for el in std::mem::take(vec) {
        let el_key = key(&el);
        if !new.iter().any(|i| key(i) == el_key) {
            new.push(el);
        }
    }
    std::mem::swap(&mut new, vec);
}

#[track_caller]
pub fn get_item<'a, 'w, 's, Q: WorldQuery, F: WorldQuery>(
    query: &'a Query<'w, 's, Q, F>,
    entity: Entity,
) -> Option<<Q::ReadOnlyFetch as Fetch<'a, 's>>::Item>
where
    F::Fetch: FilterFetch,
{
    match query.get(entity) {
        Ok(item) => Some(item),
        err @ Err(QueryEntityError::AliasedMutability(_)) => {
            err.unwrap();
            unreachable!()
        }
        Err(QueryEntityError::QueryDoesNotMatch(_) | QueryEntityError::NoSuchEntity(_)) => None,
    }
}

#[track_caller]
pub fn get_item_mut<'a, Q: WorldQuery, F: WorldQuery>(
    query: &'a mut Query<Q, F>,
    entity: Entity,
) -> Option<<Q::Fetch as Fetch<'a, 'a>>::Item>
where
    F::Fetch: FilterFetch,
{
    match query.get_mut(entity) {
        Ok(item) => Some(item),
        err @ Err(QueryEntityError::AliasedMutability(_)) => {
            err.unwrap();
            unreachable!()
        }
        Err(QueryEntityError::QueryDoesNotMatch(_) | QueryEntityError::NoSuchEntity(_)) => None,
    }
}
