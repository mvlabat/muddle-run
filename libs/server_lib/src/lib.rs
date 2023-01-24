#![feature(hash_drain_filter)]
#![feature(once_cell)]

pub use crate::net::watch_agones_updates;
pub use mr_shared_lib::{game::PlayerEventSender, player::PlayerEvent};

use crate::{
    game_events::{process_player_events_system, process_scheduled_spawns_system},
    net::{
        broadcast_disconnected_players_system, process_network_events_system,
        send_network_updates_system, startup, ConnectionStates, FetchedLevelInfo,
        NewPlayerConnections, PlayerConnections,
    },
    persistence::{
        create_level, get_user, handle_persistence_requests, init_jwks_polling, load_level,
        save_level_system, InitLevelObjects, Jwks, PersistenceConfig, PersistenceMessage,
        PersistenceRequest,
    },
    player_updates::{
        process_despawn_level_object_requests_system, process_player_input_updates_system,
        process_spawn_level_object_requests_system, process_switch_role_requests_system,
        process_update_level_object_requests_system,
    },
};
use bevy::{
    log,
    prelude::*,
    time::{FixedTimestep, TimePlugin},
};
use iyes_loopless::prelude::*;
use kube::Client;
use mr_messages_lib::{InitLevel, LevelData};
use mr_shared_lib::{
    framebuffer::FrameNumber,
    game::{
        commands::{DeferredPlayerQueues, DeferredQueue, DespawnLevelObject, UpdateLevelObject},
        level::{CollisionLogic, LevelObject, LevelObjectDesc},
        level_objects::{PlaneDesc, PlaneFormDesc},
    },
    messages::{
        self, DeferredMessagesQueue, EntityNetId, EntityNetIdCounter, PlayerNetIdCounter,
        RespawnPlayer, RunnerInput, SpawnLevelObject, SpawnLevelObjectRequest,
    },
    player::{PlayerRole, Players},
    registry::IncrementId,
    AppState, GameSessionState, LevelObjectsToSpawnToLoad, MuddleSharedPlugin,
    SIMULATIONS_PER_SECOND,
};
use mr_utils_lib::kube_discovery;
use reqwest::Url;
use rymder::GameServer;
use std::{
    net::IpAddr,
    sync::LazyLock,
    time::{Duration, Instant},
};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};

mod game_events;
mod net;
mod persistence;
mod player_updates;

pub const DEFAULT_IDLE_TIMEOUT_MILLIS: u64 = 300_000;

#[derive(Resource)]
pub struct Agones {
    pub sdk: rymder::Sdk,
    pub game_server: GameServer,
}

#[derive(Resource)]
pub struct LastPlayerDisconnectedAt(pub Instant);

#[derive(Resource)]
pub struct IdleTimeout(pub Duration);

pub static TOKIO: LazyLock<tokio::runtime::Runtime> = LazyLock::new(|| {
    std::thread::Builder::new()
        .name("tokio".to_string())
        .spawn(move || TOKIO.block_on(std::future::pending::<()>()))
        .unwrap();
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("Cannot start tokio runtime")
});

#[derive(Resource, Clone, Debug)]
pub struct MuddleServerConfig {
    pub public_persistence_url: Option<Url>,
    pub private_persistence_url: Option<Url>,
    pub idle_timeout_millis: Option<u64>,
    pub listen_port: Option<u16>,
    pub listen_ip_addr: Option<IpAddr>,
    pub public_ip_addr: Option<IpAddr>,
}

#[derive(Resource, DerefMut, Deref)]
pub struct PersistenceRequestSender(pub Option<UnboundedSender<PersistenceRequest>>);
#[derive(Resource, DerefMut, Deref)]
pub struct PersistenceRequestReceiver(pub Option<UnboundedReceiver<PersistenceRequest>>);
#[derive(Resource, DerefMut, Deref)]
pub struct PersistenceMessageSender(pub Option<UnboundedSender<PersistenceMessage>>);
#[derive(Resource, DerefMut, Deref)]
pub struct PersistenceMessageReceiver(pub Option<UnboundedReceiver<PersistenceMessage>>);

pub struct MuddleServerPlugin;

