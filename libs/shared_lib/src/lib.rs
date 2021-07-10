#![feature(const_fn_trait_bound)]
#![feature(hash_drain_filter)]
#![feature(step_trait)]
#![feature(trait_alias)]

use crate::{
    framebuffer::FrameNumber,
    game::{
        commands::{
            DeferredQueue, DespawnLevelObject, DespawnPlayer, RestartGame, SpawnPlayer,
            SwitchPlayerRole, UpdateLevelObject,
        },
        components::PlayerFrameSimulated,
        level::LevelState,
        level_objects::{process_objects_route_graph, update_level_object_movement_route_settings},
        movement::{load_object_positions, player_movement, read_movement_updates, sync_position},
        remove_disconnected_players, restart_game,
        spawn::{
            despawn_level_objects, despawn_players, process_spawned_entities, spawn_players,
            update_level_objects,
        },
        switch_player_role,
    },
    messages::{DeferredMessagesQueue, SwitchRole},
    net::network_setup,
    player::{Player, PlayerUpdates},
    registry::EntityRegistry,
};
use bevy::{
    app::Events,
    ecs::{
        archetype::{Archetype, ArchetypeComponentId},
        component::ComponentId,
        query::Access,
        schedule::{ParallelSystemDescriptorCoercion, ShouldRun},
        system::{IntoSystem, SystemId},
    },
    log,
    prelude::*,
};
use bevy_networking_turbulence::{LinkConditionerConfig, NetworkingPlugin};
use bevy_rapier3d::{
    physics,
    physics::{
        JointsEntityMap, ModificationTracker, NoUserData, PhysicsHooksWithQueryObject,
        RapierConfiguration, SimulationToRenderTime, TimestepMode,
    },
    rapier::{
        dynamics::{CCDSolver, IntegrationParameters, IslandManager, JointSet},
        geometry::{BroadPhase, ContactEvent, IntersectionEvent, NarrowPhase},
        math::Vector,
        pipeline::{PhysicsPipeline, QueryPipeline},
    },
};
use messages::{EntityNetId, PlayerNetId};
use std::{borrow::Cow, collections::HashMap, sync::Mutex};

#[cfg(feature = "client")]
pub mod client;
pub mod framebuffer;
pub mod game;
pub mod messages;
pub mod net;
pub mod player;
pub mod registry;
pub mod util;
pub mod wrapped_counter;

// Constants.
pub mod stage {
    pub const WRITE_INPUT_UPDATES: &str = "mr_shared_write_input_updates";

    pub const MAIN_SCHEDULE: &str = "mr_shared_main_schedule";
    pub const STATE_DRIVER: &str = "mr_shared_state_driver";
    pub const READ_INPUT_UPDATES: &str = "mr_shared_read_input_updates";
    pub const BROADCAST_UPDATES: &str = "mr_shared_broadcast_updates";
    pub const POST_SIMULATIONS: &str = "mr_shared_post_simulations";
    pub const POST_TICK: &str = "mr_shared_post_tick";

