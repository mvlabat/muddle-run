use bevy::{app::App, log};
use mr_server_lib::{
    init_level_data, watch_agones_updates, Agones, MuddleServerConfig, MuddleServerPlugin,
    PlayerEvent, TOKIO,
};
use mr_utils_lib::try_parse_from_env;
use std::{ops::Deref, time::Duration};

fn main() {
    let mut app = App::new();
    app.add_plugin(bevy::log::LogPlugin::default());

    mr_utils_lib::env::load_env();

    // We want to exit the process on any panic (in any thread), so this is why the custom hook.
    let orig_hook = std::panic::take_hook();

    // And when I declared to my design
    // Like Frankenstein's monster
    // "I am your father, I am your god
    // And you the magic that I conjure"
    // — Stu Mackenzie
    std::panic::set_hook(Box::new(move |panic_info| {
        orig_hook(panic_info);

        // TODO: track https://github.com/dimforge/rapier/issues/223 and fix `calculate_collider_shape` to avoid unwinding panics.
        if let Some(panic_info) = panic_info.location() {
            if panic_info.file().contains("parry") {
                return;
            }
        }

        // A kludge to let sentry send events first and then shutdown.
        std::thread::spawn(|| {
            std::thread::sleep(std::time::Duration::new(1, 0));
            std::process::exit(1);
        });
    }));

    let _guard = sentry::init(sentry::ClientOptions {
        release: sentry::release_name!(),
        ..Default::default()
    });

    // Spawn the runtime on some other thread.
    std::thread::spawn(|| TOKIO.deref()).join().unwrap();

    let agones_sdk_grpc_port: Option<u16> = try_parse_from_env!("AGONES_SDK_GRPC_PORT");
    let (player_tracking_tx, mut player_tracking_rx) =
        tokio::sync::mpsc::unbounded_channel::<PlayerEvent>();
    let agones = agones_sdk_grpc_port.map(|grpc_port| {
        log::info!("Connecting to Agones...");
        TOKIO
            .block_on(async {
                let (sdk, game_server) =
                    rymder::Sdk::connect(Some(grpc_port), Some(Duration::from_secs(2)), None)
                        .await?;

                let mut watch_client = sdk.clone();
                let mut player_tracking_client = sdk.clone();

                if let Some(health_spec) = &game_server.health_spec {
                    let health_check = sdk.health_check();
                    let period = health_spec.period;
                    tokio::spawn(async move {
                        let mut interval = tokio::time::interval(period);
                        loop {
                            interval.tick().await;
                            log::debug!("Sending Health check message...");
                            if let Err(err) = health_check.send(()).await {
                                log::error!("Failed to send Health message: {:?}", err);
                            }
                        }
                    });
                }

                tokio::spawn(async move {
                    while let Some(player_event) = player_tracking_rx.recv().await {
                        let result = match player_event {
                            PlayerEvent::Connected(uuid) => {
                                log::info!("Sending PlayerConnect event ({})...", uuid);
                                player_tracking_client.player_connect(uuid).await
                            }
                            PlayerEvent::Disconnected(uuid) => {
                                log::info!("Sending PlayerDisconnect event ({})...", uuid);
                                player_tracking_client.player_disconnect(uuid).await
                            }
                        };
                        if let Err(err) = result {
                            log::error!("Failed to report a player event to Agones: {:?}", err);
                        }
                    }
                    log::warn!("Player tracking channel is closed");
                });

                tokio::spawn(async move {
                    use rymder::futures_util::stream::StreamExt;

                    log::info!("Starting to watch GameServer updates...");
                    match watch_client.watch_gameserver().await {
                        Err(e) => eprintln!("Failed to watch for GameServer updates: {}", e),
                        Ok(mut stream) => loop {
                            // We've received a new update, or the stream is shutting down
                            match stream.next().await {
                                Some(Ok(gs)) => {
                                    log::debug!("GameServer update: {:?}", gs);
                                }
                                Some(Err(e)) => {
                                    log::error!(
                                        "GameServer Update stream encountered an error: {}",
                                        e
                                    );
                                }
                                None => {
                                    log::info!("Server closed the GameServer watch stream");
                                    break;
                                }
                            }
                        },
                    }
                });

                Ok::<_, rymder::Error>(Agones { sdk, game_server })
            })
            .unwrap()
    });

    let game_server = agones.map(|agones| {
        let allocated_game_server_rx = watch_agones_updates(agones.sdk.clone());
        app.insert_resource(agones);
        app.insert_resource(player_tracking_tx);
        match allocated_game_server_rx.blocking_recv() {
            Ok(game_server) => game_server,
            Err(err) => {
                log::error!("Failed to receive Agones allocation status: {err:?}");
                std::process::exit(1);
            }
        }
    });
    app.insert_resource(MuddleServerConfig {
        public_persistence_url: try_parse_from_env!("MUDDLE_PUBLIC_PERSISTENCE_URL"),
        private_persistence_url: try_parse_from_env!("MUDDLE_PRIVATE_PERSISTENCE_URL"),
        idle_timeout_millis: try_parse_from_env!("MUDDLE_IDLE_TIMEOUT"),
        listen_port: try_parse_from_env!("MUDDLE_LISTEN_PORT"),
        listen_ip_addr: try_parse_from_env!("MUDDLE_LISTEN_IP_ADDR"),
        public_ip_addr: try_parse_from_env!("MUDDLE_PUBLIC_IP_ADDR"),
    });
    TOKIO.block_on(async { init_level_data(&mut app, game_server).await });
    app.add_plugin(MuddleServerPlugin).run();
}