impl Plugin for MuddleServerPlugin {
    fn build(&self, app: &mut App) {
        // The minimal set of Bevy plugins needed for the game logic.
        app.add_plugin(CorePlugin::default());
        app.add_plugin(TimePlugin::default());
        app.add_plugin(TransformPlugin::default());
        app.add_plugin(bevy::diagnostic::DiagnosticsPlugin::default());
        app.add_plugin(bevy::app::ScheduleRunnerPlugin::default());

        app.add_startup_system(init_level);
        app.add_startup_system(startup);

        let server_config = app
            .world
            .get_resource::<MuddleServerConfig>()
            .expect("Expected MuddleServerConfig")
            .clone();
        let persistence_urls: Option<(Url, Url)> = server_config
            .public_persistence_url
            .zip(server_config.private_persistence_url);
        if let Some((public_url, private_url)) = persistence_urls {
            let config = PersistenceConfig {
                public_url,
                private_url,
                google_web_client_id: std::env::var("MUDDLE_GOOGLE_WEB_CLIENT_ID")
                    .expect("Expected MUDDLE_WEB_GOOGLE_CLIENT_ID"),
                google_desktop_client_id: std::env::var("MUDDLE_GOOGLE_DESKTOP_CLIENT_ID")
                    .expect("Expected MUDDLE_DESKTOP_GOOGLE_CLIENT_ID"),
                auth0_client_id: std::env::var("MUDDLE_AUTH0_CLIENT_ID")
                    .expect("Expected MUDDLE_AUTH0_CLIENT_ID"),
            };
            app.insert_resource(config);
            let (persistence_req_tx, persistence_req_rx) =
                tokio::sync::mpsc::unbounded_channel::<PersistenceRequest>();
            let (persistence_msg_tx, persistence_msg_rx) =
                tokio::sync::mpsc::unbounded_channel::<PersistenceMessage>();
            app.insert_resource(PersistenceRequestSender(Some(persistence_req_tx)));
            app.insert_resource(PersistenceRequestReceiver(Some(persistence_req_rx)));
            app.insert_resource(PersistenceMessageSender(Some(persistence_msg_tx)));
            app.insert_resource(PersistenceMessageReceiver(Some(persistence_msg_rx)));
        } else {
            log::info!("Persistence service isn't available");
            app.insert_resource(PersistenceRequestSender(None));
            app.insert_resource(PersistenceRequestReceiver(None));
            app.insert_resource(PersistenceMessageSender(None));
            app.insert_resource(PersistenceMessageReceiver(None));
        }
        app.add_startup_system(init_jwks_polling);
        app.add_startup_system(handle_persistence_requests);

        app.add_system(process_idle_timeout);

        let input_stage = SystemStage::parallel()
            .with_system(process_scheduled_spawns_system)
            .with_system(process_network_events_system)
            .with_system(process_player_input_updates_system.after(process_network_events_system))
            .with_system(process_switch_role_requests_system.after(process_network_events_system))
            // It's ok to run the following in random order since object updates aren't possible
            // on the client before an authoritative confirmation that an object has been spawned.
            .with_system(
                process_spawn_level_object_requests_system.after(process_network_events_system),
            )
            .with_system(
                process_update_level_object_requests_system.after(process_network_events_system),
            )
            .with_system(
                process_despawn_level_object_requests_system.after(process_network_events_system),
            );
        let post_game_stage = SystemStage::single_threaded()
            .with_system(process_player_events_system)
            .with_system(save_level_system);
        let broadcast_updates_stage = SystemStage::single_threaded()
            .with_system(broadcast_disconnected_players_system)
            .with_system(send_network_updates_system.run_in_state(GameSessionState::Playing));

        // Game.
        app.add_plugin(MuddleSharedPlugin::new(
            FixedTimestep::steps_per_second(SIMULATIONS_PER_SECOND as f64),
            input_stage,
            post_game_stage,
            broadcast_updates_stage,
            SystemStage::single_threaded(),
            None,
        ));

        // We override the initial state for server as we aren't using the loading state
        // atm.
        app.insert_resource(CurrentState(AppState::Playing));

        app.init_resource::<EntityNetIdCounter>();
        app.init_resource::<PlayerNetIdCounter>();
        app.init_resource::<PlayerConnections>();
        app.init_resource::<NewPlayerConnections>();
        app.init_resource::<ConnectionStates>();
        app.init_resource::<DeferredPlayerQueues<RunnerInput>>();
        app.init_resource::<DeferredPlayerQueues<PlayerRole>>();
        app.init_resource::<DeferredPlayerQueues<messages::SpawnLevelObjectRequestBody>>();
        app.init_resource::<DeferredPlayerQueues<SpawnLevelObjectRequest>>();
        app.init_resource::<DeferredPlayerQueues<LevelObject>>();
        app.init_resource::<DeferredPlayerQueues<EntityNetId>>();
        app.init_resource::<DeferredMessagesQueue<RespawnPlayer>>();
        app.init_resource::<DeferredMessagesQueue<SpawnLevelObject>>();
        app.init_resource::<DeferredMessagesQueue<UpdateLevelObject>>();
        app.init_resource::<DeferredMessagesQueue<DespawnLevelObject>>();
        app.insert_resource(LastPlayerDisconnectedAt(Instant::now()));
        app.insert_resource(IdleTimeout(
            server_config
                .idle_timeout_millis
                .map(Duration::from_millis)
                .unwrap_or_else(|| {
                    log::info!(
                        "Using the default value for MUDDLE_IDLE_TIMEOUT: {}",
                        DEFAULT_IDLE_TIMEOUT_MILLIS
                    );
                    Duration::from_millis(DEFAULT_IDLE_TIMEOUT_MILLIS)
                }),
        ));
        app.init_resource::<Jwks>();
    }
}

