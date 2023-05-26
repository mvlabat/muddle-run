#![feature(assert_matches)]
#![feature(slice_pattern)]
#![allow(clippy::only_used_in_recursion)]

pub use net::DEFAULT_SERVER_PORT;
pub use plugins::logging::MuddleTracePlugin;

use crate::{
    camera::{
        move_free_camera_pivot_system, reattach_camera_system, update_camera_transform_system,
    },
    config_storage::OfflineAuthConfig,
    game_events::process_scheduled_spawns_system,
    init_app_systems::load_shaders_system,
    input::{LevelObjectRequestsQueue, MouseRay, MouseWorldPosition, PlayerRequestsQueue},
    net::{
        auth::read_offline_auth_config_system, fill_actual_frames_ahead_system,
        has_server_to_connect, init_matchmaker_connection_system, maintain_connection_system,
        process_network_events_system, send_network_updates_system, send_requests_system,
        ServerToConnect, DEFAULT_SERVER_IP_ADDR,
    },
    ui::{
        builder_ui::{EditedLevelObject, EditedObjectUpdate},
        debug_ui::{update_debug_ui_state_system, DebugUiState},
        side_panel::OccupiedScreenSpace,
    },
    visuals::{
        control_builder_visibility_system, process_control_points_input_system,
        spawn_control_points_system, update_player_sensor_materials_system,
    },
};
use bevy::{
    app::{App, Plugin},
    diagnostic::FrameTimeDiagnosticsPlugin,
    ecs::{
        entity::Entity,
        schedule::{
            common_conditions::{in_state, not},
            Condition, IntoSystemConfig, IntoSystemConfigs, NextState, State, SystemSet,
        },
        system::{IntoSystem, Local, Res, ResMut, Resource, SystemParam},
    },
    log,
    time::Time,
    utils::{HashMap, Instant},
};
use bevy_egui::EguiPlugin;
use bevy_inspector_egui::quick::WorldInspectorPlugin;
use mr_shared_lib::{
    framebuffer::{FrameNumber, Framebuffer},
    game::client_factories::VisibilitySettings,
    messages::{EntityNetId, PlayerNetId},
    net::{ConnectionState, ConnectionStatus, MessageId},
    AppState, GameSessionState, GameTime, MuddleSharedPlugin, MuddleSystemConfigs, SimulationTime,
    COMPONENT_FRAMEBUFFER_LIMIT, SIMULATIONS_PER_SECOND, TICKS_PER_NETWORK_BROADCAST,
};
use std::{marker::PhantomData, net::SocketAddr};
use url::Url;

mod camera;
mod components;
mod config_storage;
mod game_events;
mod helpers;
mod init_app_systems;
mod input;
mod net;
mod plugins;
mod ui;
mod utils;
mod visuals;
mod websocket;

const TICKING_SPEED_FACTOR: u16 = 100;

pub struct MuddleClientPlugin;

#[derive(Debug, Hash, PartialEq, Eq, Clone, SystemSet)]
pub enum ClientSet {
    ProcessIoMessages,
    BuilderSystems,
}

