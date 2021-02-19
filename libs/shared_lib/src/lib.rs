#![feature(step_trait)]
#![feature(step_trait_ext)]
#![feature(trait_alias)]

use crate::{
    framebuffer::FrameNumber,
    game::{
        commands::{
            DespawnLevelObject, DespawnPlayer, GameCommands, SpawnLevelObject, SpawnPlayer,
        },
        level::LevelState,
        movement::{player_movement, read_movement_updates, sync_position},
        spawn::{spawn_level_objects, spawn_players},
    },
    net::network_setup,
    player::{Player, PlayerUpdates},
    registry::EntityRegistry,
};
use bevy::{
    core::FixedTimestep,
    ecs::{ArchetypeComponent, ShouldRun, SystemId, ThreadLocalExecution, TypeAccess},
    log,
    prelude::*,
};
use bevy_networking_turbulence::NetworkingPlugin;
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

// Constants.
pub mod stage {
    pub const INPUT: &str = "mr_shared_input";
    pub const SCHEDULE: &str = "mr_shared_schedule";
    pub const SPAWN: &str = "mr_shared_spawn";
    pub const PRE_GAME: &str = "mr_shared_pre_game";
    pub const GAME: &str = "mr_shared_game";
    pub const PHYSICS: &str = "mr_shared_physics";
    pub const POST_PHYSICS: &str = "mr_shared_post_physics";
    pub const BROADCAST_UPDATES: &str = "mr_shared_broadcast_updates";
    pub const POST_GAME: &str = "mr_shared_post_game";
}
pub const PLAYER_SIZE: f32 = 1.0;
pub const PLANE_SIZE: f32 = 10.0;
pub const COMPONENT_FRAMEBUFFER_LIMIT: u16 = 120 * 10; // 10 seconds of 120fps
pub const TICKS_PER_NETWORK_BROADCAST: u16 = 2;

pub struct MuddleSharedPlugin {
    input_stage: Mutex<Option<SystemStage>>,
    broadcast_updates_stage: Mutex<Option<SystemStage>>,
}

impl MuddleSharedPlugin {
    pub fn new(input_stage: SystemStage, broadcast_updates_stage: SystemStage) -> Self {
        Self {
            input_stage: Mutex::new(Some(input_stage)),
            broadcast_updates_stage: Mutex::new(Some(broadcast_updates_stage)),
        }
    }
}

impl Plugin for MuddleSharedPlugin {
    fn build(&self, builder: &mut AppBuilder) {
        builder.add_plugin(RapierResourcesPlugin);
        builder.add_plugin(NetworkingPlugin {
            link_conditioner: None,
        });

        let mut input_stage = self
            .input_stage
            .lock()
            .expect("Can't initialize the plugin more than once");
        let mut broadcast_updates_stage = self
            .broadcast_updates_stage
            .lock()
            .expect("Can't initialize the plugin more than once");

        let schedule = Schedule::default()
            .with_run_criteria(FixedTimestep::steps_per_second(120.0))
            .with_stage(
                stage::SPAWN,
                SystemStage::parallel()
                    .with_system(spawn_players.system())
                    .with_system(spawn_level_objects.system()),
            )
            .with_stage(
                stage::PRE_GAME,
                SystemStage::parallel()
                    .with_system(read_movement_updates.system())
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
                stage::BROADCAST_UPDATES,
                broadcast_updates_stage
                    .take()
                    .expect("Can't initialize the plugin more than once")
                    .with_run_criteria(TickRunCriteria::new(TICKS_PER_NETWORK_BROADCAST)),
            )
            .with_stage(
                stage::POST_GAME,
                SystemStage::parallel().with_system(tick.system()),
            );
        builder.add_stage_before(bevy::app::stage::UPDATE, stage::SCHEDULE, schedule);
        builder.add_stage_before(
            stage::SCHEDULE,
            stage::INPUT,
            input_stage
                .take()
                .expect("Can't initialize the plugin more than once"),
        );

        builder.add_startup_system(network_setup.system());

        let resources = builder.resources_mut();
        resources.get_or_insert_with(GameTime::default);
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

#[derive(Default, Debug)]
pub struct GameTime {
    /// Simulation frame.
    pub game_frame: FrameNumber,
}

pub struct TickRunCriteria {
    system_id: SystemId,
    ticks_per_step: FrameNumber,
    last_tick: Option<FrameNumber>,
    resource_access: TypeAccess<TypeId>,
    archetype_access: TypeAccess<ArchetypeComponent>,
}

impl TickRunCriteria {
    pub fn new(ticks_per_step: u16) -> Self {
        Self {
            system_id: SystemId::new(),
            ticks_per_step: FrameNumber::new(ticks_per_step),
            last_tick: None,
            resource_access: Default::default(),
            archetype_access: Default::default(),
        }
    }

    pub fn update(&mut self, time: &GameTime) -> ShouldRun {
        if self.last_tick.is_none() {
            self.last_tick = Some(time.game_frame);
        }

        if self.last_tick.unwrap() + self.ticks_per_step <= time.game_frame {
            *self.last_tick.as_mut().unwrap() += self.ticks_per_step;
            ShouldRun::YesAndLoop
        } else {
            ShouldRun::No
        }
    }
}

impl System for TickRunCriteria {
    type In = ();
    type Out = ShouldRun;

    fn name(&self) -> Cow<'static, str> {
        Cow::Borrowed(std::any::type_name::<TickRunCriteria>())
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

pub fn tick(mut time: ResMut<GameTime>) {
    log::trace!("Tick: {}", time.game_frame.value());
    time.game_frame += FrameNumber::new(1);
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
