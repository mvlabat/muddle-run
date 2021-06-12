use crate::{helpers, ui::debug_ui::DebugUiState, CurrentPlayerNetId, MainCameraEntity};
use bevy::{
    ecs::system::SystemParam,
    input::{keyboard::KeyboardInput, mouse::MouseButtonInput},
    log,
    prelude::*,
    render::camera::CameraProjection,
};
use bevy_inspector_egui::WorldInspectorParams;
use bevy_rapier3d::{na, rapier::geometry::Ray};
use chrono::{DateTime, Utc};
use mr_shared_lib::{
    game::level::LevelObject,
    messages::{EntityNetId, PlayerNetId, SpawnLevelObjectRequest},
    player::{Player, PlayerDirectionUpdate, PlayerRole, PlayerUpdates},
    GameTime, COMPONENT_FRAMEBUFFER_LIMIT,
};
use std::collections::HashMap;

const SWITCH_ROLE_COOLDOWN_SECS: i64 = 1;

/// Is drained by `send_requests`.
#[derive(Default)]
pub struct PlayerRequestsQueue {
    pub switch_role: Vec<PlayerRole>,
}

/// Is drained by `send_requests`.
#[derive(Default)]
pub struct LevelObjectRequestsQueue {
    pub spawn_requests: Vec<SpawnLevelObjectRequest>,
    pub update_requests: Vec<LevelObject>,
    pub despawn_requests: Vec<EntityNetId>,
}

#[derive(SystemParam)]
pub struct InputEvents<'a> {
    pub keys: EventReader<'a, KeyboardInput>,
    pub cursor: EventReader<'a, CursorMoved>,
    pub mouse_button: EventReader<'a, MouseButtonInput>,
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
pub struct PlayerUpdatesParams<'a> {
    switched_role_at: Local<'a, Option<DateTime<Utc>>>,
    current_player_net_id: Res<'a, CurrentPlayerNetId>,
    players: Res<'a, HashMap<PlayerNetId, Player>>,
    player_updates: ResMut<'a, PlayerUpdates>,
    player_requests: ResMut<'a, PlayerRequestsQueue>,
}

pub fn track_input_events(
    mut input_events: InputEvents,
    time: Res<GameTime>,
    mut debug_ui_state: ResMut<DebugUiState>,
    mut world_inspector_params: ResMut<WorldInspectorParams>,
    mut player_updates_params: PlayerUpdatesParams,
    mut mouse_position: ResMut<MousePosition>,
    keyboard_input: Res<Input<KeyCode>>,
) {
    process_hotkeys(
        &keyboard_input,
        &mut debug_ui_state,
        &mut world_inspector_params,
        &mut player_updates_params,
    );

    // Keyboard input.
    if let Some(player_net_id) = player_updates_params.current_player_net_id.0 {
        let direction_updates = player_updates_params.player_updates.get_direction_mut(
            player_net_id,
            time.frame_number,
            COMPONENT_FRAMEBUFFER_LIMIT,
        );
        let mut direction = Vec2::ZERO;
        if keyboard_input.pressed(KeyCode::A) || keyboard_input.pressed(KeyCode::Left) {
            direction.x -= 1.0;
        }
        if keyboard_input.pressed(KeyCode::D) || keyboard_input.pressed(KeyCode::Right) {
            direction.x += 1.0;
        }

        if keyboard_input.pressed(KeyCode::W) || keyboard_input.pressed(KeyCode::Up) {
            direction.y += 1.0;
        }
        if keyboard_input.pressed(KeyCode::S) || keyboard_input.pressed(KeyCode::Down) {
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
    for ev in input_events.keys.iter() {
        if ev.state.is_pressed() {
            log::trace!("Just pressed key: {:?}", ev.key_code);
        } else {
            log::trace!("Just released key: {:?}", ev.key_code);
        }
    }

    // Absolute cursor position (in window coordinates).
    if let Some(ev) = input_events.cursor.iter().next_back() {
        mouse_position.0 = ev.position;
    }

    // Mouse buttons.
    for ev in input_events.mouse_button.iter() {
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

fn process_hotkeys(
    keyboard_input: &Input<KeyCode>,
    debug_ui_state: &mut DebugUiState,
    world_inspector_params: &mut WorldInspectorParams,
    player_updates_params: &mut PlayerUpdatesParams,
) {
    if keyboard_input.just_pressed(KeyCode::Period) {
        debug_ui_state.show = !debug_ui_state.show;
        world_inspector_params.enabled = debug_ui_state.show;
    }

    let net_id = player_updates_params.current_player_net_id.0;
    let player = net_id.and_then(|net_id| player_updates_params.players.get(&net_id));
    if let Some((_, player)) = net_id.zip(player) {
        let active_cooldown =
            player_updates_params
                .switched_role_at
                .map_or(false, |switched_role_at| {
                    Utc::now()
                        .signed_duration_since(switched_role_at)
                        .num_seconds()
                        < SWITCH_ROLE_COOLDOWN_SECS
                });
        if keyboard_input.just_pressed(KeyCode::Escape) && !active_cooldown {
            let new_role = match player.role {
                PlayerRole::Runner => PlayerRole::Builder,
                PlayerRole::Builder => PlayerRole::Runner,
            };
            player_updates_params
                .player_requests
                .switch_role
                .push(new_role);
            *player_updates_params.switched_role_at = Some(Utc::now());
        }
    }
}