impl Plugin for MuddleClientPlugin {
    fn build(&self, app: &mut App) {
        let config_server_addr = app
            .world
            .get_resource::<MuddleClientConfig>()
            .expect("Expected MuddleClientConfig to be initialised before MuddleClientPlugin")
            .server_addr
            .unwrap_or_else(|| SocketAddr::new(DEFAULT_SERVER_IP_ADDR, DEFAULT_SERVER_PORT))
            .to_string();

        let input_set = (
            maintain_connection_system.run_if(not(in_state(AppState::Loading))),
            // Processing network events should happen before tracking input:
            // we rely on resetting current's player inputs on each delta update message (event).
            process_network_events_system.after(maintain_connection_system),
            input::track_input_events_system.after(process_network_events_system),
            input::cast_mouse_ray_system.after(input::track_input_events_system),
        )
            .into_configs();
        let broadcast_updates_set =
            (send_network_updates_system, send_requests_system).into_configs();
        let post_tick_set = (
            control_builder_visibility_system,
            update_player_sensor_materials_system,
            reattach_camera_system,
            move_free_camera_pivot_system.after(reattach_camera_system),
            pause_simulation_system,
            update_debug_ui_state_system.after(pause_simulation_system),
            control_ticking_speed_system.after(pause_simulation_system),
            fill_actual_frames_ahead_system.after(control_ticking_speed_system),
        )
            .into_configs();

        app.add_plugin(bevy_mod_picking::PickingPlugin)
            .add_plugin(FrameTimeDiagnosticsPlugin)
            .add_plugin(EguiPlugin)
            // TODO: re-enable once Bevy 0.10 support is released.
            // .add_plugin(bevy_inspector_egui_rapier::InspectableRapierPlugin)
            .add_plugin(
                WorldInspectorPlugin::new()
                    .run_if(|debug_ui_state: Res<DebugUiState>| debug_ui_state.show),
            )
            .init_resource::<input::MouseScreenPosition>()
            .init_resource::<OccupiedScreenSpace>()
            .insert_resource(ui::main_menu_ui::MainMenuUiState::new(config_server_addr))
            .add_event::<EditedObjectUpdate>()
            // Startup systems.
            .add_startup_system(init_matchmaker_connection_system)
            .add_startup_system(init_app_systems::basic_scene_system)
            .add_startup_system(read_offline_auth_config_system)
            // Loading the app.
            .add_system(load_shaders_system.run_if(in_state(AppState::Loading)))
            // Game.
            .add_plugin(MuddleSharedPlugin::new(
                MuddleSystemConfigs {
                    main_run_criteria: IntoSystem::into_system(net_adaptive_run_criteria),
                    input_set,
                    post_game_set: ().into_configs(),
                    broadcast_updates_set,
                    post_tick_set,
                },
                None,
            ))
            .add_system(process_scheduled_spawns_system)
            // Egui.
            .add_startup_system(ui::set_ui_scale_factor_system)
            .add_system(update_camera_transform_system)
            .add_system(ui::debug_ui::update_debug_visibility_system)
            .add_system(ui::debug_ui::debug_ui_system)
            .add_system(ui::side_panel::side_panels_ui_system)
            .add_system(ui::overlay_ui::app_loading_ui.run_if(in_state(AppState::Loading)))
            .add_system(
                ui::overlay_ui::connection_status_overlay_system
                    .run_if(not(in_state(AppState::Loading))),
            )
            .add_system(ui::debug_ui::inspect_object_system)
            .add_system(
                ui::player_ui::leaderboard_ui_system
                    .run_if(not(in_state(GameSessionState::Loading))),
            )
            .add_system(
                ui::player_ui::help_ui_system.run_if(not(in_state(GameSessionState::Loading))),
            )
            .add_startup_system(ui::main_menu_ui::init_menu_auth_state_system)
            .add_systems(
                ui::main_menu_ui::process_io_messages_system_set()
                    .in_set(ClientSet::ProcessIoMessages),
            )
            .add_system(
                ui::main_menu_ui::main_menu_ui_system
                    .run_if(in_state(AppState::MainMenu).and_then(not(has_server_to_connect)))
                    .after(ClientSet::ProcessIoMessages),
            )
            // Builder mode systems.
            .add_systems(ui::builder_ui::builder_system_set().in_set(ClientSet::BuilderSystems))
            // Add to the system set above after fixing https://github.com/mvlabat/muddle-run/issues/46.
            .add_system(process_control_points_input_system.after(ClientSet::BuilderSystems))
            .add_system(spawn_control_points_system.after(ClientSet::BuilderSystems));

        app.init_resource::<InitialRtt>();
        app.init_resource::<EstimatedServerTime>();
        app.init_resource::<GameTicksPerSecond>();
        app.init_resource::<TargetFramesAhead>();
        app.init_resource::<DelayServerTime>();
        app.init_resource::<ui::debug_ui::DebugUiState>();
        app.init_resource::<CurrentPlayerNetId>();
        app.init_resource::<ConnectionState>();
        app.init_resource::<PlayerRequestsQueue>();
        app.init_resource::<EditedLevelObject>();
        app.init_resource::<LevelObjectRequestsQueue>();
        app.init_resource::<LevelObjectCorrelations>();
        app.init_resource::<MouseRay>();
        app.init_resource::<MouseWorldPosition>();
        app.init_resource::<VisibilitySettings>();
        app.init_resource::<ServerToConnect>();
        app.init_resource::<OfflineAuthConfig>();
    }
}