pub async fn init_level_data(app: &mut App, game_server: Option<GameServer>) {
    let (user_id, init_level) = if let Some(game_server) = game_server {
        let metadata = game_server
            .object_meta
            .expect("Expected GameServer metadata");
        read_env_level_data(
            metadata.annotations.get("user_id").cloned(),
            metadata.annotations.get("level_title").cloned(),
            metadata.annotations.get("level_parent_id").cloned(),
            metadata.annotations.get("level_id").cloned(),
        )
    } else {
        let user_id = mr_utils_lib::var!("MUDDLE_USER_ID");
        let title = mr_utils_lib::var!("MUDDLE_LEVEL_TITLE");
        let parent_id = mr_utils_lib::var!("MUDDLE_LEVEL_PARENT_ID");
        let level_id = mr_utils_lib::var!("MUDDLE_LEVEL_ID");
        if user_id.is_some() || title.is_some() || parent_id.is_some() || level_id.is_some() {
            read_env_level_data(user_id, title, parent_id, level_id)
        } else {
            app.world
                .insert_resource(InitLevelObjects(default_level_objects()));
            return;
        }
    };

    let mut server_config = app.world.get_resource_mut::<MuddleServerConfig>().unwrap();
    if server_config.public_persistence_url.is_none()
        || server_config.private_persistence_url.is_none()
    {
        match Client::try_default().await {
            Ok(client) => {
                let persistence_urls = kube_discovery::discover_persistence(client).await;
                if let Some((public_persistence_url, private_persistence_url)) = persistence_urls {
                    server_config.public_persistence_url = Some(public_persistence_url);
                    server_config.private_persistence_url = Some(private_persistence_url);
                }
            }
            Err(err) => {
                log::warn!("Unable to detect kubernetes environment: {err:?}");
            }
        }
    }

    let public_persistence_url = server_config
        .public_persistence_url
        .clone()
        .expect("Expected public_persistence_url when booting from the Agones environment or requesting a level via the env variables");
    let private_persistence_url = server_config
        .private_persistence_url
        .clone()
        .expect("Expected private_persistence_url when booting from the Agones environment or requesting a level via the env variables");

    let (get_level_response, init_level_objects) = match init_level {
        InitLevel::Existing(id) => load_level(public_persistence_url, id)
            .await
            .expect("Failed to load the level"),
        InitLevel::Create { title, parent_id } => {
            let user_id =
                user_id.expect("Expected `user_id` when creating a new level is requested");
            let user = get_user(public_persistence_url, user_id)
                .await
                .expect("Failed to get user info");
            let level_data = match parent_id {
                Some(parent_id) => LevelData::Forked { parent_id },
                None => LevelData::Data {
                    data: serde_json::to_value(default_level_objects()).unwrap(),
                },
            };
            let level_response = create_level(
                private_persistence_url,
                user_id,
                user.display_name,
                title,
                level_data,
            )
            .await
            .expect("Failed to create a level");
            let level_objects = serde_json::from_value(level_response.level.data.clone()).unwrap();
            (level_response, InitLevelObjects(level_objects))
        }
    };
    app.world.insert_resource(init_level_objects);
    app.world
        .insert_resource(FetchedLevelInfo(get_level_response));
}

fn read_env_level_data(
    user_id: Option<String>,
    title: Option<String>,
    parent_id: Option<String>,
    level_id: Option<String>,
) -> (Option<i64>, InitLevel) {
    let user_id = user_id.map(|user_id| user_id.parse().expect("Failed to parse `user_id`"));
    let init_level = if let Some(title) = title {
        InitLevel::Create {
            title,
            parent_id: parent_id.map(|id| id.parse().expect("Failed to parse `level_parent_id`")),
        }
    } else {
        let level_id = level_id
            .expect("Expected a `level_id` annotation or `level_title` one for a new level (`MUDDLE_LEVEL_ID` or `MUDDLE_LEVEL_TITLE` env vars respectively)")
            .parse()
            .expect("Failed to parse `level_id`");
        InitLevel::Existing(level_id)
    };
    (user_id, init_level)
}

fn default_level_objects() -> Vec<LevelObject> {
    let mut entity_net_id_counter = EntityNetId(0);
    vec![LevelObject {
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
    }]
}

pub fn init_level(
    mut commands: Commands,
    mut init_level_objects: ResMut<InitLevelObjects>,
    mut entity_net_id_counter: ResMut<EntityNetIdCounter>,
    mut spawn_level_object_commands: ResMut<DeferredQueue<UpdateLevelObject>>,
) {
    let level_objects_to_spawn = std::mem::take(&mut init_level_objects.0);
    commands.insert_resource(LevelObjectsToSpawnToLoad(level_objects_to_spawn.len()));
    log::info!(
        "Level objects to spawn to load: {}",
        level_objects_to_spawn.len()
    );

    for level_object in level_objects_to_spawn {
        assert_eq!(entity_net_id_counter.increment(), level_object.net_id);
        spawn_level_object_commands.push(UpdateLevelObject {
            frame_number: FrameNumber::new(0),
            object: level_object,
        });
    }
}

pub fn process_idle_timeout(
    mut is_shutting_down: Local<bool>,
    idle_timeout: Res<IdleTimeout>,
    last_player_disconnected_at: Res<LastPlayerDisconnectedAt>,
    players: Res<Players>,
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
