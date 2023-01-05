#![feature(assert_matches)]
#![feature(slice_pattern)]
#![allow(clippy::only_used_in_recursion)]

pub use net::DEFAULT_SERVER_PORT;

use crate::{
    camera::{move_free_camera_pivot, reattach_camera},
    components::{CameraPivotDirection, CameraPivotTag},
    config_storage::OfflineAuthConfig,
    game_events::process_scheduled_spawns,
    input::{LevelObjectRequestsQueue, MouseRay, MouseWorldPosition, PlayerRequestsQueue},
    net::{
        auth::read_offline_auth_config, fill_actual_frames_ahead, init_matchmaker_connection,
        maintain_connection, process_network_events, send_network_updates, send_requests,
        ServerToConnect,
    },
    ui::{
        builder_ui::{EditedLevelObject, EditedObjectUpdate},
        debug_ui::update_debug_ui_state,
    },
    visuals::{
        control_builder_visibility, process_control_points_input, spawn_control_points,
        update_player_sensor_materials,
    },
};
use bevy::{
    app::{App, Plugin},
    diagnostic::FrameTimeDiagnosticsPlugin,
    ecs::{
        entity::Entity,
        schedule::{IntoSystemDescriptor, ShouldRun, State, StateError, SystemStage},
        system::{Commands, IntoSystem, Local, Res, ResMut, Resource, SystemParam},
    },
    hierarchy::BuildChildren,
    log,
    math::{Vec2, Vec3},
    pbr::{PointLight, PointLightBundle},
    prelude::Camera3dBundle,
    time::Time,
    transform::components::{GlobalTransform, Transform},
    utils::{HashMap, Instant},
};
use bevy_egui::EguiPlugin;
use bevy_inspector_egui::{WorldInspectorParams, WorldInspectorPlugin};
use bevy_inspector_egui_rapier::InspectableRapierPlugin;
use mr_shared_lib::{
    framebuffer::{FrameNumber, Framebuffer},
    game::client_factories::VisibilitySettings,
    messages::{EntityNetId, PlayerNetId},
    net::{ConnectionState, ConnectionStatus, MessageId},
    GameState, GameTime, MuddleSharedPlugin, SimulationTime, COMPONENT_FRAMEBUFFER_LIMIT,
    SIMULATIONS_PER_SECOND, TICKS_PER_NETWORK_BROADCAST,
};
use std::{marker::PhantomData, net::SocketAddr};
use url::Url;

mod camera;
mod components;
mod config_storage;
mod game_events;
mod helpers;
mod input;
mod net;
mod ui;
mod utils;
mod visuals;
mod websocket;

const TICKING_SPEED_FACTOR: u16 = 100;

pub struct MuddleClientPlugin;

