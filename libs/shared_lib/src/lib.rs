#![feature(step_trait)]
#![feature(step_trait_ext)]
#![feature(trait_alias)]

use crate::{
    framebuffer::FrameNumber,
    game::{
        commands::{
            DespawnLevelObject, DespawnPlayer, GameCommands, SpawnLevelObject, SpawnPlayer,
        },
        components::PlayerFrameSimulated,
        level::LevelState,
        movement::{player_movement, read_movement_updates, sync_position},
        spawn::{despawn_players, process_spawned_entities, spawn_level_objects, spawn_players},
    },
    net::network_setup,
    player::{Player, PlayerUpdates},
    registry::EntityRegistry,
};
use bevy::{
    ecs::{ArchetypeComponent, ShouldRun, SystemId, ThreadLocalExecution, TypeAccess},
    log,
    prelude::*,
};
use bevy_networking_turbulence::{LinkConditionerConfig, NetworkingPlugin};
use bevy_rapier3d::{
    physics,
    physics::{
        EntityMaps, EventQueue, InteractionPairFilters, RapierConfiguration, SimulationToRenderTime,
    },
    rapier::{
        dynamics::{IntegrationParameters, JointSet, RigidBodySet},
        geometry::{
            BroadPhase, ColliderSet, ContactPairFilter, IntersectionPairFilter, NarrowPhase,
            PairFilterContext, SolverFlags,
        },
        math::Vector,
        pipeline::{PhysicsPipeline, QueryPipeline},
    },
};
use messages::{EntityNetId, PlayerNetId};
use std::{any::TypeId, borrow::Cow, collections::HashMap, sync::Mutex};

pub mod framebuffer;
pub mod game;
pub mod looped_counter;
pub mod messages;
pub mod net;
pub mod player;
pub mod registry;
pub mod util;

// Constants.
pub mod stage {
    pub const WRITE_INPUT_UPDATES: &str = "mr_shared_write_input_updates";

    pub const MAIN_SCHEDULE: &str = "mr_shared_main_schedule";
    pub const READ_INPUT_UPDATES: &str = "mr_shared_read_input_updates";
    pub const BROADCAST_UPDATES: &str = "mr_shared_broadcast_updates";
    pub const POST_SIMULATIONS: &str = "mr_shared_post_simulations";
    pub const POST_TICK: &str = "mr_shared_post_tick";

    pub const SIMULATION_SCHEDULE: &str = "mr_shared_simulation_schedule";
    pub const SPAWN: &str = "mr_shared_spawn";
    pub const PRE_GAME: &str = "mr_shared_pre_game";
    pub const GAME: &str = "mr_shared_game";
    pub const PHYSICS: &str = "mr_shared_physics";
    pub const POST_PHYSICS: &str = "mr_shared_post_physics";
    pub const POST_GAME: &str = "mr_shared_post_game";
}
pub const PLAYER_SIZE: f32 = 1.0;
pub const PLANE_SIZE: f32 = 20.0;
pub const SIMULATIONS_PER_SECOND: u16 = 120;
pub const COMPONENT_FRAMEBUFFER_LIMIT: u16 = 120 * 10; // 10 seconds of 120fps
pub const TICKS_PER_NETWORK_BROADCAST: u16 = 2;

pub struct MuddleSharedPlugin<S: System<In = (), Out = ShouldRun>> {
    main_run_criteria: Mutex<Option<S>>,
    input_stage: Mutex<Option<SystemStage>>,
    broadcast_updates_stage: Mutex<Option<SystemStage>>,
    post_tick_stage: Mutex<Option<SystemStage>>,
    link_conditioner: Option<LinkConditionerConfig>,
}

impl<S: System<In = (), Out = ShouldRun>> MuddleSharedPlugin<S> {
    pub fn new(
        main_run_criteria: S,
        input_stage: SystemStage,
        broadcast_updates_stage: SystemStage,
        post_tick_stage: SystemStage,
        link_conditioner: Option<LinkConditionerConfig>,
    ) -> Self {
        Self {
            main_run_criteria: Mutex::new(Some(main_run_criteria)),
            input_stage: Mutex::new(Some(input_stage)),
            broadcast_updates_stage: Mutex::new(Some(broadcast_updates_stage)),
            post_tick_stage: Mutex::new(Some(post_tick_stage)),
            link_conditioner,
        }
    }
}

