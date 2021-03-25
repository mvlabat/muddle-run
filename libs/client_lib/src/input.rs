use crate::{helpers, ui::debug_ui::DebugUiState, CurrentPlayerNetId, MainCameraEntity};
use bevy::{
    ecs::SystemParam,
    input::{keyboard::KeyboardInput, mouse::MouseButtonInput},
    log,
    prelude::*,
    render::camera::CameraProjection,
};
use bevy_rapier3d::{na, rapier::geometry::Ray};
use mr_shared_lib::{
    player::{PlayerDirectionUpdate, PlayerUpdates},
    GameTime, COMPONENT_FRAMEBUFFER_LIMIT,
};

#[derive(Default)]
pub struct EventReaderState {
    pub keys: EventReader<KeyboardInput>,
    pub cursor: EventReader<CursorMoved>,
    pub mouse_button: EventReader<MouseButtonInput>,
}

#[derive(Default)]
pub struct MousePosition(pub Vec2);

pub struct MouseRay(pub Ray);

impl Default for MouseRay {
    fn default() -> Self {
        Self(Ray::new(
            na::Point3::new(0.0, 0.0, 0.0),
            na::Vector3::new(0.0, 0.0, 0.0),
        ))
    }
}

#[derive(SystemParam)]
pub struct InputParams<'a> {
    keyboard_input: Res<'a, Input<KeyCode>>,
    ev_keys: Res<'a, Events<KeyboardInput>>,
    ev_cursor: Res<'a, Events<CursorMoved>>,
    ev_mouse_button: Res<'a, Events<MouseButtonInput>>,
}

pub fn track_input_events(
    mut event_reader_state: Local<EventReaderState>,
    time: Res<GameTime>,
    mut debug_ui_state: ResMut<DebugUiState>,
    mut player_updates: ResMut<PlayerUpdates>,
    current_player_net_id: Res<CurrentPlayerNetId>,
    mut mouse_position: ResMut<MousePosition>,
    input: InputParams,
) {
    if input.keyboard_input.just_pressed(KeyCode::Period) {
        debug_ui_state.show = !debug_ui_state.show;
    }

    // Keyboard input.
    if let Some(player_net_id) = current_player_net_id.0 {
        let direction_updates = player_updates.get_direction_mut(
            player_net_id,
            time.frame_number,
            COMPONENT_FRAMEBUFFER_LIMIT,
        );
        let mut direction = Vec2::zero();
        if input.keyboard_input.pressed(KeyCode::A) || input.keyboard_input.pressed(KeyCode::Left) {
            direction.x += 1.0;
        }
        if input.keyboard_input.pressed(KeyCode::D) || input.keyboard_input.pressed(KeyCode::Right)
        {
            direction.x -= 1.0;
        }

        if input.keyboard_input.pressed(KeyCode::W) || input.keyboard_input.pressed(KeyCode::Up) {
            direction.y += 1.0;
        }
        if input.keyboard_input.pressed(KeyCode::S) || input.keyboard_input.pressed(KeyCode::Down) {
            direction.y -= 1.0;
        }
        direction_updates.insert(
            time.frame_number,
            Some(PlayerDirectionUpdate {
                direction,
                is_processed_client_input: Some(false),
            }),
        );
    }
    for ev in event_reader_state.keys.iter(&input.ev_keys) {
        if ev.state.is_pressed() {
            log::trace!("Just pressed key: {:?}", ev.key_code);
        } else {
            log::trace!("Just released key: {:?}", ev.key_code);
        }
    }

    // Absolute cursor position (in window coordinates).
    if let Some(ev) = event_reader_state.cursor.latest(&input.ev_cursor) {
        mouse_position.0 = ev.position;
    }

    // Mouse buttons.
    for ev in event_reader_state.mouse_button.iter(&input.ev_mouse_button) {
        if ev.state.is_pressed() {
            log::trace!("Just pressed mouse button: {:?}", ev.button);
        } else {
            log::trace!("Just released mouse button: {:?}", ev.button);
        }
    }
}

pub fn cast_mouse_ray(
    windows: Res<Windows>,
    mouse_position: Res<MousePosition>,
    main_camera_entity: Res<MainCameraEntity>,
    cameras: Query<(
        &Transform,
        &bevy::render::camera::Camera,
        &bevy::render::camera::PerspectiveProjection,
    )>,
    mut mouse_ray: ResMut<MouseRay>,
) {
    let window = windows.iter().next().expect("expected a window");
    let (camera_transform, _camera, camera_projection) = cameras
        .get(main_camera_entity.0)
        .expect("expected a main camera");
    mouse_ray.0 = helpers::cursor_pos_to_ray(
        mouse_position.0,
        window,
        &camera_transform.compute_matrix(),
        &camera_projection.get_projection_matrix(),
    );
}
