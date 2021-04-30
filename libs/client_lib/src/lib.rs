use crate::{
    input::MouseRay,
    net::{maintain_connection, process_network_events, send_network_updates},
    ui::debug_ui::update_debug_ui_state,
};
use bevy::{
    app::{AppBuilder, Plugin},
    core::Time,
    diagnostic::FrameTimeDiagnosticsPlugin,
    ecs::{
        archetype::{Archetype, ArchetypeComponentId},
        component::ComponentId,
        entity::Entity,
        query::Access,
        schedule::{ShouldRun, State, StateError, SystemStage},
        system::{Commands, IntoSystem, Local, Res, ResMut, System, SystemId, SystemParam},
        world::World,
    },
    log,
    math::Vec3,
    pbr::{Light, LightBundle},
    render::entity::PerspectiveCameraBundle,
    transform::components::Transform,
};
use bevy_egui::EguiPlugin;
use chrono::{DateTime, Utc};
use mr_shared_lib::{
    framebuffer::FrameNumber,
    messages::PlayerNetId,
    net::{ConnectionState, ConnectionStatus},
    GameState, GameTime, MuddleSharedPlugin, SimulationTime, COMPONENT_FRAMEBUFFER_LIMIT,
    SIMULATIONS_PER_SECOND,
};
use std::borrow::Cow;

mod helpers;
mod input;
mod net;
mod ui;

const TICKING_SPEED_FACTOR: u16 = 10;

pub struct MuddleClientPlugin;

impl Plugin for MuddleClientPlugin {
    fn build(&self, builder: &mut AppBuilder) {
        let input_stage = SystemStage::single_threaded()
            // Processing network events should happen before tracking input
            // because we reset current's player inputs on each delta update.
            .with_system(maintain_connection.system())
            .with_system(process_network_events.system())
            .with_system(input::track_input_events.system())
            .with_system(input::cast_mouse_ray.system());
        let broadcast_updates_stage =
            SystemStage::parallel().with_system(send_network_updates.system());
        let post_tick_stage = SystemStage::single_threaded()
            .with_system(pause_simulation.system())
            .with_system(control_ticking_speed.system())
            .with_system(update_debug_ui_state.system());

        builder
            .add_plugin(FrameTimeDiagnosticsPlugin)
            .add_plugin(EguiPlugin)
            .init_resource::<WindowInnerSize>()
            .init_resource::<input::MousePosition>()
            // Startup systems.
            .add_startup_system(init_state.system())
            .add_startup_system(basic_scene.system())
            // Game.
            .add_plugin(MuddleSharedPlugin::new(
                NetAdaptiveTimestemp::default(),
                input_stage,
                broadcast_updates_stage,
                post_tick_stage,
                None,
            ))
            // Egui.
            .add_system(ui::debug_ui::update_ui_scale_factor.system())
            .add_system(ui::debug_ui::debug_ui.system())
            .add_system(ui::overlay_ui::connection_status_overlay.system())
            .add_system(ui::debug_ui::inspect_object.system());

        let world = builder.world_mut();
        world.get_resource_or_insert_with(InitialRtt::default);
        world.get_resource_or_insert_with(EstimatedServerTime::default);
        world.get_resource_or_insert_with(GameTicksPerSecond::default);
        world.get_resource_or_insert_with(TargetFramesAhead::default);
        world.get_resource_or_insert_with(PlayerDelay::default);
        world.get_resource_or_insert_with(AdjustedSpeedReason::default);
        world.get_resource_or_insert_with(ui::debug_ui::DebugUiState::default);
        world.get_resource_or_insert_with(CurrentPlayerNetId::default);
        world.get_resource_or_insert_with(ConnectionState::default);
        world.get_resource_or_insert_with(MouseRay::default);
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
            .map(|duration| FrameNumber::new((SIMULATIONS_PER_SECOND as f32 * duration) as u16))
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
            rate: SIMULATIONS_PER_SECOND,
        }
    }
}

#[derive(Default)]
pub struct CurrentPlayerNetId(pub Option<PlayerNetId>);

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
    commands.spawn_bundle(LightBundle {
        light: Light {
            range: 200.0,
            intensity: 400.0,
            ..Default::default()
        },
        transform: Transform::from_translation(Vec3::new(4.0, 10.0, -14.0)),
        ..Default::default()
    });
    // Camera.
    let main_camera_entity = commands
        .spawn_bundle(PerspectiveCameraBundle {
            transform: Transform::from_translation(Vec3::new(5.0, 10.0, -14.0))
                .looking_at(Vec3::default(), Vec3::Y),
            ..Default::default()
        })
        .id();
    commands.insert_resource(MainCameraEntity(main_camera_entity));
}