impl Plugin for MuddleClientPlugin {
    fn build(&self, app: &mut App) {
        let input_stage = SystemStage::single_threaded()
            // Processing network events should happen before tracking input
            // because we reset current's player inputs on each delta update.
            .with_system(maintain_connection)
            .with_system(process_network_events.after(maintain_connection))
            .with_system(input::track_input_events.after(process_network_events))
            .with_system(input::cast_mouse_ray.after(input::track_input_events));
        let broadcast_updates_stage = SystemStage::single_threaded()
            .with_system(send_network_updates)
            .with_system(send_requests);
        let post_tick_stage = SystemStage::single_threaded()
            .with_system(control_builder_visibility)
            .with_system(update_player_sensor_materials)
            .with_system(reattach_camera)
            .with_system(move_free_camera_pivot.after(reattach_camera))
            .with_system(pause_simulation)
            .with_system(update_debug_ui_state.after(pause_simulation))
            .with_system(control_ticking_speed.after(pause_simulation))
            .with_system(fill_actual_frames_ahead.after(control_ticking_speed));

        app.add_plugin(bevy_mod_picking::PickingPlugin)
            .add_plugin(FrameTimeDiagnosticsPlugin)
            .add_plugin(EguiPlugin)
            .add_plugin(InspectableRapierPlugin)
            .add_plugin(WorldInspectorPlugin::new())
            .init_resource::<WindowInnerSize>()
            .init_resource::<input::MouseScreenPosition>()
            .add_event::<EditedObjectUpdate>()
            // Startup systems.
            .add_startup_system(init_matchmaker_connection)
            .add_startup_system(basic_scene)
            .add_startup_system(read_offline_auth_config)
            // Game.
            .add_plugin(MuddleSharedPlugin::new(
                IntoSystem::into_system(net_adaptive_run_criteria),
                input_stage,
                SystemStage::single_threaded(),
                broadcast_updates_stage,
                post_tick_stage,
                None,
            ))
            .add_system(process_scheduled_spawns)
            // Egui.
            .add_startup_system(ui::set_ui_scale_factor)
            .add_system(ui::debug_ui::update_debug_visibility)
            .add_system(ui::debug_ui::debug_ui)
            .add_system(ui::debug_ui::profiler_ui)
            .add_system(ui::overlay_ui::connection_status_overlay)
            .add_system(ui::debug_ui::inspect_object)
            .add_system(ui::player_ui::leaderboard_ui)
            .add_system(ui::player_ui::help_ui)
            .add_system(ui::main_menu_ui::main_menu_ui)
            // Not only Egui for builder mode.
            .add_system_set(ui::builder_ui::builder_system_set().label("builder_system_set"))
            // Add to the system set above after fixing https://github.com/mvlabat/muddle-run/issues/46.
            .add_system(process_control_points_input.after("builder_system_set"))
            .add_system(spawn_control_points.after("builder_system_set"));

        let world = &mut app.world;
        world
            .get_resource_mut::<WorldInspectorParams>()
            .unwrap()
            .enabled = false;
        world.get_resource_or_insert_with(InitialRtt::default);
        world.get_resource_or_insert_with(EstimatedServerTime::default);
        world.get_resource_or_insert_with(GameTicksPerSecond::default);
        world.get_resource_or_insert_with(TargetFramesAhead::default);
        world.get_resource_or_insert_with(DelayServerTime::default);
        world.get_resource_or_insert_with(ui::debug_ui::DebugUiState::default);
        world.get_resource_or_insert_with(CurrentPlayerNetId::default);
        world.get_resource_or_insert_with(ConnectionState::default);
        world.get_resource_or_insert_with(PlayerRequestsQueue::default);
        world.get_resource_or_insert_with(EditedLevelObject::default);
        world.get_resource_or_insert_with(LevelObjectRequestsQueue::default);
        world.get_resource_or_insert_with(LevelObjectCorrelations::default);
        world.get_resource_or_insert_with(MouseRay::default);
        world.get_resource_or_insert_with(MouseWorldPosition::default);
        world.get_resource_or_insert_with(VisibilitySettings::default);
        world.get_resource_or_insert_with(ServerToConnect::default);
        world.get_resource_or_insert_with(OfflineAuthConfig::default);
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
pub struct WindowInnerSize {
    pub width: usize,
    pub height: usize,
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
/// so that a clien receives updates in time before the simulation.
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

fn pause_simulation(
    mut game_state: ResMut<State<GameState>>,
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

    // We always assume that `GameState::Playing` is the initial state and
    // `GameState::Paused` is pushed to the top of the stack.
    if let GameState::Paused = game_state.current() {
        if is_connected && has_server_updates {
            let result = game_state.pop();
            match result {
                Ok(()) => {
                    log::info!("Unpausing the game");
                }
                Err(StateError::StateAlreadyQueued) => {
                    // TODO: investigate why this runs more than once before
                    // changing the state sometimes.
                }
                Err(StateError::StackEmpty | StateError::AlreadyInState) => unreachable!(),
            }
            return;
        }
    }

    if !is_connected || !has_server_updates {
        let result = game_state.push(GameState::Paused);
        match result {
            Ok(()) => {
                log::info!("Pausing the game");
            }
            Err(StateError::AlreadyInState) | Err(StateError::StateAlreadyQueued) => {
                // It's ok. Bevy won't let us push duplicate values - that's
                // what we rely on.
            }
            Err(StateError::StackEmpty) => unreachable!(),
        }
    }
}

fn basic_scene(mut commands: Commands) {
    // Add entities to the scene.
    commands.spawn(PointLightBundle {
        point_light: PointLight {
            range: 256.0,
            intensity: 1280000.0,
            ..Default::default()
        },
        transform: Transform::from_translation(Vec3::new(-64.0, -92.0, 144.0)),
        ..Default::default()
    });
    // Camera.
    let main_camera_entity = commands
        .spawn(Camera3dBundle {
            transform: Transform::from_translation(Vec3::new(-3.0, -14.0, 14.0))
                .looking_at(Vec3::default(), Vec3::Z),
            ..Default::default()
        })
        .insert(bevy_mod_picking::PickingCameraBundle::default())
        .id();
    let main_camera_pivot_entity = commands
        .spawn_empty()
        .insert(CameraPivotTag)
        .insert(CameraPivotDirection(Vec2::ZERO))
        .insert(Transform::IDENTITY)
        .insert(GlobalTransform::IDENTITY)
        .add_child(main_camera_entity)
        .id();
    commands.insert_resource(MainCameraPivotEntity(main_camera_pivot_entity));
    commands.insert_resource(MainCameraEntity(main_camera_entity));
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

fn control_ticking_speed(
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
    game_state: Res<State<GameState>>,
) -> ShouldRun {
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
    if game_state.current() != &GameState::Playing {
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
                ShouldRun::Yes
            } else {
                ShouldRun::YesAndCheckAgain
            }
        } else {
            state.started_looping_at = Some(Instant::now());
            ShouldRun::YesAndCheckAgain
        }
    } else {
        state.started_looping_at = None;
        ShouldRun::No
    }
}