// Resources.

#[derive(Resource)]
pub struct MuddleClientConfig {
    pub persistence_url: Option<Url>,
    pub google_client_id: Option<String>,
    pub google_client_secret: Option<String>,
    pub auth0_client_id: Option<String>,
    pub matchmaker_url: Option<Url>,
    pub server_addr: Option<SocketAddr>,
}

#[derive(Resource, Default)]
pub struct ExpectedFramesAhead {
    pub frames: FrameNumber,
}

#[derive(Resource, Default)]
pub struct InitialRtt {
    pub sent_at: Option<Instant>,
    pub received_at: Option<Instant>,
}

impl InitialRtt {
    pub fn duration_secs(&self) -> Option<f32> {
        self.sent_at
            .zip(self.received_at)
            .map(|(sent_at, received_at)| received_at.duration_since(sent_at).as_secs_f32())
    }

    pub fn frames(&self) -> Option<FrameNumber> {
        self.duration_secs()
            .map(|duration| FrameNumber::new((SIMULATIONS_PER_SECOND * duration) as u16))
    }
}

/// We update this only when a client receives a fresh delta update.
/// If we see that a client is too far ahead of the server, we may pause the
/// game. See the `pause_simulation` system.
#[derive(Resource, Default)]
pub struct EstimatedServerTime {
    pub updated_at: FrameNumber,
    pub frame_number: FrameNumber,
}

/// If an incoming delta update comes later or earlier than `server_frame` (from
/// the `SimulationTime` resource), we update this value to let the clocks sync
/// so that a client receives updates in time before the simulation.
/// See the `sync_clock` function.
#[derive(Resource, Default)]
pub struct DelayServerTime {
    pub frame_count: i16,
}

/// If rtt between a client and a server changes, we need to change how much a
/// client is ahead of a server. See the `sync_clock` function.
#[derive(Resource)]
pub struct TargetFramesAhead {
    /// Stores the results of `SimulationTime::player_frames_ahead`.
    pub actual_frames_ahead: Framebuffer<u16>,
    pub target: u16,
    pub jitter_buffer_len: u16,
}

impl Default for TargetFramesAhead {
    fn default() -> Self {
        let buffer = Framebuffer::new(FrameNumber::new(0), 10_000 / TICKS_PER_NETWORK_BROADCAST);
        Self {
            actual_frames_ahead: buffer,
            target: 0,
            jitter_buffer_len: 0,
        }
    }
}

#[derive(Resource)]
pub struct GameTicksPerSecond {
    pub value: f32,
}

impl Default for GameTicksPerSecond {
    fn default() -> Self {
        Self {
            value: SIMULATIONS_PER_SECOND,
        }
    }
}

#[derive(Resource, Default)]
pub struct CurrentPlayerNetId(pub Option<PlayerNetId>);

#[derive(Resource, Default)]
pub struct LevelObjectCorrelations {
    correlations: HashMap<MessageId, EntityNetId>,
    last_correlation_id: MessageId,
}

impl LevelObjectCorrelations {
    pub fn next_correlation_id(&mut self) -> MessageId {
        let old = self.last_correlation_id;
        self.last_correlation_id += MessageId::new(1);
        old
    }

    pub fn correlate(&mut self, message_id: MessageId, entity_net_id: EntityNetId) {
        self.correlations.insert(message_id, entity_net_id);
    }

