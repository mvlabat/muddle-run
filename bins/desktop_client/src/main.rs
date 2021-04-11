use bevy::prelude::*;
use mr_client_lib::MuddleClientPlugin;

fn main() {
    env_logger::init();
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
