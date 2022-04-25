use crate::{
    components::CameraPivotDirection, helpers, ui::debug_ui::DebugUiState, CurrentPlayerNetId,
    MainCameraEntity, MainCameraPivotEntity,
};
use bevy::{
    ecs::system::SystemParam,
    input::{keyboard::KeyboardInput, mouse::MouseButtonInput},
    log,
    prelude::*,
    render::camera::CameraProjection,
    utils::{HashMap, Instant},
};
use bevy_egui::EguiContext;
use bevy_inspector_egui::WorldInspectorParams;
use mr_shared_lib::{
    game::{components::Spawned, level::LevelObject},
    messages::{EntityNetId, PlayerNetId, SpawnLevelObjectRequest},
    player::{Player, PlayerDirectionUpdate, PlayerRole, PlayerUpdates},
    registry::EntityRegistry,
    GameTime, COMPONENT_FRAMEBUFFER_LIMIT,
};
use std::marker::PhantomData;

const SWITCH_ROLE_COOLDOWN_SECS: u64 = 1;

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
pub struct InputEvents<'w, 's> {
    pub keys: EventReader<'w, 's, KeyboardInput>,
    pub cursor: EventReader<'w, 's, CursorMoved>,
    pub mouse_button: EventReader<'w, 's, MouseButtonInput>,
}

/// Represents a cursor position in window coordinates (the ones that are coming from Window events).
#[derive(Default)]
pub struct MouseScreenPosition(pub Vec2);

/// MouseRay intersection with the (center=[0.0, 0.0, 0.0], normal=[0.0, 0.0, 1.0]) plane.
#[derive(Default)]
pub struct MouseWorldPosition(pub Vec2);

pub struct MouseRay {
    pub origin: Vec3,
    pub direction: Vec3,
}

impl Default for MouseRay {
    fn default() -> Self {
        Self {
            origin: Vec3::ZERO,
            direction: Vec3::ZERO,
        }
    }
}

#[derive(SystemParam)]
pub struct PlayerUpdatesParams<'w, 's> {
    switched_role_at: Local<'s, Option<Instant>>,
    current_player_net_id: Res<'w, CurrentPlayerNetId>,
    players: Res<'w, HashMap<PlayerNetId, Player>>,
    player_registry: Res<'w, EntityRegistry<PlayerNetId>>,
    players_query: Query<'w, 's, &'static Spawned>,
    main_camera_pivot_entity: Res<'w, MainCameraPivotEntity>,
    camera_query: Query<'w, 's, &'static mut CameraPivotDirection>,
    player_updates: ResMut<'w, PlayerUpdates>,
    player_requests: ResMut<'w, PlayerRequestsQueue>,
}

#[derive(SystemParam)]
pub struct UiParams<'w, 's> {
    egui_context: ResMut<'w, EguiContext>,
    debug_ui_state: ResMut<'w, DebugUiState>,
    #[system_param(ignore)]
    marker: PhantomData<&'s ()>,
}

pub fn track_input_events(
    mut input_events: InputEvents,
    time: Res<GameTime>,
    mut ui_params: UiParams,
    mut world_inspector_params: ResMut<WorldInspectorParams>,
    mut player_updates_params: PlayerUpdatesParams,
    mut mouse_position: ResMut<MouseScreenPosition>,
    keyboard_input: Res<Input<KeyCode>>,
) {
    #[cfg(feature = "profiler")]
    puffin::profile_function!();
    if ui_params.egui_context.ctx_mut().wants_keyboard_input() {
        return;
    }

    process_hotkeys(
        &keyboard_input,
        &mut ui_params.debug_ui_state,
        &mut world_inspector_params,
        &mut player_updates_params,
    );

    // Keyboard input.
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

    let current_player_is_spawned = player_updates_params
        .current_player_net_id
        .0
        .and_then(|net_id| player_updates_params.player_registry.get_entity(net_id))
        .and_then(|player_entity| player_updates_params.players_query.get(player_entity).ok())
        .map_or(false, |spawned| spawned.is_spawned(time.frame_number));
    let mut camera_pivot_direction = player_updates_params
        .camera_query
        .get_mut(player_updates_params.main_camera_pivot_entity.0)
        .expect("Expected the camera to initialize in `basic_scene`");
    // If a player is spawned, set its direction. Otherwise, update the free camera pivot.
    if current_player_is_spawned {
        let direction_updates = player_updates_params.player_updates.get_direction_mut(
            player_updates_params.current_player_net_id.0.unwrap(),
            time.frame_number,
            COMPONENT_FRAMEBUFFER_LIMIT,
        );
        direction_updates.insert(
            time.frame_number,
            Some(PlayerDirectionUpdate {
                direction,
                is_processed_client_input: Some(false),
            }),
        );
        camera_pivot_direction.0 = Vec2::ZERO;
    } else {
        camera_pivot_direction.0 = direction;
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
    mouse_position: Res<MouseScreenPosition>,
    main_camera_entity: Res<MainCameraEntity>,
    cameras: Query<(
        &GlobalTransform,
        &bevy::render::camera::Camera,
        &bevy::render::camera::PerspectiveProjection,
    )>,
    mut mouse_ray: ResMut<MouseRay>,
    mut mouse_world_position: ResMut<MouseWorldPosition>,
) {
    #[cfg(feature = "profiler")]
    puffin::profile_function!();
    let window = windows.iter().next().expect("expected a window");
    let (camera_transform, _camera, camera_projection) = cameras
        .get(main_camera_entity.0)
        .expect("expected a main camera");
    *mouse_ray = helpers::cursor_pos_to_ray(
        mouse_position.0,
        window,
        &camera_transform.compute_matrix(),
        &camera_projection.get_projection_matrix(),
    );

    mouse_world_position.0 = {
        let plane_normal = Vec3::Z;
        let denom = plane_normal.dot(mouse_ray.direction);
        if denom.abs() > f32::EPSILON {
            let t = (-mouse_ray.origin).dot(plane_normal) / denom;
            let i = mouse_ray
                .origin
                .lerp(mouse_ray.origin + mouse_ray.direction, t);
            Vec2::new(i.x, i.y)
        } else {
            Vec2::ZERO
        }
    };
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
        #[cfg(feature = "profiler")]
        puffin::set_scopes_on(debug_ui_state.show);
    }

    let net_id = player_updates_params.current_player_net_id.0;
    let player = net_id.and_then(|net_id| player_updates_params.players.get(&net_id));
    if let Some((_, player)) = net_id.zip(player) {
        let active_cooldown =
            player_updates_params
                .switched_role_at
                .map_or(false, |switched_role_at| {
                    Instant::now().duration_since(switched_role_at).as_secs()
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
            *player_updates_params.switched_role_at = Some(Instant::now());
        }
    }
}