impl<S: System<In = (), Out = ShouldRun>> Plugin for MuddleSharedPlugin<S> {
    fn build(&self, builder: &mut AppBuilder) {
        builder.add_plugin(RapierResourcesPlugin);
        builder.add_plugin(NetworkingPlugin {
            link_conditioner: self.link_conditioner.clone(),
        });

        let mut main_run_criteria = self
            .main_run_criteria
            .lock()
            .expect("Can't initialize the plugin more than once");
        let mut input_stage = self
            .input_stage
            .lock()
            .expect("Can't initialize the plugin more than once");
        let mut broadcast_updates_stage = self
            .broadcast_updates_stage
            .lock()
            .expect("Can't initialize the plugin more than once");
        let mut post_tick_stage = self
            .post_tick_stage
            .lock()
            .expect("Can't initialize the plugin more than once");

        let simulation_schedule = Schedule::default()
            .with_run_criteria(SimulationTickRunCriteria::default())
            .with_stage(
                stage::SPAWN,
                SystemStage::serial()
                    .with_system(despawn_players.system())
                    .with_system(spawn_players.system())
                    .with_system(spawn_level_objects.system()),
            )
            .with_stage(
                stage::PRE_GAME,
                SystemStage::parallel()
                    .with_system(physics::create_body_and_collider_system.system())
                    .with_system(physics::create_joints_system.system()),
            )
            .with_stage(
                stage::GAME,
                SystemStage::parallel().with_system(player_movement.system()),
            )
            .with_stage(
                stage::PHYSICS,
                SystemStage::parallel().with_system(physics::step_world_system.system()),
            )
            .with_stage(
                stage::POST_PHYSICS,
                SystemStage::parallel()
                    .with_system(sync_position.system())
                    .with_system(physics::destroy_body_and_collider_system.system())
                    .with_system(physics::sync_transform_system.system()),
            )
            .with_stage(
                stage::POST_GAME,
                SystemStage::parallel().with_system(tick_simulation_frame.system()),
            );

        let main_schedule = Schedule::default()
            .with_run_criteria(
                main_run_criteria
                    .take()
                    .expect("Can't initialize the plugin more than once"),
            )
            .with_stage(stage::SIMULATION_SCHEDULE, simulation_schedule)
            .with_stage(
                stage::BROADCAST_UPDATES,
                broadcast_updates_stage
                    .take()
                    .expect("Can't initialize the plugin more than once")
                    .with_run_criteria(GameTickRunCriteria::new(TICKS_PER_NETWORK_BROADCAST)),
            )
            .with_stage(
                stage::POST_SIMULATIONS,
                SystemStage::serial()
                    .with_system(tick_game_frame.system())
                    .with_system(process_spawned_entities.system()),
            )
            .with_stage(
                stage::POST_TICK,
                post_tick_stage
                    .take()
                    .expect("Can't initialize the plugin more than once")
                    .with_run_criteria(GameTickRunCriteria::new(TICKS_PER_NETWORK_BROADCAST)),
            );

        builder.add_stage_before(
            bevy::app::stage::UPDATE,
            stage::MAIN_SCHEDULE,
            main_schedule,
        );
        builder.add_stage_before(
            stage::MAIN_SCHEDULE,
            stage::READ_INPUT_UPDATES,
            SystemStage::parallel().with_system(read_movement_updates.system()),
        );
        builder.add_stage_before(
            stage::READ_INPUT_UPDATES,
            stage::WRITE_INPUT_UPDATES,
            input_stage
                .take()
                .expect("Can't initialize the plugin more than once"),
        );

        builder.add_startup_system(network_setup.system());

        let resources = builder.resources_mut();
        resources.get_or_insert_with(GameTime::default);
        resources.get_or_insert_with(SimulationTime::default);
        resources.get_or_insert_with(LevelState::default);
        resources.get_or_insert_with(PlayerUpdates::default);
        resources.get_or_insert_with(GameCommands::<SpawnPlayer>::default);
        resources.get_or_insert_with(GameCommands::<DespawnPlayer>::default);
        resources.get_or_insert_with(GameCommands::<SpawnLevelObject>::default);
        resources.get_or_insert_with(GameCommands::<DespawnLevelObject>::default);
        resources.get_or_insert_with(EntityRegistry::<PlayerNetId>::default);
        resources.get_or_insert_with(EntityRegistry::<EntityNetId>::default);
        resources.get_or_insert_with(HashMap::<PlayerNetId, Player>::default);
    }
}