    pub fn query(&mut self, message_id: MessageId) -> Option<EntityNetId> {
        let entity_net_id = self.correlations.get(&message_id).copied();
        self.correlations.clear();
        entity_net_id
    }
}

#[derive(Resource)]
pub struct MainCameraPivotEntity(pub Entity);

#[derive(Resource)]
pub struct MainCameraEntity(pub Entity);

fn pause_simulation_system(
    mut next_game_session_state: ResMut<NextState<GameSessionState>>,
    game_session_state: Res<State<GameSessionState>>,
    connection_state: Res<ConnectionState>,
    game_time: Res<GameTime>,
    estimated_server_time: Res<EstimatedServerTime>,
) {
    #[cfg(feature = "profiler")]
    puffin::profile_function!();
    let is_connected = matches!(connection_state.status(), ConnectionStatus::Connected);

    let has_server_updates = game_time
        .frame_number
        .value()
        .saturating_sub(estimated_server_time.frame_number.value())
        < COMPONENT_FRAMEBUFFER_LIMIT / 2;

    if let GameSessionState::Paused = game_session_state.0 {
        if is_connected && has_server_updates {
            log::info!(
                "Changing the game session state to {:?}",
                GameSessionState::Playing
            );
            next_game_session_state.set(GameSessionState::Playing);
            return;
        }
    }

    // We can pause the game only when we are actually playing (not loading).
    if matches!(game_session_state.0, GameSessionState::Playing)
        && (!is_connected || !has_server_updates)
    {
        log::info!(
            "Changing the game session state to {:?}",
            GameSessionState::Paused
        );
        next_game_session_state.set(GameSessionState::Paused);
    }
}

#[derive(SystemParam)]
pub struct ControlTickingSpeedParams<'w, 's> {
    current_ticks_per_second: ResMut<'w, GameTicksPerSecond>,
    simulation_time: ResMut<'w, SimulationTime>,
    time: ResMut<'w, GameTime>,
    target_frames_ahead: Res<'w, TargetFramesAhead>,
    delay_server_time: ResMut<'w, DelayServerTime>,
    #[system_param(ignore)]
    marker: PhantomData<&'s ()>,
}

fn control_ticking_speed_system(
    mut frames_ticked: Local<u16>,
    mut prev_generation: Local<usize>,
    mut params: ControlTickingSpeedParams,
) {
    #[cfg(feature = "profiler")]
    puffin::profile_function!();

    let target_frames_ahead = params.target_frames_ahead.target;
    let actual_frames_ahead = params.simulation_time.player_frames_ahead();

    let frames_to_go_ahead = target_frames_ahead as i16 - actual_frames_ahead as i16;
    let new_ticks_per_second = if *frames_ticked < TICKING_SPEED_FACTOR {
        params.current_ticks_per_second.value
    } else if frames_to_go_ahead.abs() > params.delay_server_time.frame_count.abs() {
        if frames_to_go_ahead > 0 {
            ticks_per_second_faster()
        } else {
            ticks_per_second_slower()
        }
    } else if params.delay_server_time.frame_count > 0 {
        ticks_per_second_slower()
    } else if params.delay_server_time.frame_count < 0 {
        ticks_per_second_faster()
    } else {
        SIMULATIONS_PER_SECOND
    };
    params.current_ticks_per_second.value = new_ticks_per_second;

    let slow_down_server = params.delay_server_time.frame_count > 0;
    let speed_up_server = params.delay_server_time.frame_count < 0;
    let slow_down_player = frames_to_go_ahead < 0;
    let speed_up_player = frames_to_go_ahead > 0;

    #[cfg(debug_assertions)]
    log::trace!(
        "Server time: {}, player_time: {}, frames_to_go_ahead: {}, delay server: {}, ticks per second: {}, c: {}",
        params.simulation_time.server_frame,
        params.simulation_time.player_frame,
        frames_to_go_ahead,
        params.delay_server_time.frame_count,
        new_ticks_per_second,
        *frames_ticked,
    );

    if *frames_ticked == TICKING_SPEED_FACTOR / 2 {
        if params.current_ticks_per_second.value == ticks_per_second_faster()
            && slow_down_server
            && speed_up_player
        {
            params.delay_server_time.frame_count -= 1;
            params.simulation_time.server_frame -= FrameNumber::new(1);
        } else if params.current_ticks_per_second.value == ticks_per_second_slower()
            && speed_up_server
            && slow_down_player
        {
            params.simulation_time.player_frame -= FrameNumber::new(1);
        }
    } else if *frames_ticked == TICKING_SPEED_FACTOR {
        *frames_ticked = 0;
        if params.current_ticks_per_second.value == ticks_per_second_faster() {
            if speed_up_server {
                params.delay_server_time.frame_count += 1;
                if slow_down_player {
                    params.simulation_time.player_frame -= FrameNumber::new(1);
                }
            } else if speed_up_player {
                params.simulation_time.server_frame -= FrameNumber::new(1);
            } else {
                unreachable!("Can't speed up when neither the server time is delayed, nor the player time is behind the frames ahead target");
            }
        } else if params.current_ticks_per_second.value == ticks_per_second_slower() {
            if slow_down_server {
                params.delay_server_time.frame_count -= 1;
                if speed_up_player {
                    params.simulation_time.server_frame -= FrameNumber::new(1);
                }
            } else if slow_down_player {
                params.simulation_time.player_frame -= FrameNumber::new(1);
            } else {
                unreachable!("Can't slow down when neither the server time is ahead, nor the player time is ahead of the frames ahead target");
            }
        }
    }

    *frames_ticked += 1;
    *prev_generation = params.time.session;
}

