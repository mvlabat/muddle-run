#![feature(const_option_ext)]
#![feature(drain_filter)]
#![feature(const_fn_floating_point_arithmetic)]
#![feature(hash_drain_filter)]
#![feature(step_trait)]
#![feature(trait_alias)]
#![allow(clippy::return_self_not_must_use)]

use crate::{
    framebuffer::FrameNumber,
    game::{
        collisions::{process_collision_events_system, process_players_with_new_collisions_system},
        commands::{
            DeferredQueue, DespawnLevelObject, DespawnPlayer, SpawnPlayer, SwitchPlayerRole,
            UpdateLevelObject,
        },
        components::PlayerFrameSimulated,
        events::{CollisionLogicChanged, PlayerDeath, PlayerFinish},
        level::{maintain_available_spawn_areas_system, LevelState},
        level_objects::{
            process_objects_route_graph_system, update_level_object_movement_route_settings_system,
        },
        movement::{
            isolate_client_mispredicted_world_system, load_object_positions_system,
            player_movement_system, read_movement_updates_system, sync_position_system,
        },
        remove_disconnected_players_system, reset_game_world_system,
        spawn::{
            despawn_level_objects_system, despawn_players_system, poll_calculating_shapes_system,
            process_spawned_entities_system, spawn_players_system, update_level_objects_system,
            ColliderShapePromiseResult, ColliderShapeReceiver, ColliderShapeSender,
        },
        switch_player_role_system,
    },
    messages::{DeferredMessagesQueue, SwitchRole},
    net::network_setup_system,
    player::{PlayerUpdates, Players},
    registry::EntityRegistry,
};
use bevy::{
    ecs::{
        event::Events,
        schedule::{ScheduleLabel, SystemConfigs},
        system::IntoSystem,
    },
    log,
    prelude::*,
};
use bevy_disturbulence::{NetworkingPlugin, SocketConfig};
use bevy_rapier2d::{
    pipeline::CollisionEvent,
    plugin::{PhysicsSet, RapierConfiguration, RapierPhysicsPlugin, TimestepMode},
};
use messages::{EntityNetId, PlayerNetId};
use std::sync::Mutex;

#[cfg(feature = "client")]
pub mod client;
pub mod collider_flags;
pub mod framebuffer;
pub mod game;
pub mod messages;
pub mod net;
pub mod player;
pub mod registry;
#[cfg(not(feature = "client"))]
pub mod server;
pub mod util;
pub mod wrapped_counter;

// Constants.
pub mod stage {
    pub const WRITE_INPUT_UPDATES: &str = "mr_shared_write_input_updates";
    pub const APP_STATE_TRANSITION: &str = "mr_shared_app_state_transition";
    pub const GAME_SESSION_STATE_TRANSITION: &str = "mr_shared_game_session_state_transition";
    pub const READ_INPUT_UPDATES: &str = "mr_shared_read_input_updates";
    // Here, between `WRITE_INPUT_UPDATES` and `BROADCAST_UPDATES` runs the main
    // schedule, which also includes the `SIMULATION_SCHEDULE` (running the game
    // logic and physics).
    // ...
    pub const MAIN_SCHEDULE: &str = "mr_shared_main_schedule";
    pub const SIMULATION_SCHEDULE: &str = "mr_shared_simulation_schedule";
    // ...
    pub const BROADCAST_UPDATES: &str = "mr_shared_broadcast_updates";
    pub const POST_SIMULATIONS: &str = "mr_shared_post_simulations";
    pub const POST_TICK: &str = "mr_shared_post_tick";

    // Stages of the `SIMULATION_SCHEDULE`:
    pub const SPAWN: &str = "mr_shared_spawn";
    pub const PRE_GAME: &str = "mr_shared_pre_game";
    pub const FINALIZE_PHYSICS: &str = "mr_shared_finalize_physics";
    pub const GAME: &str = "mr_shared_game";
    pub const PHYSICS: &str = "mr_shared_physics";
    pub const POST_PHYSICS: &str = "mr_shared_post_physics";
    pub const POST_GAME: &str = "mr_shared_post_game";
    pub const SIMULATION_FINAL: &str = "mr_shared_simulation_final";
}

#[derive(Debug, Hash, PartialEq, Eq, Clone, SystemSet)]
enum AppSet {
    WriteInputUpdates,
    MainSchedule,
}

