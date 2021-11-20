#![feature(bool_to_option)]
#![feature(hash_drain_filter)]
#![feature(once_cell)]

pub use mr_shared_lib::try_parse_from_env;

use crate::{
    game_events::{process_player_events, process_scheduled_spawns},
    net::{process_network_events, send_network_updates, startup, PlayerConnections},
    player_updates::{
        process_despawn_level_object_requests, process_player_input_updates,
        process_spawn_level_object_requests, process_switch_role_requests,
        process_update_level_object_requests,
    },
};
use bevy::{core::FixedTimestep, log, prelude::*, utils::HashMap};
use mr_shared_lib::{
    framebuffer::FrameNumber,
    game::{
        commands::{DeferredPlayerQueues, DeferredQueue, DespawnLevelObject, UpdateLevelObject},
        level::{CollisionLogic, LevelObject, LevelObjectDesc},
        level_objects::{PlaneDesc, PlaneFormDesc},
    },
    messages::{
        self, DeferredMessagesQueue, EntityNetId, PlayerNetId, RespawnPlayer, RunnerInput,
        SpawnLevelObject, SpawnLevelObjectRequest,
    },
    net::ConnectionState,
    player::{Player, PlayerRole},
    registry::IncrementId,
    simulations_per_second, MuddleSharedPlugin,
};
use std::{
    lazy::SyncLazy,
    time::{Duration, Instant},
};

mod game_events;
mod net;
mod player_updates;

pub struct Agones {
    pub sdk: rymder::Sdk,
    pub game_server: rymder::GameServer,
}

pub struct LastPlayerDisconnectedAt(pub Instant);

pub struct IdleTimeout(pub Duration);

pub static TOKIO: SyncLazy<tokio::runtime::Runtime> = SyncLazy::new(|| {
    std::thread::Builder::new()
        .name("tokio".to_string())
        .spawn(move || TOKIO.block_on(std::future::pending::<()>()))
        .unwrap();
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("Cannot start tokio runtime")
});

pub struct MuddleServerPlugin;

impl Plugin for MuddleServerPlugin {
    fn build(&self, builder: &mut AppBuilder) {
        // The minimal set of Bevy plugins needed for the game logic.
        builder.add_plugin(bevy::log::LogPlugin::default());
        builder.add_plugin(bevy::core::CorePlugin::default());
        builder.add_plugin(bevy::transform::TransformPlugin::default());
        builder.add_plugin(bevy::diagnostic::DiagnosticsPlugin::default());
        builder.add_plugin(bevy::app::ScheduleRunnerPlugin::default());

        builder.add_startup_system(init_level.system());
        builder.add_startup_system(startup.system());
        builder.add_system(process_idle_timeout.system());

        let input_stage = SystemStage::parallel()
            .with_system(process_scheduled_spawns.system())
            .with_system(process_network_events.system().label("net"))
            .with_system(process_player_input_updates.system().after("net"))
            .with_system(process_switch_role_requests.system().after("net"))
            // It's ok to run the following in random order since object updates aren't possible
            // on the client before an authoritative confirmation that an object has been spawned.
            .with_system(process_spawn_level_object_requests.system().after("net"))
            .with_system(process_update_level_object_requests.system().after("net"))
            .with_system(process_despawn_level_object_requests.system().after("net"));
        let post_game_stage = SystemStage::parallel().with_system(process_player_events.system());
        let broadcast_updates_stage =
            SystemStage::parallel().with_system(send_network_updates.system());

        // Game.
        builder.add_plugin(MuddleSharedPlugin::new(
            FixedTimestep::steps_per_second(simulations_per_second() as f64),
            input_stage,
            post_game_stage,
            broadcast_updates_stage,
            SystemStage::parallel(),
            None,
        ));

        let resources = builder.world_mut();
        resources.get_resource_or_insert_with(EntityNetId::default);
        resources.get_resource_or_insert_with(PlayerNetId::default);
        resources.get_resource_or_insert_with(PlayerConnections::default);
        resources.get_resource_or_insert_with(Vec::<(PlayerNetId, u32)>::default);
        resources.get_resource_or_insert_with(HashMap::<u32, ConnectionState>::default);
        resources.get_resource_or_insert_with(DeferredPlayerQueues::<RunnerInput>::default);
        resources.get_resource_or_insert_with(DeferredPlayerQueues::<PlayerRole>::default);
        resources.get_resource_or_insert_with(
            DeferredPlayerQueues::<messages::SpawnLevelObjectRequestBody>::default,
        );
        resources
            .get_resource_or_insert_with(DeferredPlayerQueues::<SpawnLevelObjectRequest>::default);
        resources.get_resource_or_insert_with(DeferredPlayerQueues::<LevelObject>::default);
        resources.get_resource_or_insert_with(DeferredPlayerQueues::<EntityNetId>::default);
        resources.get_resource_or_insert_with(DeferredMessagesQueue::<RespawnPlayer>::default);
        resources.get_resource_or_insert_with(DeferredMessagesQueue::<SpawnLevelObject>::default);
        resources.get_resource_or_insert_with(DeferredMessagesQueue::<UpdateLevelObject>::default);
        resources.get_resource_or_insert_with(DeferredMessagesQueue::<DespawnLevelObject>::default);
        resources.get_resource_or_insert_with(|| LastPlayerDisconnectedAt(Instant::now()));
        resources.get_resource_or_insert_with(|| IdleTimeout(idle_timeout()));
    }
}

