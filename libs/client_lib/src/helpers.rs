use crate::{input::MouseRay, CurrentPlayerNetId, MainCameraEntity};
use bevy::{
    ecs::{
        entity::Entity,
        system::{Local, Query, Res, SystemParam},
    },
    input::{mouse::MouseButton, Input},
    math::{Mat4, Vec2, Vec4},
    utils::HashMap,
    window::Window,
};
use mr_shared_lib::{messages::PlayerNetId, player::Player};

#[derive(SystemParam)]
pub struct PlayerParams<'a> {
    pub players: Res<'a, HashMap<PlayerNetId, Player>>,
    pub current_player_net_id: Res<'a, CurrentPlayerNetId>,
}

impl<'a> PlayerParams<'a> {
    pub fn current_player(&self) -> Option<&Player> {
        self.current_player_net_id
            .0
            .and_then(|net_id| self.players.get(&net_id))
    }
}

#[derive(SystemParam)]
pub struct MouseEntityPicker<'a> {
    picked_entity: Local<'a, Option<Entity>>,
    mouse_input: Res<'a, Input<MouseButton>>,
    camera_query: Query<'a, &'static bevy_mod_picking::PickingCamera>,
    camera_entity: Res<'a, MainCameraEntity>,
}

impl<'a> MouseEntityPicker<'a> {
    pub fn hovered_entity(&self) -> Option<Entity> {
        let picking_camera = self.camera_query.get(self.camera_entity.0).unwrap();
        picking_camera.intersect_top().map(|(entity, _)| entity)
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
) -> MouseRay {
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

    MouseRay {
        origin: camera_transform.w_axis.truncate(),
        direction: ray_world.normalize(),
    }
}
