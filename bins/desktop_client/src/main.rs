use bevy::prelude::*;
use mr_client_shared::MuddlePlugin;

fn main() {
    env_logger::init();
    App::build()
        // Window and rendering.
        .add_resource(WindowDescriptor {
            title: "Muddle Run".to_owned(),
            width: 1024,
            height: 768,
            ..Default::default()
        })
        .add_resource(Msaa { samples: 4 })
        .add_plugins(DefaultPlugins)
        .add_plugin(MuddlePlugin)
        .run();
}