#[derive(Debug, Hash, PartialEq, Eq, Clone, SystemSet)]
#[system_set(base)]
enum MainSet {
    SimulationSchedule,
    BroadcastUpdates,
    PostSimulation,
    Last,
}

#[derive(Debug, Hash, PartialEq, Eq, Clone, SystemSet)]
#[system_set(base)]
enum SimulationSet {
    Spawn,
    PreGame,
    Game,
    GamePhysics,
    PostGamePhysics,
    PostGame,
    Last,
}

/// Copy of the bevy_rapier's `PhysicsSet`.
/// We need a copy to turn it not into a base set.
#[derive(Debug, Hash, PartialEq, Eq, Clone, SystemSet)]
enum PhysicsSimulationSet {
    SyncBackend,
    SyncBackendFlush,
    StepSimulation,
    Writeback,
}

#[derive(Debug, Hash, PartialEq, Eq, Clone, ScheduleLabel)]
pub struct MainSchedule;

#[derive(Debug, Hash, PartialEq, Eq, Clone, ScheduleLabel)]
pub struct SimulationSchedule;

pub const GHOST_SIZE_MULTIPLIER: f32 = 1.001;
pub const PLAYER_RADIUS: f32 = 0.35;
pub const PLAYER_SENSOR_RADIUS: f32 = 0.05;
pub const PLANE_SIZE: f32 = 20.0;
pub const COMPONENT_FRAMEBUFFER_LIMIT: u16 = 120 * 10;
// 10 seconds of 120fps
pub const TICKS_PER_NETWORK_BROADCAST: u16 = 2;
pub const MAX_LAG_COMPENSATION_MILLIS: u16 = 200;
pub const SIMULATIONS_PER_SECOND: f32 = {
    const fn parse(v: &'static str) -> Option<u16> {
        let parser = konst::Parser::from_str(v);
        Some(konst::unwrap_ctx!(parser.parse_u16()).0)
    }

    (match match std::option_env!("SIMULATIONS_PER_SECOND") {
        Some(v) => parse(v),
        None => None,
    } {
        Some(v) => v,
        None => SIMULATIONS_PER_SECOND_DEFAULT,
    }) as f32
};
pub const LAG_COMPENSATED_FRAMES: FrameNumber = {
    let v = (MAX_LAG_COMPENSATION_MILLIS as f32 / (1000.0 / SIMULATIONS_PER_SECOND)) as u16;
    FrameNumber::new(v)
};

const SIMULATIONS_PER_SECOND_DEFAULT: u16 = 120;

pub struct MuddleSharedPlugin<S: System<In = (), Out = bool>> {
    system_configs: Mutex<Option<MuddleSystemConfigs<S>>>,
    socket_config: Option<SocketConfig>,
}

pub struct MuddleSystemConfigs<S: System<In = (), Out = bool>> {
    pub main_run_criteria: S,
    pub input_set: SystemConfigs,
    pub post_game_set: SystemConfigs,
    pub broadcast_updates_set: SystemConfigs,
    pub post_tick_set: SystemConfigs,
}

impl<S: System<In = (), Out = bool>> MuddleSharedPlugin<S> {
    pub fn new(
        system_configs: MuddleSystemConfigs<S>,
        socket_config: Option<SocketConfig>,
    ) -> Self {
        Self {
            system_configs: Mutex::new(Some(system_configs)),
            socket_config,
        }
    }
}

fn create_looping_schedule_with_criteria_system<Marker>(
    init_world: &mut World,
    schedule: impl ScheduleLabel + Clone,
    run_criteria_system: impl IntoSystem<(), bool, Marker>,
) -> impl FnMut(&mut World) {
    let mut run_criteria_system = IntoSystem::into_system(run_criteria_system);
    run_criteria_system.initialize(init_world);
    move |world: &mut World| {
        while run_criteria_system.run((), world) {
            world.run_schedule(schedule.clone());
        }
    }
}