    pub const SIMULATION_SCHEDULE: &str = "mr_shared_simulation_schedule";
    pub const SPAWN: &str = "mr_shared_spawn";
    pub const PRE_GAME: &str = "mr_shared_pre_game";
    pub const FINALIZE_PHYSICS: &str = "mr_shared_finalize_physics";
    pub const GAME: &str = "mr_shared_game";
    pub const PHYSICS: &str = "mr_shared_physics";
    pub const POST_PHYSICS: &str = "mr_shared_post_physics";
    pub const POST_GAME: &str = "mr_shared_post_game";
}
pub const GHOST_SIZE_MULTIPLIER: f32 = 1.001;
pub const PLAYER_SIZE: f32 = 0.5;
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
                SystemStage::parallel()
                    .with_system(switch_player_role.system().label("player_role"))
                    .with_system(
                        despawn_players
                            .system()
                            .label("despawn_players")
                            .after("player_role"),
                    )
                    .with_system(spawn_players.system().after("despawn_players"))
                    .with_system(despawn_level_objects.system().label("despawn_objects"))
                    // Updating level objects might despawn entities completely if they are
                    // updated with replacement. Running it before `despawn_level_objects` might
                    // result into an edge-case where changes to the `Spawned` component are not
                    // propagated.
                    .with_system(update_level_objects.system().after("despawn_objects")),
            )
            .with_stage(
                stage::PRE_GAME,
                SystemStage::parallel()
                    .with_system(update_level_object_movement_route_settings.system())
                    .with_system(physics::attach_bodies_and_colliders_system.system())
                    .with_system(physics::create_joints_system.system()),
            )
            .with_stage(
                stage::FINALIZE_PHYSICS,
                SystemStage::parallel()
                    .with_system(physics::finalize_collider_attach_to_bodies.system()),
            )
            .with_stage(
                stage::GAME,
                SystemStage::parallel()
                    .with_system(player_movement.system())
                    .with_system(process_objects_route_graph.system().label("route_graph"))
                    .with_system(load_object_positions.system().after("route_graph")),
            )
            .with_stage(
                stage::PHYSICS,
                SystemStage::parallel()
                    .with_system(physics::step_world_system::<NoUserData>.system()),
            )
            .with_stage(
                stage::POST_PHYSICS,
                SystemStage::parallel()
                    .with_system(physics::sync_transforms.system().label("sync_transforms"))
                    .with_system(sync_position.system().after("sync_transforms")),
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
                SystemStage::parallel()
                    .with_system(tick_game_frame.system().label("tick"))
                    .with_system(process_spawned_entities.system().after("tick"))
                    // Remove disconnected players doesn't depend on ticks, so it's fine.
                    .with_system(remove_disconnected_players.system()),
            )
            .with_stage(
                stage::POST_TICK,
                post_tick_stage
                    .take()
                    .expect("Can't initialize the plugin more than once")
                    .with_system(physics::collect_removals.system()),
            );

        builder.add_stage_before(
            bevy::app::CoreStage::Update,
            stage::MAIN_SCHEDULE,
            main_schedule,
        );
        builder.add_stage_before(
            stage::MAIN_SCHEDULE,
            stage::READ_INPUT_UPDATES,
            SystemStage::parallel()
                .with_system(restart_game.exclusive_system().label("restart_game"))
                .with_system_set(
                    SystemSet::on_update(GameState::Playing)
                        .after("restart_game")
                        .with_system(read_movement_updates.system()),
                ),
        );
        builder.add_stage_before(
            stage::READ_INPUT_UPDATES,
            stage::WRITE_INPUT_UPDATES,
            input_stage
                .take()
                .expect("Can't initialize the plugin more than once"),
        );

        // Is `GameState::Paused` for client (see `init_state`).
        builder.add_state(GameState::Playing);
        builder.add_state_to_stage(stage::READ_INPUT_UPDATES, GameState::Playing);

        builder.add_startup_system(network_setup.system());

        #[cfg(feature = "client")]
        builder.add_startup_system(crate::client::materials::init_object_materials.system());

        let resources = builder.world_mut();
        resources.get_resource_or_insert_with(GameTime::default);
        resources.get_resource_or_insert_with(SimulationTime::default);
        resources.get_resource_or_insert_with(LevelState::default);
        resources.get_resource_or_insert_with(PlayerUpdates::default);
        resources.get_resource_or_insert_with(DeferredQueue::<RestartGame>::default);
        resources.get_resource_or_insert_with(DeferredQueue::<SpawnPlayer>::default);
        resources.get_resource_or_insert_with(DeferredQueue::<DespawnPlayer>::default);
        resources.get_resource_or_insert_with(DeferredQueue::<UpdateLevelObject>::default);
        resources.get_resource_or_insert_with(DeferredQueue::<DespawnLevelObject>::default);
        resources.get_resource_or_insert_with(DeferredQueue::<SwitchPlayerRole>::default);
        resources.get_resource_or_insert_with(EntityRegistry::<PlayerNetId>::default);
        resources.get_resource_or_insert_with(EntityRegistry::<EntityNetId>::default);
        resources.get_resource_or_insert_with(HashMap::<PlayerNetId, Player>::default);
        // Is used only on the server side.
        resources.get_resource_or_insert_with(DeferredMessagesQueue::<SwitchRole>::default);
    }
}

