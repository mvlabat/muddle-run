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
        collisions::{process_collision_events, process_players_with_new_collisions},
        commands::{
            DeferredQueue, DespawnLevelObject, DespawnPlayer, RestartGame, SpawnPlayer,
            SwitchPlayerRole, UpdateLevelObject,
        },
        components::PlayerFrameSimulated,
        events::{CollisionLogicChanged, PlayerDeath, PlayerFinish},
        level::{maintain_available_spawn_areas, LevelState},
        level_objects::{process_objects_route_graph, update_level_object_movement_route_settings},
        movement::{
            isolate_client_mispredicted_world, load_object_positions, player_movement,
            read_movement_updates, sync_position,
        },
        remove_disconnected_players, restart_game,
        spawn::{
            despawn_level_objects, despawn_players, poll_calculating_shapes,
            process_spawned_entities, spawn_players, update_level_objects,
            ColliderShapePromiseResult,
        },
        switch_player_role,
    },
    messages::{DeferredMessagesQueue, SwitchRole},
    net::network_setup,
    player::{Player, PlayerUpdates},
    registry::EntityRegistry,
};
use bevy::{
    ecs::{
        event::Events,
        schedule::{ParallelSystemDescriptorCoercion, ShouldRun},
        system::IntoSystem,
    },
    log,
    prelude::*,
    utils::HashMap,
};
use bevy_networking_turbulence::{LinkConditionerConfig, NetworkingPlugin};
use bevy_rapier2d::{
    pipeline::{CollisionEvent, PhysicsHooksWithQueryResource},
    plugin::{
        PhysicsStages, RapierConfiguration, RapierContext, RapierPhysicsPlugin,
        SimulationToRenderTime, TimestepMode,
    },
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
    pub const SIMULATION_FINAL: &str = "mr_shared_simulation_final";
}
pub const GHOST_SIZE_MULTIPLIER: f32 = 1.001;
pub const PLAYER_RADIUS: f32 = 0.35;
pub const PLAYER_SENSOR_RADIUS: f32 = 0.05;
pub const PLANE_SIZE: f32 = 20.0;
pub const COMPONENT_FRAMEBUFFER_LIMIT: u16 = 120 * 10; // 10 seconds of 120fps
pub const TICKS_PER_NETWORK_BROADCAST: u16 = 2;
pub const MAX_LAG_COMPENSATION_MILLIS: u16 = 200;
pub const SIMULATIONS_PER_SECOND: f32 = {
    const fn parse(v: &'static str) -> Option<u16> {
        let parser = konst::Parser::from_str(v);
        Some(konst::unwrap_ctx!(parser.parse_u16()).0)
    }

    std::option_env!("SIMULATIONS_PER_SECOND")
        .and_then(parse)
        .unwrap_or(SIMULATIONS_PER_SECOND_DEFAULT) as f32
};
pub const LAG_COMPENSATED_FRAMES: FrameNumber = {
    let v = (MAX_LAG_COMPENSATION_MILLIS as f32 / (1000.0 / SIMULATIONS_PER_SECOND)) as u16;
    FrameNumber::new(v)
};

const SIMULATIONS_PER_SECOND_DEFAULT: u16 = 120;

#[derive(Debug, Hash, PartialEq, Eq, Clone, SystemLabel)]
pub enum PhysicsSystemSetLabel {
    SyncBackend,
    StepSimulation,
    Writeback,
    DetectDespawn,
}

pub struct MuddleSharedPlugin<S: System<In = (), Out = ShouldRun>> {
    main_run_criteria: Mutex<Option<S>>,
    input_stage: Mutex<Option<SystemStage>>,
    post_game_stage: Mutex<Option<SystemStage>>,
    broadcast_updates_stage: Mutex<Option<SystemStage>>,
    post_tick_stage: Mutex<Option<SystemStage>>,
    link_conditioner: Option<LinkConditionerConfig>,
}

impl<S: System<In = (), Out = ShouldRun>> MuddleSharedPlugin<S> {
    pub fn new(
        main_run_criteria: S,
        input_stage: SystemStage,
        post_game_stage: SystemStage,
        broadcast_updates_stage: SystemStage,
        post_tick_stage: SystemStage,
        link_conditioner: Option<LinkConditionerConfig>,
    ) -> Self {
        Self {
            main_run_criteria: Mutex::new(Some(main_run_criteria)),
            input_stage: Mutex::new(Some(input_stage)),
            post_game_stage: Mutex::new(Some(post_game_stage)),
            broadcast_updates_stage: Mutex::new(Some(broadcast_updates_stage)),
            post_tick_stage: Mutex::new(Some(post_tick_stage)),
            link_conditioner,
        }
    }
}