impl<S: System<In = (), Out = bool>> Plugin for MuddleSharedPlugin<S> {
    fn build(&self, app: &mut App) {
        app.add_plugin(RapierPhysicsPlugin::<()>::default().with_default_system_setup(false));
        app.insert_resource(RapierConfiguration {
            gravity: Vec2::ZERO,
            timestep_mode: TimestepMode::Fixed {
                dt: 1.0 / SIMULATIONS_PER_SECOND,
                substeps: 1,
            },
            ..RapierConfiguration::default()
        });
        // Note: `AppState` initialisation is overwritten by `mr_server`.
        app.add_state::<AppState>();
        app.add_state::<GameSessionState>();
        app.add_plugin(NetworkingPlugin {
            socket_config: self.socket_config.clone().unwrap_or_default(),
            ..NetworkingPlugin::default()
        });

        let MuddleSystemConfigs {
            mut main_run_criteria,
            input_set,
            post_game_set,
            broadcast_updates_set,
            post_tick_set,
        } = self
            .system_configs
            .lock()
            .expect("Can't initialize the plugin more than once")
            .take()
            .expect("Can't initialize the plugin more than once");

        let mut simulation_schedule = Schedule::new();
        simulation_schedule.configure_sets(
            (
                SimulationSet::Spawn,
                SimulationSet::PreGame,
                SimulationSet::Game,
                SimulationSet::GamePhysics,
                SimulationSet::PostGamePhysics,
                SimulationSet::PostGame,
                SimulationSet::Last,
            )
                .chain(),
        );

        // `SimulationSet::Spawn` base set systems.
        let spawn_set = (
            Events::<CollisionEvent>::update_system,
            Events::<PlayerFinish>::update_system,
            Events::<PlayerDeath>::update_system,
            switch_player_role_system,
            despawn_players_system.after(switch_player_role_system),
            despawn_level_objects_system,
            // Adding components to an entity if there's a command to remove it the queue
            // will lead to crash. Executing this system before `update_level_objects` helps
            // to avoid this scenario.
            poll_calculating_shapes_system.before(update_level_objects_system),
            // Updating level objects might despawn entities completely if they are
            // updated with replacement. Running it before `despawn_level_objects` might
            // result into an edge-case where changes to the `Spawned` component are not
            // propagated.
            update_level_objects_system.after(despawn_level_objects_system),
            maintain_available_spawn_areas_system.after(update_level_objects_system),
            spawn_players_system
                .after(despawn_players_system)
                .after(maintain_available_spawn_areas_system),
        );
        simulation_schedule.add_systems(spawn_set.in_base_set(SimulationSet::Spawn));

        // `SimulationSet::PreGame` base set systems.
        simulation_schedule.add_systems(
            (
                apply_system_buffers,
                update_level_object_movement_route_settings_system,
                apply_system_buffers,
            )
                .chain()
                .in_base_set(SimulationSet::PreGame),
        );

        // `SimulationSet::Game` base set systems.
        let game_set = (
            isolate_client_mispredicted_world_system,
            player_movement_system,
            process_objects_route_graph_system,
            load_object_positions_system.after(process_objects_route_graph_system),
        );
        simulation_schedule.add_systems(game_set.in_base_set(SimulationSet::Game));

        // `SimulationSet::GamePhysics` base set systems.
        simulation_schedule.configure_sets(
            (
                PhysicsSimulationSet::SyncBackend,
                PhysicsSimulationSet::SyncBackendFlush,
                PhysicsSimulationSet::StepSimulation,
                PhysicsSimulationSet::Writeback,
            )
                .chain()
                .in_base_set(SimulationSet::GamePhysics),
        );
        simulation_schedule.add_systems(
            RapierPhysicsPlugin::<()>::get_systems(PhysicsSet::SyncBackend)
                .in_set(PhysicsSimulationSet::SyncBackend),
        );
        simulation_schedule.add_systems(
            RapierPhysicsPlugin::<()>::get_systems(PhysicsSet::SyncBackendFlush)
                .in_set(PhysicsSimulationSet::SyncBackendFlush),
        );
        simulation_schedule.add_systems(
            RapierPhysicsPlugin::<()>::get_systems(PhysicsSet::StepSimulation)
                .in_set(PhysicsSimulationSet::StepSimulation),
        );
        simulation_schedule.add_systems(
            RapierPhysicsPlugin::<()>::get_systems(PhysicsSet::Writeback)
                .in_set(PhysicsSimulationSet::Writeback),
        );

        // `SimulationSet::PostGamePhysics` base set systems.
        simulation_schedule.add_systems((
            process_collision_events_system.pipe(process_players_with_new_collisions_system),
            sync_position_system,
        ));

        // `SimulationSet::PostGame` base set systems.
        simulation_schedule.add_systems(post_game_set.in_base_set(SimulationSet::PostGame));

        // `SimulationSet::Last` base set systems.
        simulation_schedule.add_systems(
            (tick_simulation_frame_system, apply_system_buffers).in_base_set(SimulationSet::Last),
        );

        // let simulation_schedule = Schedule::default()
        //     .with_run_criteria(IntoSystem::into_system(simulation_tick_run_criteria))
        //     .with_stage(
        //         stage::SPAWN,
        //         SystemStage::single_threaded()
        //             .with_system(Events::<CollisionEvent>::update_system)
        //             .with_system(Events::<PlayerFinish>::update_system)
        //             .with_system(Events::<PlayerDeath>::update_system)
        //             .with_system(switch_player_role_system)
        //
        // .with_system(despawn_players_system.after(switch_player_role_system))
        //             .with_system(despawn_level_objects_system)
        //             // Updating level objects might despawn entities completely if
        // they are             // updated with replacement. Running it before
        // `despawn_level_objects` might             // result into an edge-case
        // where changes to the `Spawned` component are not             //
        // propagated.
        // .with_system(update_level_objects_system.after(despawn_level_objects_system))
        //             // Adding components to an entity if there's a command to remove
        // it the queue             // will lead to crash. Executing this system
        // before `update_level_objects` helps             // to avoid this
        // scenario.
        // .with_system(poll_calculating_shapes_system.
        // before(update_level_objects_system))             .with_system(
        //
        // maintain_available_spawn_areas_system.after(update_level_objects_system),
        //             )
        //             .with_system(
        //                 spawn_players_system
        //                     .after(despawn_players_system)
        //                     .after(maintain_available_spawn_areas_system),
        //             ),
        //     )
        //     .with_stage(
        //         stage::PRE_GAME,
        //         SystemStage::single_threaded()
        //             .with_system(update_level_object_movement_route_settings_system),
        //     )
        //     .with_stage(
        //         stage::GAME,
        //         SystemStage::single_threaded()
        //             .with_system(isolate_client_mispredicted_world_system)
        //             .with_system(player_movement_system)
        //             .with_system(process_objects_route_graph_system)
        //             .with_system(
        //
        // load_object_positions_system.after(process_objects_route_graph_system),
        //             ),
        //     )
        //     .with_stage(
        //         stage::PHYSICS,
        //         SystemStage::single_threaded()
        //             .with_system_set(RapierPhysicsPlugin::<()>::get_systems(
        //                 PhysicsSet::SyncBackend,
        //             ))
        //             .with_system_set(
        //
        // RapierPhysicsPlugin::<()>::get_systems(PhysicsStages::StepSimulation)
        //                     .label(PhysicsSystemSetLabel::StepSimulation)
        //                     .after(PhysicsSystemSetLabel::SyncBackend),
        //             )
        //             .with_system_set(
        //
        // RapierPhysicsPlugin::<()>::get_systems(PhysicsStages::Writeback)
        //                     .label(PhysicsSystemSetLabel::Writeback)
        //                     .after(PhysicsSystemSetLabel::StepSimulation),
        //             ),
        //     )
        //     .with_stage(
        //         stage::POST_PHYSICS,
        //         SystemStage::single_threaded()
        //             .with_system(
        //                 process_collision_events_system
        //                     .pipe(process_players_with_new_collisions_system),
        //             )
        //             .with_system(sync_position_system),
        //     )
        //     .with_stage(stage::POST_GAME, post_game_stage)
        //     .with_stage(
        //         stage::SIMULATION_FINAL,
        //         SystemStage::single_threaded().
        // with_system(tick_simulation_frame_system),     );

        let mut main_schedule = Schedule::new();
        // app.init_schedule(MainSchedule);
        // let mut main_schedule = app.get_schedule_mut(MainSchedule).unwrap();

        main_schedule.configure_sets(
            (
                MainSet::SimulationSchedule,
                MainSet::BroadcastUpdates,
                MainSet::PostSimulation,
                MainSet::Last,
            )
                .chain(),
        );

        // `MainSet::SimulationSchedule` base set systems.
        main_schedule.add_system(
            create_looping_schedule_with_criteria_system(
                &mut app.world,
                SimulationSchedule,
                simulation_tick_run_criteria,
            )
            .in_base_set(MainSet::SimulationSchedule),
        );

        // `MainSet::BroadcastUpdates` base set systems.
        main_schedule.add_systems(
            broadcast_updates_set
                .distributive_run_if(game_tick_run_criteria(TICKS_PER_NETWORK_BROADCAST))
                .in_base_set(MainSet::BroadcastUpdates),
        );

        // `MainSet::PostSimulation` base set systems.
        let post_simulation_set = (
            // These systems are also present in the simulation set.
            // If the game is loading, the simulation set isn't run, but we
            // still need these systems as spawning level objects is
            // part of loading.
            poll_calculating_shapes_system.run_if(in_state(GameSessionState::Loading)),
            update_level_objects_system.run_if(in_state(GameSessionState::Loading)),
            tick_game_frame_system,
            process_spawned_entities_system.after(tick_game_frame_system),
            // Removing disconnected players doesn't depend on ticks, so it's
            // fine to have it unordered.
            remove_disconnected_players_system,
        )
            .chain();
        main_schedule.add_systems(post_simulation_set.in_base_set(MainSet::PostSimulation));

        // `MainSet::Last` base set systems.
        main_schedule.add_systems(post_tick_set.in_base_set(MainSet::Last));

        app.add_schedule(MainSchedule, main_schedule);
        app.add_schedule(SimulationSchedule, simulation_schedule);

        app.add_startup_system(network_setup_system);

        app.configure_sets(
            (AppSet::WriteInputUpdates, AppSet::MainSchedule).in_base_set(CoreSet::Update),
        );
        app.add_systems(input_set.in_set(AppSet::WriteInputUpdates));
        main_run_criteria.initialize(&mut app.world);
        let run_main_schedule_system = create_looping_schedule_with_criteria_system(
            &mut app.world,
            MainSchedule,
            main_run_criteria,
        );
        app.add_systems(
            (
                reset_game_world_system.run_if(
                    state_changed::<GameSessionState>()
                        .and_then(in_state(GameSessionState::Loading)),
                ),
                apply_system_buffers,
                read_movement_updates_system.run_if(in_state(GameSessionState::Playing)),
                run_main_schedule_system,
            )
                .chain()
                .in_set(AppSet::MainSchedule),
        );

        #[cfg(feature = "client")]
        app.add_startup_system(client::assets::init_muddle_assets_system);

        let world = &mut app.world;
        world.get_resource_or_insert_with(GameTime::default);
        world.get_resource_or_insert_with(SimulationTime::default);
        world.get_resource_or_insert_with(LevelState::default);
        world.get_resource_or_insert_with(PlayerUpdates::default);
        world.get_resource_or_insert_with(DeferredQueue::<SpawnPlayer>::default);
        world.get_resource_or_insert_with(DeferredQueue::<DespawnPlayer>::default);
        world.get_resource_or_insert_with(DeferredQueue::<UpdateLevelObject>::default);
        world.get_resource_or_insert_with(DeferredQueue::<DespawnLevelObject>::default);
        world.get_resource_or_insert_with(DeferredQueue::<SwitchPlayerRole>::default);
        world.get_resource_or_insert_with(EntityRegistry::<PlayerNetId>::default);
        world.get_resource_or_insert_with(EntityRegistry::<EntityNetId>::default);
        world.get_resource_or_insert_with(Players::default);
        world.get_resource_or_insert_with(Events::<CollisionLogicChanged>::default);
        world.get_resource_or_insert_with(Events::<PlayerDeath>::default);
        world.get_resource_or_insert_with(Events::<PlayerFinish>::default);
        // Is used only on the server side.
        world.get_resource_or_insert_with(DeferredMessagesQueue::<SwitchRole>::default);

        let (shape_sender, shape_receiver) =
            crossbeam_channel::unbounded::<ColliderShapePromiseResult>();
        world.insert_resource(ColliderShapeSender(shape_sender));
        world.insert_resource(ColliderShapeReceiver(shape_receiver));
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Hash, Default, States)]
pub enum AppState {
    /// Currently, this state is used only for clients.
    /// We use this state to spawn dummy PBR entities to trigger shaders
    /// loading. This is useful for browsers in the first place, as loading
    /// shaders is blocking there and freezes the app (so a loading screen
    /// should be shown).
    #[default]
    Loading,
    /// This state is used when a client is launched in the mode when going
    /// through the authentication and matchmaking menus is required before
    /// connecting to a server.
    MainMenu,
    /// A level is being loaded or played.
    Playing,
}