pub struct RapierResourcesPlugin;

impl Plugin for RapierResourcesPlugin {
    fn build(&self, builder: &mut AppBuilder) {
        builder
            .insert_resource(PhysicsPipeline::new())
            .insert_resource(QueryPipeline::new())
            .insert_resource(RapierConfiguration {
                gravity: Vector::new(0.0, 0.0, 0.0),
                timestep_mode: TimestepMode::FixedTimestep,
                ..RapierConfiguration::default()
            })
            .insert_resource(IntegrationParameters::default())
            .insert_resource(BroadPhase::new())
            .insert_resource(NarrowPhase::new())
            .insert_resource(IslandManager::new())
            .insert_resource(JointSet::new())
            .insert_resource(CCDSolver::new())
            .insert_resource(PhysicsHooksWithQueryObject::<NoUserData>(Box::new(())))
            .insert_resource(Events::<IntersectionEvent>::default())
            .insert_resource(Events::<ContactEvent>::default())
            .insert_resource(SimulationToRenderTime::default())
            .insert_resource(JointsEntityMap::default())
            .insert_resource(ModificationTracker::default());
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub enum GameState {
    Paused,
    Playing,
}

#[derive(Default, Clone, PartialEq, Debug)]
pub struct GameTime {
    pub session: usize,
    pub frame_number: FrameNumber,
}

#[derive(Default, Debug)]
pub struct SimulationTime {
    /// Is expected to be ahead of `server_frame` on the client side, is equal to `server_frame`
    /// on the server side.
    pub player_frame: FrameNumber,
    pub player_generation: u64,
    pub server_frame: FrameNumber,
    pub server_generation: u64,
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
        if cfg!(not(feature = "client")) {
            assert_eq!(self.player_frame, self.server_frame);
        } else {
            assert!(self.player_frame >= self.server_frame);
        }
        let frames_ahead = self.player_frame - self.server_frame;
        if (self.server_frame.value() as i32 - frame_number.value() as i32).abs() as u16
            > u16::MAX / 2
            && self.server_frame > frame_number
        {
            self.server_generation -= 1;
        }
        self.server_frame = self.server_frame.min(frame_number);
        let (player_frame, overflown) = self.server_frame.add(frames_ahead);
        self.player_frame = player_frame;
        if overflown {
            self.player_generation = self.server_generation + 1;
        } else {
            self.player_generation = self.server_generation;
        }
    }

    pub fn player_frames_ahead(&self) -> u16 {
        assert!(self.player_frame >= self.server_frame);
        (self.player_frame - self.server_frame).value()
    }

    pub fn prev_frame(&self) -> SimulationTime {
        assert!(
            (self.player_frame.value() > 0 || self.player_generation > 0)
                && (self.server_frame.value() > 0 || self.server_generation > 0)
        );
        let player_generation = if self.player_frame == FrameNumber::new(0) {
            self.player_generation - 1
        } else {
            self.player_generation
        };
        let server_generation = if self.server_frame == FrameNumber::new(0) {
            self.server_generation - 1
        } else {
            self.server_generation
        };
        Self {
            player_frame: self.player_frame - FrameNumber::new(1),
            player_generation,
            server_frame: self.server_frame - FrameNumber::new(1),
            server_generation,
        }
    }
}

#[derive(Default, Clone)]
pub struct GameTickRunCriteriaState {
    ticks_per_step: FrameNumber,
    last_generation: Option<usize>,
    last_tick: FrameNumber,
}

pub struct GameTickRunCriteria {
    state: GameTickRunCriteriaState,
    internal_system: Box<dyn System<In = (), Out = ShouldRun>>,
}

impl GameTickRunCriteria {
    pub fn new(ticks_per_step: u16) -> Self {
        Self {
            state: GameTickRunCriteriaState {
                ticks_per_step: FrameNumber::new(ticks_per_step),
                last_generation: None,
                last_tick: FrameNumber::new(0),
            },
            internal_system: Box::new(Self::prepare_system.system()),
        }
    }

