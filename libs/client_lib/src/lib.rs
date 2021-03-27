use crate::{
    input::MouseRay,
    net::{initiate_connection, process_network_events, send_network_updates},
    ui::debug_ui::update_debug_ui_state,
};
use bevy::{
    app::{AppBuilder, Plugin},
    core::Time,
    diagnostic::FrameTimeDiagnosticsPlugin,
    ecs::{
        ArchetypeComponent, Commands, Entity, IntoSystem, Local, Res, ResMut, Resources, ShouldRun,
        System, SystemId, SystemParam, SystemStage, ThreadLocalExecution, TypeAccess, World,
    },
    math::Vec3,
    pbr::LightBundle,
    render::entity::Camera3dBundle,
    transform::components::Transform,
};
use bevy_egui::EguiPlugin;
use bevy_networking_turbulence::LinkConditionerConfig;
use mr_shared_lib::{
    framebuffer::FrameNumber, messages::PlayerNetId, net::ConnectionState, GameTime,
    MuddleSharedPlugin, SimulationTime, SIMULATIONS_PER_SECOND,
};
use std::{any::TypeId, borrow::Cow, time::Instant};

mod helpers;
mod input;
mod net;
mod ui;

const TICKING_SPEED_FACTOR: u16 = 10;

pub struct MuddleClientPlugin;

impl Plugin for MuddleClientPlugin {
    fn build(&self, builder: &mut AppBuilder) {
        let input_stage = SystemStage::serial()
            // Processing network events should happen before tracking input
            // because we reset current's player inputs on each delta update.
            .with_system(process_network_events.system())
            .with_system(input::track_input_events.system())
            .with_system(input::cast_mouse_ray.system());
        let broadcast_updates_stage =
            SystemStage::parallel().with_system(send_network_updates.system());
        let post_tick_stage = SystemStage::serial()
            .with_system(control_ticking_speed.system())
            .with_system(update_debug_ui_state.system());

        builder
            .add_plugin(FrameTimeDiagnosticsPlugin)
            .add_plugin(EguiPlugin)
            .init_resource::<WindowInnerSize>()
            .init_resource::<input::MousePosition>()
            // Startup systems.
            .add_startup_system(basic_scene.system())
            // Networking.
            .add_startup_system(initiate_connection.system())
            // Game.
            .add_plugin(MuddleSharedPlugin::new(
                NetAdaptiveTimestemp::default(),
                input_stage,
                broadcast_updates_stage,
                post_tick_stage,
                // None,
                Some(LinkConditionerConfig {
                    incoming_latency: 100,
                    incoming_jitter: 20,
                    incoming_loss: 0.0,
                    incoming_corruption: 0.0,
                }),
            ))
            // Egui.
            .add_system(ui::debug_ui::update_ui_scale_factor.system())
            .add_system(ui::debug_ui::debug_ui.system())
            .add_system(ui::debug_ui::inspect_object.system());

        let resources = builder.resources_mut();
        resources.get_or_insert_with(InitialRtt::default);
        resources.get_or_insert_with(EstimatedServerTime::default);
        resources.get_or_insert_with(GameTicksPerSecond::default);
        resources.get_or_insert_with(TargetFramesAhead::default);
        resources.get_or_insert_with(PlayerDelay::default);
        resources.get_or_insert_with(AdjustedSpeedReason::default);
        resources.get_or_insert_with(ui::debug_ui::DebugUiState::default);
        resources.get_or_insert_with(CurrentPlayerNetId::default);
        resources.get_or_insert_with(ConnectionState::default);
        resources.get_or_insert_with(MouseRay::default);
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
            .map(|duration| FrameNumber::new((SIMULATIONS_PER_SECOND as f32 * duration) as u16))
    }
}

#[derive(Default)]
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

fn basic_scene(commands: &mut Commands) {
    // Add entities to the scene.
    commands
        .spawn(LightBundle {
            transform: Transform::from_translation(Vec3::new(4.0, 10.0, -14.0)),
            ..Default::default()
        })
        // Camera.
        .spawn(Camera3dBundle {
            transform: Transform::from_translation(Vec3::new(5.0, 10.0, -14.0))
                .looking_at(Vec3::default(), Vec3::unit_y()),
            ..Default::default()
        });
    let main_camera_entity = commands.current_entity().unwrap();
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

pub struct NetAdaptiveTimestemp {
    accumulator: f64,
    looping: bool,
    system_id: SystemId,
    resource_access: TypeAccess<TypeId>,
    archetype_access: TypeAccess<ArchetypeComponent>,
}

impl Default for NetAdaptiveTimestemp {
    fn default() -> Self {
        Self {
            system_id: SystemId::new(),
            accumulator: 0.0,
            looping: false,
            resource_access: Default::default(),
            archetype_access: Default::default(),
        }
    }
}

impl NetAdaptiveTimestemp {
    pub fn update(&mut self, time: &Time, step: f64) -> ShouldRun {
        if !self.looping {
            self.accumulator += time.delta_seconds_f64();
        }

        if self.accumulator >= step {
            self.accumulator -= step;
            self.looping = true;
            ShouldRun::YesAndLoop
        } else {
            self.looping = false;
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
        self.system_id
    }

    fn update(&mut self, _world: &World) {}

    fn archetype_component_access(&self) -> &TypeAccess<ArchetypeComponent> {
        &self.archetype_access
    }

    fn resource_access(&self) -> &TypeAccess<TypeId> {
        &self.resource_access
    }

    fn thread_local_execution(&self) -> ThreadLocalExecution {
        ThreadLocalExecution::Immediate
    }

    unsafe fn run_unsafe(
        &mut self,
        _input: Self::In,
        _world: &World,
        resources: &Resources,
    ) -> Option<Self::Out> {
        let time = resources.get::<Time>().unwrap();
        let rate = resources.get::<GameTicksPerSecond>().unwrap().rate;
        let result = self.update(&time, 1.0 / rate as f64);
        Some(result)
    }

    fn run_thread_local(&mut self, _world: &mut World, _resources: &mut Resources) {}

    fn initialize(&mut self, _world: &mut World, _resources: &mut Resources) {
        self.resource_access.add_read(TypeId::of::<Time>());
        self.resource_access
            .add_read(TypeId::of::<GameTicksPerSecond>());
    }
}