#[derive(SystemParam)]
pub struct ControlTickingSpeedParams<'a> {
    tick_rate: ResMut<'a, GameTicksPerSecond>,
    simulation_time: ResMut<'a, SimulationTime>,
    time: ResMut<'a, GameTime>,
    target_frames_ahead: Res<'a, TargetFramesAhead>,
    player_delay: ResMut<'a, PlayerDelay>,
    adjusted_speed_reason: ResMut<'a, AdjustedSpeedReason>,
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

    let target_player_frame =
        params.simulation_time.server_frame + params.target_frames_ahead.frames_count;
    params.tick_rate.rate = match params
        .simulation_time
        .player_frame
        .cmp(&target_player_frame)
    {
        Ordering::Equal => {
            *params.adjusted_speed_reason = AdjustedSpeedReason::None;
            SIMULATIONS_PER_SECOND
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
                SIMULATIONS_PER_SECOND
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

    if prev_tick_rate.rate != params.tick_rate.rate || *prev_generation != params.time.generation {
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
    *prev_generation = params.time.generation;
}

fn faster_tick_rate() -> u16 {
    if SIMULATIONS_PER_SECOND % TICKING_SPEED_FACTOR != 0 {
        panic!(
            "SIMULATIONS_PER_SECOND must a multiple of {}",
            TICKING_SPEED_FACTOR
        );
    }
    SIMULATIONS_PER_SECOND + SIMULATIONS_PER_SECOND / TICKING_SPEED_FACTOR
}

fn slower_tick_rate() -> u16 {
    if SIMULATIONS_PER_SECOND % TICKING_SPEED_FACTOR != 0 {
        panic!(
            "SIMULATIONS_PER_SECOND must a multiple of {}",
            TICKING_SPEED_FACTOR
        );
    }
    SIMULATIONS_PER_SECOND - SIMULATIONS_PER_SECOND / TICKING_SPEED_FACTOR
}

#[derive(Default, Clone)]
pub struct NetAdaptiveTimestempState {
    accumulator: f64,
    looping: bool,
}

pub struct NetAdaptiveTimestemp {
    state: NetAdaptiveTimestempState,
    internal_system: Box<dyn System<In = (), Out = ShouldRun>>,
}

impl Default for NetAdaptiveTimestemp {
    fn default() -> Self {
        Self {
            state: NetAdaptiveTimestempState::default(),
            internal_system: Box::new(Self::prepare_system.system()),
        }
    }
}

impl NetAdaptiveTimestemp {
    fn prepare_system(
        mut state: Local<NetAdaptiveTimestempState>,
        time: Res<Time>,
        game_ticks_per_second: Res<GameTicksPerSecond>,
    ) -> ShouldRun {
        let rate = game_ticks_per_second.rate;
        let step = 1.0 / rate as f64;

        if !state.looping {
            state.accumulator += time.delta_seconds_f64();
        }

        if state.accumulator >= step {
            state.accumulator -= step;
            state.looping = true;
            ShouldRun::YesAndCheckAgain
        } else {
            state.looping = false;
            ShouldRun::No
        }
    }
}

impl System for NetAdaptiveTimestemp {
    type In = ();
    type Out = ShouldRun;

    fn name(&self) -> Cow<'static, str> {
        Cow::Borrowed(std::any::type_name::<NetAdaptiveTimestemp>())
    }

    fn id(&self) -> SystemId {
        self.internal_system.id()
    }

    fn new_archetype(&mut self, archetype: &Archetype) {
        self.internal_system.new_archetype(archetype);
    }

    fn component_access(&self) -> &Access<ComponentId> {
        self.internal_system.component_access()
    }

    fn archetype_component_access(&self) -> &Access<ArchetypeComponentId> {
        self.internal_system.archetype_component_access()
    }

    fn is_send(&self) -> bool {
        self.internal_system.is_send()
    }

    unsafe fn run_unsafe(&mut self, _input: Self::In, world: &World) -> Self::Out {
        self.internal_system.run_unsafe((), world)
    }

    fn apply_buffers(&mut self, world: &mut World) {
        self.internal_system.apply_buffers(world)
    }

    fn initialize(&mut self, world: &mut World) {
        self.internal_system = Box::new(
            Self::prepare_system
                .system()
                .config(|c| c.0 = Some(self.state.clone())),
        );
        self.internal_system.initialize(world);
    }

    fn check_change_tick(&mut self, change_tick: u32) {
        self.internal_system.check_change_tick(change_tick)
    }
}