pub struct RapierResourcesPlugin;

impl Plugin for RapierResourcesPlugin {
    fn build(&self, builder: &mut AppBuilder) {
        builder
            .add_resource(PhysicsPipeline::new())
            .add_resource(QueryPipeline::new())
            .add_resource(RapierConfiguration {
                gravity: Vector::new(0.0, 0.0, 0.0),
                ..RapierConfiguration::default()
            })
            .add_resource(IntegrationParameters::default())
            .add_resource(BroadPhase::new())
            .add_resource(NarrowPhase::new())
            .add_resource(RigidBodySet::new())
            .add_resource(ColliderSet::new())
            .add_resource(JointSet::new())
            .add_resource(InteractionPairFilters::new())
            .add_resource(EventQueue::new(true))
            .add_resource(SimulationToRenderTime::default())
            .add_resource(EntityMaps::default());
    }
}

// TODO: split into two resources for simulation and game frames to live separately?
//  This will probably help with avoiding bugs where we mistakenly use game frame
//  instead of simulation frame.
#[derive(Default, Clone, PartialEq, Debug)]
pub struct GameTime {
    pub generation: usize,
    pub frame_number: FrameNumber,
}

#[derive(Default, Debug)]
pub struct SimulationTime {
    /// Is expected to be ahead of `server_frame` on the client side, is equal to `server_frame`
    /// on the server side.
    pub player_frame: FrameNumber,
    pub server_frame: FrameNumber,
}

impl SimulationTime {
    pub fn entity_simulation_frame(
        &self,
        player_frame_simulated: Option<&PlayerFrameSimulated>,
    ) -> FrameNumber {
        if player_frame_simulated.is_some() {
            self.player_frame
        } else {
            self.server_frame
        }
    }

    pub fn rewind(&mut self, frame_number: FrameNumber) {
        let frames_ahead = self.player_frame - self.server_frame;
        self.server_frame = std::cmp::min(self.server_frame, frame_number);
        self.player_frame = self.server_frame + frames_ahead;
    }
}

pub struct GameTickRunCriteria {
    system_id: SystemId,
    ticks_per_step: FrameNumber,
    last_generation: Option<usize>,
    last_tick: FrameNumber,
    resource_access: TypeAccess<TypeId>,
    archetype_access: TypeAccess<ArchetypeComponent>,
}

impl GameTickRunCriteria {
    pub fn new(ticks_per_step: u16) -> Self {
        Self {
            system_id: SystemId::new(),
            ticks_per_step: FrameNumber::new(ticks_per_step),
            last_generation: None,
            last_tick: FrameNumber::new(0),
            resource_access: Default::default(),
            archetype_access: Default::default(),
        }
    }

    pub fn update(&mut self, time: &GameTime) -> ShouldRun {
        if self.last_generation != Some(time.generation) {
            self.last_generation = Some(time.generation);
            self.last_tick = time.frame_number - self.ticks_per_step;
        }

        if self.last_tick + self.ticks_per_step <= time.frame_number {
            trace!("Run and loop a game schedule (game {})", time.frame_number);
            self.last_tick += self.ticks_per_step;
            ShouldRun::YesAndLoop
        } else {
            trace!("Don't run a game schedule (game {})", time.frame_number);
            ShouldRun::No
        }
    }
}

impl System for GameTickRunCriteria {
    type In = ();
    type Out = ShouldRun;

