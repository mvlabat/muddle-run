use bevy::prelude::*;
use mr_client_lib::MuddleClientPlugin;
use git_version::git_version;

fn main() {
    env_logger::init();

    let release = sentry::release_name!().map(|name| name + "+" + git_version!());
    if let Some(release) = &release {
        log::info!("Release: {}", release);
    }
    let _guard = sentry::init(sentry::ClientOptions {
        release,
        ..Default::default()
    });

    App::build()
        // Window and rendering.
        .insert_resource(WindowDescriptor {
            title: "Muddle Run".to_owned(),
            width: 1024.0,
            height: 768.0,
            ..Default::default()
        })
        .insert_resource(Msaa { samples: 4 })
        .add_plugins(DefaultPlugins)
        .add_plugin(MuddleClientPlugin)
        .run();
}