pub fn init_level(
    agones: Option<Res<Agones>>,
    mut entity_net_id_counter: ResMut<EntityNetId>,
    mut spawn_level_object_commands: ResMut<DeferredQueue<UpdateLevelObject>>,
) {
    if let Some(agones) = agones {
        let mut sdk = agones.sdk.clone();
        TOKIO.spawn(async move {
            if let Err(err) = sdk.allocate().await {
                log::error!(
                    "Failed to mark the Game Server as ready, exiting: {:?}",
                    err
                );
                std::process::exit(1);
            }
        });
    }

    spawn_level_object_commands.push(UpdateLevelObject {
        frame_number: FrameNumber::new(0),
        object: LevelObject {
            net_id: entity_net_id_counter.increment(),
            label: "Ground".to_owned(),
            desc: LevelObjectDesc::Plane(PlaneDesc {
                position: Vec2::ZERO,
                form_desc: PlaneFormDesc::Concave {
                    points: vec![
                        Vec2::new(-8.0, -5.0),
                        Vec2::new(8.0, -5.0),
                        Vec2::new(10.0, 5.0),
                        Vec2::new(0.0, 3.50),
                        Vec2::new(-10.0, 5.0),
                    ],
                },
                is_spawn_area: false,
            }),
            route: None,
            collision_logic: CollisionLogic::None,
        },
    });
}

pub fn process_idle_timeout(
    idle_timeout: Res<IdleTimeout>,
    last_player_disconnected_at: Res<LastPlayerDisconnectedAt>,
    players: Res<HashMap<PlayerNetId, Player>>,
    agones: Option<Res<Agones>>,
) {
    if players.is_empty()
        && Instant::now().duration_since(last_player_disconnected_at.0) > idle_timeout.0
    {
        log::info!("Shutting down due to being idle...");
        if let Some(agones) = agones {
            let mut sdk = agones.sdk.clone();
            TOKIO.spawn(async move {
                if let Err(err) = sdk.shutdown().await {
                    log::error!("Failed to request shutdown, exiting: {:?}", err);
                    std::process::exit(0);
                }
            });
        } else {
            std::process::exit(0);
        }
    }
}

fn idle_timeout() -> Duration {
    let default = 300_000;
    try_parse_from_env!("MUDDLE_IDLE_TIMEOUT")
        .map(Duration::from_millis)
        .unwrap_or_else(|| {
            log::info!(
                "Using the default value for MUDDLE_IDLE_TIMEOUT: {}",
                default
            );
            Duration::from_millis(default)
        })
}
