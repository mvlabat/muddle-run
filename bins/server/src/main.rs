use bevy::{app::App, log};
use mr_server_lib::{try_parse_from_env, Agones, MuddleServerPlugin, TOKIO};
use std::{ops::Deref, time::Duration};
use git_version::git_version;

fn main() {
    env_logger::init();

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

    let release = sentry::release_name!().map(|name| name + "+" + git_version!());
    if let Some(release) = &release {
        log::info!("Release: {}", release);
    }
    let _guard = sentry::init(sentry::ClientOptions {
        release,
        ..Default::default()
    });

    // Spawn the runtime on some other thread.
    std::thread::spawn(|| TOKIO.deref()).join().unwrap();

    let agones_sdk_grpc_port: Option<u16> = try_parse_from_env!("AGONES_SDK_GRPC_PORT");
    let agones = agones_sdk_grpc_port.map(|grpc_port| {
        log::info!("Connecting to Agones...");
        TOKIO
            .block_on(async {
                let (sdk, game_server) =
                    rymder::Sdk::connect(Some(grpc_port), Some(Duration::from_secs(2)), None)
                        .await?;

                let mut watch_client = sdk.clone();

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

    let mut app_builder = App::build();
    if let Some(agones) = agones {
        app_builder.insert_resource(agones);
    }
    app_builder.add_plugin(MuddleServerPlugin).run();
}