impl<S: System<In = (), Out = ShouldRun>> Plugin for MuddleSharedPlugin<S> {
    fn build(&self, app: &mut App) {
        app.add_plugin(RapierResourcesPlugin);
        app.add_plugin(NetworkingPlugin {
            link_conditioner: self.link_conditioner.clone(),
            ..NetworkingPlugin::default()
        });

        let mut main_run_criteria = self
            .main_run_criteria
            .lock()
            .expect("Can't initialize the plugin more than once");
        let mut input_stage = self
            .input_stage
            .lock()
            .expect("Can't initialize the plugin more than once");
        let mut post_game_stage = self
            .post_game_stage
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
            .with_run_criteria(IntoSystem::into_system(simulation_tick_run_criteria))
            .with_stage(
                stage::SPAWN,
                SystemStage::single_threaded()
                    .with_system(Events::<CollisionEvent>::update_system)
                    .with_system(Events::<PlayerFinish>::update_system)
                    .with_system(Events::<PlayerDeath>::update_system)
                    .with_system(switch_player_role.label("player_role"))
                    .with_system(
                        despawn_players
                            .label("despawn_players")
                            .after("player_role"),
                    )
                    .with_system(despawn_level_objects)
                    // Updating level objects might despawn entities completely if they are
                    // updated with replacement. Running it before `despawn_level_objects` might
                    // result into an edge-case where changes to the `Spawned` component are not
                    // propagated.
                    .with_system(update_level_objects.after(despawn_level_objects))
                    // Adding components to an entity if there's a command to remove it the queue
                    // will lead to crash. Executing this system before `update_level_objects` helps
                    // to avoid this scenario.
                    .with_system(poll_calculating_shapes.before(update_level_objects))
                    .with_system(maintain_available_spawn_areas.after(update_level_objects))
                    .with_system(
                        spawn_players
                            .after(despawn_players)
                            .after(maintain_available_spawn_areas),
                    ),
            )
            .with_stage(
                stage::PRE_GAME,
                SystemStage::single_threaded()
                    .with_system(update_level_object_movement_route_settings),
            )
            .with_stage(
                stage::GAME,
                SystemStage::single_threaded()
                    .with_system(isolate_client_mispredicted_world)
                    .with_system(player_movement)
                    .with_system(process_objects_route_graph)
                    .with_system(load_object_positions.after(process_objects_route_graph)),
            )
            .with_stage(
                stage::PHYSICS,
                SystemStage::single_threaded()
                    .with_system_set(
                        RapierPhysicsPlugin::<()>::get_systems(PhysicsStages::SyncBackend)
                            .label(PhysicsSystemSetLabel::SyncBackend),
                    )
                    .with_system_set(
                        RapierPhysicsPlugin::<()>::get_systems(PhysicsStages::StepSimulation)
                            .label(PhysicsSystemSetLabel::StepSimulation)
                            .after(PhysicsSystemSetLabel::SyncBackend),
                    )
                    .with_system_set(
                        RapierPhysicsPlugin::<()>::get_systems(PhysicsStages::Writeback)
                            .label(PhysicsSystemSetLabel::Writeback)
                            .after(PhysicsSystemSetLabel::StepSimulation),
                    ),
            )
            .with_stage(
                stage::POST_PHYSICS,
                SystemStage::single_threaded()
                    .with_system(
                        process_collision_events.chain(process_players_with_new_collisions),
                    )
                    .with_system(sync_position)
                    .with_system_set(RapierPhysicsPlugin::<()>::get_systems(
                        PhysicsStages::DetectDespawn,
                    )),
            )
            .with_stage(
                stage::POST_GAME,
                post_game_stage
                    .take()
                    .expect("Can't initialize the plugin more than once"),
            )
            .with_stage(
                stage::SIMULATION_FINAL,
                SystemStage::single_threaded().with_system(tick_simulation_frame),
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
                    .with_run_criteria(game_tick_run_criteria(TICKS_PER_NETWORK_BROADCAST)),
            )
            .with_stage(
                stage::POST_SIMULATIONS,
                SystemStage::single_threaded()
                    .with_system(tick_game_frame)
                    .with_system(process_spawned_entities.after(tick_game_frame))
                    // Remove disconnected players doesn't depend on ticks, so it's fine.
                    .with_system(remove_disconnected_players),
            )
            .with_stage(
                stage::POST_TICK,
                post_tick_stage
                    .take()
                    .expect("Can't initialize the plugin more than once")
                    .with_system_set(RapierPhysicsPlugin::<()>::get_systems(
                        PhysicsStages::DetectDespawn,
                    )),
            );

        app.add_stage_before(
            bevy::app::CoreStage::Update,
            stage::MAIN_SCHEDULE,
            main_schedule,
        );
        app.add_stage_before(
            stage::MAIN_SCHEDULE,
            stage::READ_INPUT_UPDATES,
            SystemStage::single_threaded()
                .with_system(restart_game.exclusive_system().label("restart_game"))
                .with_system_set(
                    SystemSet::on_update(GameState::Playing)
                        .after("restart_game")
                        .with_system(read_movement_updates),
                ),
        );
        app.add_stage_before(
            stage::READ_INPUT_UPDATES,
            stage::WRITE_INPUT_UPDATES,
            input_stage
                .take()
                .expect("Can't initialize the plugin more than once"),
        );

        // Is `GameState::Paused` for client (see `init_state`).
        app.add_state(GameState::Playing);
        app.add_state_to_stage(stage::READ_INPUT_UPDATES, GameState::Playing);

        app.add_startup_system(network_setup);

        #[cfg(feature = "client")]
        app.add_startup_system(client::assets::init_muddle_assets);

        let world = &mut app.world;
        world.get_resource_or_insert_with(GameTime::default);
        world.get_resource_or_insert_with(SimulationTime::default);
        world.get_resource_or_insert_with(LevelState::default);
        world.get_resource_or_insert_with(PlayerUpdates::default);
        world.get_resource_or_insert_with(DeferredQueue::<RestartGame>::default);
        world.get_resource_or_insert_with(DeferredQueue::<SpawnPlayer>::default);
        world.get_resource_or_insert_with(DeferredQueue::<DespawnPlayer>::default);
        world.get_resource_or_insert_with(DeferredQueue::<UpdateLevelObject>::default);
        world.get_resource_or_insert_with(DeferredQueue::<DespawnLevelObject>::default);
        world.get_resource_or_insert_with(DeferredQueue::<SwitchPlayerRole>::default);
        world.get_resource_or_insert_with(EntityRegistry::<PlayerNetId>::default);
        world.get_resource_or_insert_with(EntityRegistry::<EntityNetId>::default);
        world.get_resource_or_insert_with(HashMap::<PlayerNetId, Player>::default);
        world.get_resource_or_insert_with(Events::<CollisionLogicChanged>::default);
        world.get_resource_or_insert_with(Events::<PlayerDeath>::default);
        world.get_resource_or_insert_with(Events::<PlayerFinish>::default);
        // Is used only on the server side.
        world.get_resource_or_insert_with(DeferredMessagesQueue::<SwitchRole>::default);

        let (shape_sender, shape_receiver) =
            crossbeam_channel::unbounded::<ColliderShapePromiseResult>();
        world.insert_resource(shape_sender);
        world.insert_resource(shape_receiver);
    }
}