#[allow(clippy::assertions_on_constants)]
fn ticks_per_second_faster() -> f32 {
    SIMULATIONS_PER_SECOND + SIMULATIONS_PER_SECOND / TICKING_SPEED_FACTOR as f32
}

#[allow(clippy::assertions_on_constants)]
fn ticks_per_second_slower() -> f32 {
    SIMULATIONS_PER_SECOND - SIMULATIONS_PER_SECOND / TICKING_SPEED_FACTOR as f32
}

#[derive(Default, Clone)]
pub struct NetAdaptiveRunCriteriaState {
    accumulator: f64,
    started_looping_at: Option<Instant>,
}

fn net_adaptive_run_criteria(
    mut state: Local<NetAdaptiveRunCriteriaState>,
    time: Res<Time>,
    game_ticks_per_second: Res<GameTicksPerSecond>,
    game_state: Res<State<GameSessionState>>,
) -> bool {
    #[cfg(feature = "profiler")]
    puffin::profile_function!();
    // See `control_ticking_speed` for the rate value changes.
    let rate = game_ticks_per_second.value;
    let step = 1.0 / rate as f64;

    // If it's the first run after the previous render (or it's the first run ever),
    // we add the delta to the accumulator and start looping while it's higher
    // or equals the step.
    if state.started_looping_at.is_none() {
        state.accumulator += time.delta_seconds_f64();
    }

    // In the scenario when a client was frozen (minimized, for example) and it got
    // disconnected, we don't want to replay all the accumulated frames.
    if game_state.0 != GameSessionState::Playing {
        state.accumulator = state.accumulator.min(1.0);
    }

    if state.accumulator >= step {
        state.accumulator -= step;
        if let Some(started_looping_at) = state.started_looping_at {
            let secs_being_in_loop = Instant::now()
                .duration_since(started_looping_at)
                .as_secs_f32();
            // We can't afford running game logic for too long without rendering, so we
            // target at least 20 fps.
            let threshold_secs = 0.05;
            if secs_being_in_loop > threshold_secs {
                state.started_looping_at = None;
                true
            } else {
                true
            }
        } else {
            state.started_looping_at = Some(Instant::now());
            true
        }
    } else {
        state.started_looping_at = None;
        false
    }
}