#[derive(Clone, Debug, Eq, PartialEq, Hash, States)]
pub enum GameSessionState {
    Loading,
    Playing,
    Paused,
}

impl Default for GameSessionState {
    fn default() -> Self {
        Self::Loading
    }
}

/// The resource is added when a client/server starts spawning level objects on
/// loading the level. When every level object is spawned, the counter reaches
/// zero. After that, we remove the resource and switch `GameSessionState` to
/// `Playing`.
#[derive(Resource, Deref, DerefMut)]
pub struct LevelObjectsToSpawnToLoad(pub usize);

#[derive(Resource, Default, Clone, Debug, PartialEq, Eq)]
pub struct GameTime {
    pub session: usize,
    pub frame_number: FrameNumber,
}

#[derive(Resource, Debug)]
pub struct SimulationTime {
    /// Is expected to be ahead of `server_frame` on the client side, is equal
    /// to `server_frame` on the server side.
    pub player_frame: FrameNumber,
    pub player_generation: u64,
    pub server_frame: FrameNumber,
    pub server_generation: u64,
    player_frames_to_rerun: Option<FrameNumber>,
}

impl Default for SimulationTime {
    fn default() -> Self {
        Self {
            player_frame: Default::default(),
            player_generation: 1,
            server_frame: Default::default(),
            server_generation: 1,
            player_frames_to_rerun: Default::default(),
        }
    }
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
        let prev_server = self.server_frame;
        let prev_player = self.player_frame;