pub struct RapierResourcesPlugin;

impl Plugin for RapierResourcesPlugin {
    fn build(&self, builder: &mut App) {
        builder
            .insert_resource(RapierConfiguration {
                gravity: Vec2::ZERO,
                timestep_mode: TimestepMode::Fixed {
                    dt: 1.0 / SIMULATIONS_PER_SECOND as f32,
                    substeps: 1,
                },
                ..RapierConfiguration::default()
            })
            .insert_resource(SimulationToRenderTime::default())
            .insert_resource(RapierContext::default())
            .insert_resource(Events::<CollisionEvent>::default())
            .insert_resource(PhysicsHooksWithQueryResource::<()>(Box::new(())));
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub enum GameState {
    Paused,
    Playing,
}

impl Default for GameState {
    fn default() -> Self {
        Self::Paused
    }
}

#[derive(Default, Clone, Debug, PartialEq, Eq)]
pub struct GameTime {
    pub session: usize,
    pub frame_number: FrameNumber,
}

#[derive(Debug)]
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
            if self.server_frame.diff_abs(frame_number).value() > u16::MAX / 2 {
                // This shouldn't overflow as we start counting from 1, and we never decrement
                // more than once without incrementing.
                self.server_generation -= 1;
            }
            self.server_frame = frame_number;
            self.player_frame = frame_number;
            self.player_generation = self.server_generation;
        } else if self.player_frame > frame_number {
            if self.player_frame.diff_abs(frame_number).value() > u16::MAX / 2 {
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

fn game_tick_run_criteria(ticks_per_step: u16) -> impl System<In = (), Out = ShouldRun> {
    IntoSystem::into_system(
        move |mut state: Local<GameTickRunCriteriaState>, time: Res<GameTime>| -> ShouldRun {
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
                ShouldRun::YesAndCheckAgain
            } else {
                trace!("Don't run a game schedule (game {})", time.frame_number);
                ShouldRun::No
            }
        },
    )
}

#[derive(Default, Clone)]
pub struct SimulationTickRunCriteriaState {
    last_game_frame: Option<FrameNumber>,
    last_player_frame: FrameNumber,
    last_server_frame: FrameNumber,
}

fn simulation_tick_run_criteria(
    mut state: Local<SimulationTickRunCriteriaState>,
    game_state: Res<State<GameState>>,
    game_time: Res<GameTime>,
    simulation_time: Res<SimulationTime>,
) -> ShouldRun {
    #[cfg(feature = "profiler")]
    puffin::profile_function!();
    // Checking that a game frame has changed helps us to avoid the panicking in
    // case we rewind simulation frame just 1 frame back.
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

pub fn tick_simulation_frame(mut time: ResMut<SimulationTime>) {
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

pub fn tick_game_frame(mut time: ResMut<GameTime>) {
    #[cfg(feature = "profiler")]
    puffin::profile_function!();
    log::trace!("Concluding game frame tick: {}", time.frame_number.value());
    time.frame_number += FrameNumber::new(1);
}