    fn prepare_system(
        mut state: Local<GameTickRunCriteriaState>,
        time: Res<GameTime>,
    ) -> ShouldRun {
        if state.last_generation != Some(time.session) {
            state.last_generation = Some(time.session);
            state.last_tick = time.frame_number - state.ticks_per_step;
        }

        if state.last_tick + state.ticks_per_step <= time.frame_number {
            trace!("Run and loop a game schedule (game {})", time.frame_number);
            let ticks_per_step = state.ticks_per_step;
            state.last_tick += ticks_per_step;
            ShouldRun::YesAndCheckAgain
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

#[derive(Default, Clone)]
pub struct SimulationTickRunCriteriaState {
    last_game_frame: Option<FrameNumber>,
    last_player_frame: FrameNumber,
    last_server_frame: FrameNumber,
}

pub struct SimulationTickRunCriteria {
    state: SimulationTickRunCriteriaState,
    internal_system: Box<dyn System<In = (), Out = ShouldRun>>,
}

impl Default for SimulationTickRunCriteria {
    fn default() -> Self {
        Self {
            state: SimulationTickRunCriteriaState {
                last_game_frame: None,
                last_player_frame: FrameNumber::new(0),
                last_server_frame: FrameNumber::new(0),
            },
            internal_system: Box::new(Self::prepare_system.system()),
        }
    }
}

impl SimulationTickRunCriteria {
    fn prepare_system(
        mut state: Local<SimulationTickRunCriteriaState>,
        game_state: Res<State<GameState>>,
        game_time: Res<GameTime>,
        simulation_time: Res<SimulationTime>,
    ) -> ShouldRun {
        // Checking that a game frame has changed will make us avoid panicking in case we rewind
        // simulation frame just 1 frame back.
        if state.last_game_frame != Some(game_time.frame_number) {
            state.last_game_frame = Some(game_time.frame_number);
        } else if state.last_player_frame == simulation_time.player_frame
            && state.last_server_frame == simulation_time.server_frame
            && game_state.current() == &GameState::Playing
        {
            panic!(
                "Simulation frame hasn't advanced: {}, {}",
                simulation_time.player_frame, simulation_time.server_frame
            );
        }
        state.last_player_frame = simulation_time.player_frame;
        state.last_server_frame = simulation_time.server_frame;

        if state.last_player_frame <= game_time.frame_number
            && game_state.current() == &GameState::Playing
        {
            trace!(
                "Run and loop a simulation schedule (simulation: {}, game: {}, state: {:?})",
                simulation_time.player_frame,
                game_time.frame_number,
                *game_state
            );
            ShouldRun::YesAndCheckAgain
        } else {
            trace!(
                "Don't run a simulation schedule (simulation: {}, game: {}, state: {:?})",
                simulation_time.player_frame,
                game_time.frame_number,
                *game_state
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

pub fn tick_simulation_frame(mut time: ResMut<SimulationTime>) {
    log::trace!(
        "Concluding simulation frame tick: {}, {}",
        time.server_frame.value(),
        time.player_frame.value()
    );
    if time.server_frame.value() == u16::MAX {
        time.server_generation += 1;
    }
    time.server_frame += FrameNumber::new(1);
    if time.player_frame.value() == u16::MAX {
        time.player_generation += 1;
    }
    time.player_frame += FrameNumber::new(1);
}

pub fn tick_game_frame(mut time: ResMut<GameTime>) {
    log::trace!("Concluding game frame tick: {}", time.frame_number.value());
    time.frame_number += FrameNumber::new(1);
}