        if cfg!(feature = "client") {
            assert!(self.player_frame >= self.server_frame);
            let frames_ahead = self.player_frame - self.server_frame;
            if frames_ahead.value() > 0 && self.player_frame >= frame_number {
                // If local server time is behind the delta update frame, we don't want make the
                // client re-run more more frames that it has to rewind (resulting in being
                // ahead of the server more than initially).
                let delta_update_ahead = if frame_number > self.server_frame {
                    // Take `min` just for safety, to avoid overflowing.
                    frames_ahead.min(frame_number - self.server_frame)
                } else {
                    FrameNumber::new(0)
                };
                let frames_to_rerun = frames_ahead - delta_update_ahead;
                if frames_to_rerun.value() > 0 {
                    self.player_frames_to_rerun
                        .get_or_insert(frames_ahead - delta_update_ahead);
                }
            }
        } else {
            assert_eq!(self.player_frame, self.server_frame);
        }

        if self.server_frame > frame_number {
            if (self.server_frame.value() as i32 - frame_number.value() as i32).unsigned_abs()
                as u16
                > u16::MAX / 2
            {
                // This shouldn't overflow as we start counting from 1, and we never decrement
                // more than once without incrementing.
                self.server_generation -= 1;
            }
            self.server_frame = frame_number;
            self.player_frame = frame_number;
            self.player_generation = self.server_generation;
        } else if self.player_frame > frame_number {
            if (self.player_frame.value() as i32 - frame_number.value() as i32).unsigned_abs()
                as u16
                > u16::MAX / 2
            {
                // This shouldn't overflow as we start counting from 1, and we never decrement
                // more than once without incrementing.
                self.player_generation -= 1;
            }
            self.player_frame = frame_number;
        }

