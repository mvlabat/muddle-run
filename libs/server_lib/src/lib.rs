#![feature(bool_to_option)]
#![feature(hash_drain_filter)]
#![feature(let_else)]
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
use mr_messages_lib::PLAYER_CAPACITY;
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
use reqwest::Url;
use std::{
    lazy::SyncLazy,
    time::{Duration, Instant},
};
use tokio::sync::mpsc::UnboundedReceiver;

mod game_events;
mod kube_discovery;
mod net;
mod persistence;
mod player_updates;

pub struct Agones {
    pub sdk: rymder::Sdk,
    pub game_server: rymder::GameServer,
}

use crate::persistence::{
    handle_persistence_requests, init_jwks_polling, PersistenceConfig, PersistenceMessage,
    PersistenceRequest,
};
pub use mr_shared_lib::player::PlayerEvent;
use mr_utils_lib::jwks::Jwks;

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

        let persistence_url: Option<Url> = try_parse_from_env!("MUDDLE_PERSISTENCE_URL")
            .or_else(|| TOKIO.block_on(kube_discovery::discover_persistence()));
        if let Some(url) = persistence_url {
            let config = PersistenceConfig {
                url,
                google_web_client_id: std::env::var("MUDDLE_GOOGLE_WEB_CLIENT_ID")
                    .expect("Expected MUDDLE_WEB_GOOGLE_CLIENT_ID"),
                google_desktop_client_id: std::env::var("MUDDLE_GOOGLE_DESKTOP_CLIENT_ID")
                    .expect("Expected MUDDLE_DESKTOP_GOOGLE_CLIENT_ID"),
                auth0_client_id: std::env::var("MUDDLE_AUTH0_CLIENT_ID")
                    .expect("Expected MUDDLE_AUTH0_CLIENT_ID"),
            };
            builder.insert_resource(config);
            let (persistence_req_tx, persistence_req_rx) =
                tokio::sync::mpsc::unbounded_channel::<PersistenceRequest>();
            let (persistence_msg_tx, persistence_msg_rx) =
                tokio::sync::mpsc::unbounded_channel::<PersistenceMessage>();
            builder.insert_resource(persistence_req_tx);
            builder.insert_resource(Some(persistence_req_rx));
            builder.insert_resource(persistence_msg_tx);
            builder.insert_resource(persistence_msg_rx);
        } else {
            log::info!("Persistence service isn't available");
            builder.insert_resource::<Option<UnboundedReceiver<PersistenceRequest>>>(None);
        }
        builder.add_startup_system(init_jwks_polling.system());
        builder.add_startup_system(handle_persistence_requests.system());

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
        resources.get_resource_or_insert_with(Jwks::default);
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
            log::info!("Marking the GameServer as Allocated...");
            if let Err(err) = sdk.allocate().await {
                log::error!(
                    "Failed to mark the Game Server as ready, exiting: {:?}",
                    err
                );
                std::process::exit(1);
            }
            log::info!("Setting GameServer player capacity to {}...", PLAYER_CAPACITY);
            if let Err(err) = sdk.set_player_capacity(PLAYER_CAPACITY as u64).await {
                log::error!(
                    "Failed to set Game Server player capacity, exiting: {:?}",
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
    mut is_shutting_down: Local<bool>,
    idle_timeout: Res<IdleTimeout>,
    last_player_disconnected_at: Res<LastPlayerDisconnectedAt>,
    players: Res<HashMap<PlayerNetId, Player>>,
    agones: Option<Res<Agones>>,
) {
    if players.is_empty()
        && Instant::now().duration_since(last_player_disconnected_at.0) > idle_timeout.0
        && !*is_shutting_down
    {
        log::info!("Shutting down due to being idle...");
        *is_shutting_down = true;
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
