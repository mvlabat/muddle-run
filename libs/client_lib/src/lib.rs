use crate::{
    input::MouseRay,
    net::{initiate_connection, process_network_events, send_network_updates},
};
use bevy::{
    diagnostic::FrameTimeDiagnosticsPlugin,
    ecs::{ArchetypeComponent, ShouldRun, SystemId, ThreadLocalExecution, TypeAccess},
    prelude::*,
};
use bevy_egui::EguiPlugin;
use mr_shared_lib::{
    framebuffer::FrameNumber, messages::PlayerNetId, net::ConnectionState, GameTicksPerSecond,
    MuddleSharedPlugin, SIMULATIONS_PER_SECOND,
};
use std::{any::TypeId, borrow::Cow, time::Instant};

mod helpers;
mod input;
mod net;
mod ui;

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
                None,
            ))
            // Egui.
            .add_system(ui::debug_ui::update_ui_scale_factor.system())
            .add_system(ui::debug_ui::debug_ui.system())
            .add_system(ui::debug_ui::inspect_object.system());

        let resources = builder.resources_mut();
        resources.get_or_insert_with(InitialRtt::default);
        resources.get_or_insert_with(EstimatedServerTime::default);
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
    pub frame_number: FrameNumber,
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