        log::trace!(
            "Rewind to {{server: {} (prev: {}), player: {} (prev: {}), frame: {}}}",
            self.server_frame,
            prev_server,
            self.player_frame,
            prev_player,
            frame_number,
        );
    }

    pub fn player_frames_ahead(&self) -> u16 {
        assert!(self.player_frame >= self.server_frame);
        (self.player_frame - self.server_frame).value()
            + self
                .player_frames_to_rerun
                .map_or(0, |frames| frames.value())
    }

    pub fn prev_frame(&self) -> SimulationTime {
        // Just make sure that we won't overflow.
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
            player_frames_to_rerun: self.player_frames_to_rerun,
        }
    }

    pub fn player_frame_simulated_only(&self) -> bool {
        self.player_frames_to_rerun.is_some()
    }
}

#[derive(Default, Clone)]
pub struct GameTickRunCriteriaState {
    last_generation: Option<usize>,
    last_tick: FrameNumber,
}

fn game_tick_run_criteria(
    ticks_per_step: u16,
) -> impl FnMut(Local<GameTickRunCriteriaState>, Res<GameTime>) -> bool + Clone {
    move |mut state: Local<GameTickRunCriteriaState>, time: Res<GameTime>| -> bool {
        let ticks_per_step = FrameNumber::new(ticks_per_step);
        #[cfg(feature = "profiler")]
        puffin::profile_function!();
        if state.last_generation != Some(time.session) {
            state.last_generation = Some(time.session);
            state.last_tick = time.frame_number - ticks_per_step;
        }

        if state.last_tick + ticks_per_step <= time.frame_number {
            trace!("Run and loop a game schedule (game {})", time.frame_number);
            let ticks_per_step = ticks_per_step;
            state.last_tick += ticks_per_step;
            true
        } else {
            trace!("Don't run a game schedule (game {})", time.frame_number);
            false
        }
    }
}

