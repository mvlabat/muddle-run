use bevy::{log, log::LogPlugin, prelude::*};
use mr_client_lib::{MuddleClientConfig, MuddleClientPlugin, DEFAULT_SERVER_PORT};
use mr_utils_lib::try_parse_from_env;
use std::net::SocketAddr;

fn main() {
    let mut app = App::new();
    app.add_plugin(bevy_puffin::PuffinTracePlugin::new().with_scopes_off());

    mr_utils_lib::env::load_env();

    let _guard = sentry::init(sentry::ClientOptions {
        release: sentry::release_name!(),
        ..Default::default()
    });

    app.insert_resource(MuddleClientConfig {
        persistence_url: try_parse_from_env!("MUDDLE_PUBLIC_PERSISTENCE_URL"),
        google_client_id: try_parse_from_env!("MUDDLE_GOOGLE_CLIENT_ID"),
        google_client_secret: try_parse_from_env!("MUDDLE_GOOGLE_CLIENT_SECRET"),
        auth0_client_id: try_parse_from_env!("MUDDLE_AUTH0_CLIENT_ID"),
        matchmaker_url: try_parse_from_env!("MUDDLE_MATCHMAKER_URL"),
        server_addr: server_addr(),
    })
    // Window and rendering.
    .insert_resource(WindowDescriptor {
        title: "Muddle Run".to_owned(),
        width: 1024.0,
        height: 768.0,
        ..Default::default()
    })
    .insert_resource(Msaa { samples: 4 })
    .add_plugins_with(DefaultPlugins, |plugin| plugin.disable::<LogPlugin>())
    .add_plugin(MuddleClientPlugin)
    .run();
}

fn server_addr() -> Option<SocketAddr> {
    let port: u16 = try_parse_from_env!("MUDDLE_SERVER_PORT").unwrap_or(DEFAULT_SERVER_PORT);
    try_parse_from_env!("MUDDLE_SERVER_IP_ADDR").map(|ip_addr| SocketAddr::new(ip_addr, port))
}
