#![feature(assert_matches)]
#![feature(let_else)]
#![feature(slice_pattern)]

use crate::{
    camera::{move_free_camera_pivot, reattach_camera},
    components::{CameraPivotDirection, CameraPivotTag},
    config_storage::OfflineAuthConfig,
    game_events::process_scheduled_spawns,
    input::{LevelObjectRequestsQueue, MouseRay, MouseWorldPosition, PlayerRequestsQueue},
    net::{
        auth::read_offline_auth_config, init_matchmaker_connection, maintain_connection,
        process_network_events, send_network_updates, send_requests, ServerToConnect,
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
    core::Time,
    diagnostic::FrameTimeDiagnosticsPlugin,
    ecs::{
        entity::Entity,
        schedule::{ParallelSystemDescriptorCoercion, ShouldRun, State, StateError, SystemStage},
        system::{Commands, IntoSystem, Local, Res, ResMut, SystemParam},
    },
    log,
    math::{Vec2, Vec3},
    pbr::{PointLight, PointLightBundle},
    render::camera::PerspectiveCameraBundle,
    transform::components::{GlobalTransform, Parent, Transform},
    utils::{HashMap, Instant},
};
use bevy_egui::EguiPlugin;
use bevy_inspector_egui::{WorldInspectorParams, WorldInspectorPlugin};
use chrono::{DateTime, Utc};
use mr_shared_lib::{
    framebuffer::FrameNumber,
    game::client_factories::VisibilitySettings,
    messages::{EntityNetId, PlayerNetId},
    net::{ConnectionState, ConnectionStatus, MessageId},
    simulations_per_second, GameState, GameTime, MuddleSharedPlugin, SimulationTime,
    COMPONENT_FRAMEBUFFER_LIMIT,
};
use std::marker::PhantomData;

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

const TICKING_SPEED_FACTOR: u16 = 10;

pub struct MuddleClientPlugin;

impl Plugin for MuddleClientPlugin {
    fn build(&self, app: &mut App) {
        let input_stage = SystemStage::parallel()
            // Processing network events should happen before tracking input
            // because we reset current's player inputs on each delta update.
            .with_system(maintain_connection.label("connection"))
            .with_system(process_network_events.label("network").after("connection"))
            .with_system(input::track_input_events.label("input").after("network"))
            .with_system(input::cast_mouse_ray.after("input"));
        let broadcast_updates_stage = SystemStage::parallel()
            .with_system(send_network_updates)
            .with_system(send_requests);
        let post_tick_stage = SystemStage::parallel()
            .with_system(control_builder_visibility)
            .with_system(update_player_sensor_materials)
            .with_system(reattach_camera.label("reattach_camera"))
            .with_system(move_free_camera_pivot.after("reattach_camera"))
            .with_system(pause_simulation.label("pause_simulation"))
            .with_system(
                control_ticking_speed
                    .label("control_speed")
                    .after("pause_simulation"),
            )
            .with_system(update_debug_ui_state.after("pause_simulation"));

        app.add_plugin(bevy_mod_picking::PickingPlugin)
            .add_plugin(FrameTimeDiagnosticsPlugin)
            .add_plugin(EguiPlugin)
            .add_plugin(WorldInspectorPlugin::new())
            .init_resource::<WindowInnerSize>()
            .init_resource::<input::MouseScreenPosition>()
            .add_event::<EditedObjectUpdate>()
            // Startup systems.
            .add_startup_system(init_matchmaker_connection)
            .add_startup_system(init_state)
            .add_startup_system(basic_scene)
            .add_startup_system(read_offline_auth_config)
            // Game.
            .add_plugin(MuddleSharedPlugin::new(
                net_adaptive_timestamp.system(),
                input_stage,
                SystemStage::parallel(),
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

        #[cfg(feature = "profiler")]
        mr_shared_lib::util::profile_schedule(&mut app.schedule);

        let world = &mut app.world;
        world
            .get_resource_mut::<WorldInspectorParams>()
            .unwrap()
            .enabled = false;
        world.get_resource_or_insert_with(InitialRtt::default);
        world.get_resource_or_insert_with(EstimatedServerTime::default);
        world.get_resource_or_insert_with(GameTicksPerSecond::default);
        world.get_resource_or_insert_with(TargetFramesAhead::default);
        world.get_resource_or_insert_with(PlayerDelay::default);
        world.get_resource_or_insert_with(AdjustedSpeedReason::default);
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
        world.get_resource_or_insert_with(Option::<ServerToConnect>::default);
        world.get_resource_or_insert_with(OfflineAuthConfig::default);
    }
}

// Resources.
#[derive(Default)]
pub struct WindowInnerSize {
    pub width: usize,
    pub height: usize,
}

#[derive(Default)]
pub struct ExpectedFramesAhead {
    pub frames: FrameNumber,
}

#[derive(Default)]
pub struct InitialRtt {
    pub sent_at: Option<DateTime<Utc>>,
    pub received_at: Option<DateTime<Utc>>,
}

impl InitialRtt {
    pub fn duration_secs(&self) -> Option<f32> {
        self.sent_at
            .zip(self.received_at)
            .map(|(sent_at, received_at)| {
                received_at
                    .signed_duration_since(sent_at)
                    .to_std()
                    .unwrap()
                    .as_secs_f32()
            })
    }

    pub fn frames(&self) -> Option<FrameNumber> {
        self.duration_secs()
            .map(|duration| FrameNumber::new((simulations_per_second() as f32 * duration) as u16))
    }
}

#[derive(Default)]
/// This resource is used for adjusting game speed.
/// If the estimated `frame_number` falls too much behind, the game is paused.
pub struct EstimatedServerTime {
    pub updated_at: FrameNumber,
    pub frame_number: FrameNumber,
}

#[derive(Default)]
pub struct PlayerDelay {
    pub frame_count: i16,
}

#[derive(Default, Debug)]
pub struct TargetFramesAhead {
    /// Is always zero for the server.
    pub frames_count: FrameNumber,
}

pub struct GameTicksPerSecond {
    pub rate: u16,
}

impl Default for GameTicksPerSecond {
    fn default() -> Self {
        Self {
            rate: simulations_per_second(),
        }
    }
}

#[derive(Default)]
pub struct CurrentPlayerNetId(pub Option<PlayerNetId>);

#[derive(Default)]
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

pub struct MainCameraPivotEntity(pub Entity);

pub struct MainCameraEntity(pub Entity);

fn init_state(mut game_state: ResMut<State<GameState>>) {
    log::info!("Pausing the game");
    game_state.push(GameState::Paused).unwrap();
}

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

    // We always assume that `GameState::Playing` is the initial state and `GameState::Paused`
    // is pushed to the top of the stack.
    if let GameState::Paused = game_state.current() {
        if is_connected && has_server_updates {
            let result = game_state.pop();
            match result {
                Ok(()) => {
                    log::info!("Unpausing the game");
                }
                Err(StateError::StateAlreadyQueued) => {
                    // TODO: investigate why this runs more than once before changing the state sometimes.
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
                // It's ok. Bevy won't let us push duplicate values - that's what we rely on.
            }
            Err(StateError::StackEmpty) => unreachable!(),
        }
    }
}

fn basic_scene(mut commands: Commands) {
    // Add entities to the scene.
    commands.spawn_bundle(PointLightBundle {
        point_light: PointLight {
            range: 256.0,
            intensity: 1280000.0,
            ..Default::default()
        },
        transform: Transform::from_translation(Vec3::new(-64.0, -92.0, 144.0)),
        ..Default::default()
    });
    let main_camera_pivot_entity = commands
        .spawn()
        .insert(CameraPivotTag)
        .insert(CameraPivotDirection(Vec2::ZERO))
        .insert(Transform::identity())
        .insert(GlobalTransform::identity())
        .id();
    // Camera.
    let main_camera_entity = commands
        .spawn_bundle(PerspectiveCameraBundle {
            transform: Transform::from_translation(Vec3::new(-3.0, -14.0, 14.0))
                .looking_at(Vec3::default(), Vec3::Z),
            ..Default::default()
        })
        .insert_bundle(bevy_mod_picking::PickingCameraBundle::default())
        .insert(Parent(main_camera_pivot_entity))
        .id();
    commands.insert_resource(MainCameraPivotEntity(main_camera_pivot_entity));
    commands.insert_resource(MainCameraEntity(main_camera_entity));
}

#[derive(SystemParam)]
pub struct ControlTickingSpeedParams<'w, 's> {
    tick_rate: ResMut<'w, GameTicksPerSecond>,
    simulation_time: ResMut<'w, SimulationTime>,
    time: ResMut<'w, GameTime>,
    target_frames_ahead: Res<'w, TargetFramesAhead>,
    player_delay: ResMut<'w, PlayerDelay>,
    adjusted_speed_reason: ResMut<'w, AdjustedSpeedReason>,
    #[system_param(ignore)]
    marker: PhantomData<&'s ()>,
}

#[derive(Debug, Clone, Copy)]
pub enum AdjustedSpeedReason {
    SyncingFrames,
    ResizingServerInputBuffer,
    /// Means that speed isn't adjusted.
    None,
}

impl Default for AdjustedSpeedReason {
    fn default() -> Self {
        Self::None
    }
}

fn control_ticking_speed(
    mut frames_ticked: Local<u16>,
    mut prev_generation: Local<usize>,
    mut prev_tick_rate: Local<GameTicksPerSecond>,
    mut params: ControlTickingSpeedParams,
) {
    use std::cmp::Ordering;
    #[cfg(feature = "profiler")]
    puffin::profile_function!();

    let target_player_frame =
        params.simulation_time.server_frame + params.target_frames_ahead.frames_count;
    params.tick_rate.rate = match params
        .simulation_time
        .player_frame
        .cmp(&target_player_frame)
    {
        Ordering::Equal => {
            *params.adjusted_speed_reason = AdjustedSpeedReason::None;
            simulations_per_second()
        }
        Ordering::Greater => {
            *params.adjusted_speed_reason = AdjustedSpeedReason::ResizingServerInputBuffer;
            slower_tick_rate()
        }
        Ordering::Less => {
            *params.adjusted_speed_reason = AdjustedSpeedReason::ResizingServerInputBuffer;
            faster_tick_rate()
        }
    };

    if !matches!(
        *params.adjusted_speed_reason,
        AdjustedSpeedReason::ResizingServerInputBuffer
    ) {
        params.tick_rate.rate = match params.player_delay.frame_count.cmp(&0) {
            Ordering::Equal => {
                *params.adjusted_speed_reason = AdjustedSpeedReason::None;
                simulations_per_second()
            }
            Ordering::Greater => {
                *params.adjusted_speed_reason = AdjustedSpeedReason::SyncingFrames;
                faster_tick_rate()
            }
            Ordering::Less => {
                *params.adjusted_speed_reason = AdjustedSpeedReason::SyncingFrames;
                slower_tick_rate()
            }
        };
    }

    if prev_tick_rate.rate != params.tick_rate.rate || *prev_generation != params.time.session {
        *frames_ticked = 0;
    }

    if *frames_ticked == TICKING_SPEED_FACTOR {
        *frames_ticked = 0;
        match *params.adjusted_speed_reason {
            AdjustedSpeedReason::SyncingFrames => {
                if params.tick_rate.rate == faster_tick_rate() {
                    params.player_delay.frame_count -= 1;
                } else if params.tick_rate.rate == slower_tick_rate() {
                    params.player_delay.frame_count += 1;
                }
            }
            AdjustedSpeedReason::ResizingServerInputBuffer => {
                if params.tick_rate.rate == faster_tick_rate() {
                    params.simulation_time.server_frame -= FrameNumber::new(1);
                } else if params.tick_rate.rate == slower_tick_rate() {
                    params.simulation_time.player_frame -= FrameNumber::new(1);
                    params.time.frame_number -= FrameNumber::new(1);
                }
            }
            AdjustedSpeedReason::None => {}
        }
    }

    *frames_ticked += 1;
    prev_tick_rate.rate = params.tick_rate.rate;
    *prev_generation = params.time.session;
}

fn faster_tick_rate() -> u16 {
    assert!(
        simulations_per_second() % TICKING_SPEED_FACTOR == 0,
        "SIMULATIONS_PER_SECOND must a multiple of {}",
        TICKING_SPEED_FACTOR
    );
    simulations_per_second() + simulations_per_second() / TICKING_SPEED_FACTOR
}

fn slower_tick_rate() -> u16 {
    assert!(
        simulations_per_second() % TICKING_SPEED_FACTOR == 0,
        "SIMULATIONS_PER_SECOND must a multiple of {}",
        TICKING_SPEED_FACTOR
    );
    simulations_per_second() - simulations_per_second() / TICKING_SPEED_FACTOR
}

#[derive(Default, Clone)]
pub struct NetAdaptiveTimestempState {
    accumulator: f64,
    started_looping_at: Option<Instant>,
}

fn net_adaptive_timestamp(
    mut state: Local<NetAdaptiveTimestempState>,
    time: Res<Time>,
    game_ticks_per_second: Res<GameTicksPerSecond>,
) -> ShouldRun {
    #[cfg(feature = "profiler")]
    puffin::profile_function!();
    let rate = game_ticks_per_second.rate;
    let step = 1.0 / rate as f64;

    if state.started_looping_at.is_none() {
        state.accumulator += time.delta_seconds_f64();
    }

    if state.accumulator >= step {
        state.accumulator -= step;
        if let Some(started_looping_at) = state.started_looping_at {
            let secs_being_in_loop = Instant::now()
                .duration_since(started_looping_at)
                .as_secs_f32();
            let threshold_secs = 0.05; // 20 fsp
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