#[derive(Default, Clone)]
pub struct SimulationTickRunCriteriaState {
    last_game_frame: Option<FrameNumber>,
    last_player_frame: FrameNumber,
    last_server_frame: FrameNumber,
}

fn simulation_tick_run_criteria(
    mut state: Local<SimulationTickRunCriteriaState>,
    game_state: Res<State<GameSessionState>>,
    game_time: Res<GameTime>,
    simulation_time: Res<SimulationTime>,
) -> bool {
    #[cfg(feature = "profiler")]
    puffin::profile_function!();
    // Checking that a game frame has changed helps us to avoid the panicking in
    // case we rewind simulation frame just 1 frame back.
    if state.last_game_frame != Some(game_time.frame_number) {
        state.last_game_frame = Some(game_time.frame_number);
    } else if state.last_player_frame == simulation_time.player_frame
        && state.last_server_frame == simulation_time.server_frame
        && game_state.0 == GameSessionState::Playing
    {
        panic!(
            "Simulation frame hasn't advanced: {}, {}",
            simulation_time.player_frame, simulation_time.server_frame
        );
    }
    state.last_player_frame = simulation_time.player_frame;
    state.last_server_frame = simulation_time.server_frame;

    if state.last_player_frame <= game_time.frame_number
        && game_state.0 == GameSessionState::Playing
    {
        trace!(
            "Run and loop a simulation schedule (simulation: {}, game: {}, state: {:?})",
            simulation_time.player_frame,
            game_time.frame_number,
            *game_state
        );
        true
    } else {
        trace!(
            "Don't run a simulation schedule (simulation: {}, game: {}, state: {:?})",
            simulation_time.player_frame,
            game_time.frame_number,
            *game_state
        );
        false
    }
}

pub fn tick_simulation_frame_system(mut time: ResMut<SimulationTime>) {
    // Tick server frame (only if we aren't still correcting client mispredictions).
    if time.player_frames_to_rerun.is_none() {
        if time.server_frame.value() == u16::MAX {
            time.server_generation += 1;
        }
        time.server_frame += FrameNumber::new(1);
    }

    // Tick player frame.
    if time.player_frame.value() == u16::MAX {
        time.player_generation += 1;
    }
    time.player_frame += FrameNumber::new(1);

    log::trace!(
        "New frame values: {}, {} (ahead: {}, to rerun: {:?})",
        time.server_frame.value(),
        time.player_frame.value(),
        time.player_frames_ahead(),
        time.player_frames_to_rerun,
    );

    // Check whether we finished replaying client inputs to correct mispredictions
    // (check that we've caught up with the previous `player_frames_to_rerun`
    // value).
    if let Some(player_frames_to_rerun) = &mut time.player_frames_to_rerun {
        *player_frames_to_rerun -= FrameNumber::new(1);
        if player_frames_to_rerun.value() == 0 {
            time.player_frames_to_rerun = None;
        }
    }
}

pub fn tick_game_frame_system(mut time: ResMut<GameTime>) {
    #[cfg(feature = "profiler")]
    puffin::profile_function!();
    log::trace!("Concluding game frame tick: {}", time.frame_number.value());
    time.frame_number += FrameNumber::new(1);
}
