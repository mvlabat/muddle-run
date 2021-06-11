use crate::input::MouseRay;
use bevy::{
    ecs::{
        entity::Entity,
        system::{Local, Query, Res, SystemParam},
    },
    input::{mouse::MouseButton, Input},
    log,
    math::{Mat4, Vec2, Vec4},
    window::Window,
};
use bevy_rapier3d::{
    physics::ColliderHandleComponent,
    rapier::{
        geometry::{ColliderSet, InteractionGroups, Ray},
        na,
        pipeline::QueryPipeline,
    },
};

#[derive(SystemParam)]
pub struct MouseEntityPicker<'a> {
    picked_entity: Local<'a, Option<Entity>>,
    colliders: Query<'a, (Entity, &'static ColliderHandleComponent)>,
    mouse_input: Res<'a, Input<MouseButton>>,
    mouse_ray: Res<'a, MouseRay>,
    query_pipeline: Res<'a, QueryPipeline>,
    collider_set: Res<'a, ColliderSet>,
}

impl<'a> MouseEntityPicker<'a> {
    pub fn hovered_entity(&self) -> Option<Entity> {
        if let Some((collider, _)) = self.query_pipeline.cast_ray(
            &self.collider_set,
            &self.mouse_ray.0,
            f32::MAX,
            true,
            InteractionGroups::all(),
            None,
        ) {
            if let Some((entity, _)) = self
                .colliders
                .iter()
                .find(|(_, collider_component)| collider_component.handle() == collider)
            {
                return Some(entity);
            } else {
                // TODO: this does happen. We need to investigate.
                //  (Might appear in some weird edge-cases when restarting a client or a server.)
                log::error!("No entity with collider {:?} was found", collider);
            }
        }
        None
    }

    pub fn pick_entity(&mut self) {
        if self.mouse_input.just_pressed(MouseButton::Left) {
            *self.picked_entity = self.hovered_entity();
        }
    }

    pub fn picked_entity(&self) -> Option<Entity> {
        *self.picked_entity
    }

    pub fn take_picked_entity(&mut self) -> Option<Entity> {
        self.picked_entity.take()
    }
}

// Heavily inspired by https://github.com/bevyengine/bevy/pull/432/.
pub fn cursor_pos_to_ray(
    cursor_viewport: Vec2,
    window: &Window,
    camera_transform: &Mat4,
    camera_perspective: &Mat4,
) -> Ray {
    // Calculate the cursor pos in NDC space [(-1,-1), (1,1)].
    let cursor_ndc = Vec4::from((
        (cursor_viewport.x / window.width() as f32) * 2.0 - 1.0,
        (cursor_viewport.y / window.height() as f32) * 2.0 - 1.0,
        -1.0, // let the cursor be on the far clipping plane
        1.0,
    ));

    let object_to_world = camera_transform;
    let object_to_ndc = camera_perspective;

    // Transform the cursor position into object/camera space. This also turns the cursor into
    // a vector that's pointing from the camera center onto the far plane.
    let mut ray_camera = object_to_ndc.inverse().mul_vec4(cursor_ndc);
    ray_camera.z = -1.0;
    ray_camera.w = 0.0; // treat the vector as a direction (0 = Direction, 1 = Position)

    // Transform the cursor into world space.
    let ray_world = object_to_world.mul_vec4(ray_camera);
    let ray_world = ray_world.truncate();

    let camera_pos = camera_transform.w_axis.truncate();
    let camera_pos = na::Point3::new(camera_pos.x, camera_pos.y, camera_pos.z);
    Ray::new(
        camera_pos,
        na::Vector3::from_row_slice(ray_world.normalize().as_ref()),
    )
}