    fn name(&self) -> Cow<'static, str> {
        Cow::Borrowed(std::any::type_name::<GameTickRunCriteria>())
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
        let time = resources.get::<GameTime>().unwrap();
        let result = self.update(&time);

        Some(result)
    }

    fn run_thread_local(&mut self, _world: &mut World, _resources: &mut Resources) {}

    fn initialize(&mut self, _world: &mut World, _resources: &mut Resources) {
        self.resource_access.add_read(TypeId::of::<GameTime>());
    }
}

pub struct SimulationTickRunCriteria {
    system_id: SystemId,
    last_game_frame: Option<FrameNumber>,
    last_player_frame: FrameNumber,
    last_server_frame: FrameNumber,
    resource_access: TypeAccess<TypeId>,
    archetype_access: TypeAccess<ArchetypeComponent>,
}

impl Default for SimulationTickRunCriteria {
    fn default() -> Self {
        Self {
            system_id: SystemId::new(),
            last_game_frame: None,
            last_player_frame: FrameNumber::new(0),
            last_server_frame: FrameNumber::new(0),
            resource_access: Default::default(),
            archetype_access: Default::default(),
        }
    }
}

impl SimulationTickRunCriteria {
    pub fn update(&mut self, game_time: &GameTime, simulation_time: &SimulationTime) -> ShouldRun {
        // Checking that a game frame has changed will make us avoid panicking in case we rewind
        // simulation frame just 1 frame back.
        if self.last_game_frame != Some(game_time.frame_number) {
            self.last_game_frame = Some(game_time.frame_number);
        } else if self.last_player_frame == simulation_time.player_frame
            && self.last_server_frame == simulation_time.server_frame
        {
            panic!(
                "Simulation frame hasn't advanced: {}, {}",
                simulation_time.player_frame, simulation_time.server_frame
            );
        }
        self.last_player_frame = simulation_time.player_frame;
        self.last_server_frame = simulation_time.server_frame;

        if self.last_player_frame <= game_time.frame_number {
            trace!(
                "Run and loop a simulation schedule (simulation: {}, game {})",
                simulation_time.player_frame,
                game_time.frame_number
            );
            ShouldRun::YesAndLoop
        } else {
            trace!(
                "Don't run a simulation schedule (simulation: {}, game {})",
                simulation_time.player_frame,
                game_time.frame_number
            );
            ShouldRun::No
        }
    }
}

impl System for SimulationTickRunCriteria {
    type In = ();
    type Out = ShouldRun;

    fn name(&self) -> Cow<'static, str> {
        Cow::Borrowed(std::any::type_name::<SimulationTickRunCriteria>())
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
        let time = resources.get::<GameTime>().unwrap();
        let simulation_time = resources.get::<SimulationTime>().unwrap();
        let result = self.update(&time, &simulation_time);

        Some(result)
    }

    fn run_thread_local(&mut self, _world: &mut World, _resources: &mut Resources) {}

    fn initialize(&mut self, _world: &mut World, _resources: &mut Resources) {
        self.resource_access.add_read(TypeId::of::<GameTime>());
    }
}

pub fn tick_simulation_frame(mut time: ResMut<SimulationTime>) {
    log::trace!(
        "Concluding simulation frame tick: {}, {}",
        time.server_frame.value(),
        time.player_frame.value()
    );
    time.server_frame += FrameNumber::new(1);
    time.player_frame += FrameNumber::new(1);
}

pub fn tick_game_frame(mut time: ResMut<GameTime>) {
    log::trace!("Concluding game frame tick: {}", time.frame_number.value());
    time.frame_number += FrameNumber::new(1);
}

struct PairFilter;

impl ContactPairFilter for PairFilter {
    fn filter_contact_pair(&self, _context: &PairFilterContext) -> Option<SolverFlags> {
        Some(SolverFlags::COMPUTE_IMPULSES)
    }
}

impl IntersectionPairFilter for PairFilter {
    fn filter_intersection_pair(&self, _context: &PairFilterContext) -> bool {
        true
    }
}
