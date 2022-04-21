use crate::{
    framebuffer::FrameNumber, game::components::rotate, simulations_per_second, PLAYER_RADIUS,
    PLAYER_SENSOR_RADIUS,
};
use bevy::math::Vec2;
use bevy_rapier2d::rapier::geometry::TypedShape;
use rand::Rng;

pub fn player_respawn_time() -> FrameNumber {
    FrameNumber::new(simulations_per_second() * 3)
}

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

pub fn random_point_inside_shape(shape: TypedShape, object_radius: f32) -> Vec2 {
    let mut rng = rand::thread_rng();
    match shape {
        TypedShape::Ball(ball) => rotate(
            Vec2::new(
                rng.gen::<f32>() * (ball.radius - object_radius).max(0.0),
                0.0,
            ),
            rng.gen_range(0.0..std::f32::consts::PI * 2.0),
        ),
        TypedShape::Cuboid(cuboid) => {
            Vec2::new(
                rng.gen::<f32>() * (cuboid.half_extents.x * 2.0 - object_radius).max(0.0),
                rng.gen::<f32>() * (cuboid.half_extents.y * 2.0 - object_radius).max(0.0),
            ) - Vec2::from(cuboid.half_extents)
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

#[cfg(feature = "profiler")]
thread_local!(static PUFFIN_SCOPES: std::cell::RefCell<bevy::utils::HashMap<Box<dyn bevy::ecs::schedule::StageLabel>, puffin::ProfilerScope>> = std::cell::RefCell::new(bevy::utils::HashMap::default()));

#[cfg(feature = "profiler")]
pub fn profile_schedule(schedule: &mut bevy::ecs::schedule::Schedule) {
    use bevy::ecs::system::IntoExclusiveSystem;

    let stages = schedule
        .iter_stages()
        .map(|(stage_label, _)| stage_label.dyn_clone())
        .collect::<Vec<_>>();
    for stage_label in stages {
        let puffin_id: &'static str =
            Box::leak(format!("Stage {:?}", stage_label).into_boxed_str());
        let before_stage_label: &'static str =
            Box::leak(format!("puffin_before {:?}", stage_label).into_boxed_str());
        let after_stage_label: &'static str =
            Box::leak(format!("puffin_after {:?}", stage_label).into_boxed_str());
        let stage_label_to_remove = stage_label.dyn_clone();

        schedule.add_stage_before(
            stage_label.dyn_clone(),
            before_stage_label,
            bevy::ecs::schedule::SystemStage::parallel().with_system(
                (move |_world: &mut bevy::ecs::world::World| {
                    PUFFIN_SCOPES.with(|scopes| {
                        let mut scopes = scopes.borrow_mut();
                        scopes.insert(
                            stage_label.dyn_clone(),
                            puffin::ProfilerScope::new(puffin_id, puffin::current_file_name!(), ""),
                        );
                    });
                })
                .exclusive_system(),
            ),
        );
        schedule.add_stage_after(
            stage_label_to_remove.dyn_clone(),
            after_stage_label,
            bevy::ecs::schedule::SystemStage::parallel().with_system(
                (move |_world: &mut bevy::ecs::world::World| {
                    PUFFIN_SCOPES.with(|scopes| {
                        let mut scopes = scopes.borrow_mut();
                        scopes.remove(&stage_label_to_remove);
                    });
                })
                .exclusive_system(),
            ),
        );
    }
}
