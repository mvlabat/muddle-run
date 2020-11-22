use bevy::prelude::*;
use mr_client_shared::MuddlePlugin;

fn main() {
    env_logger::init();
    App::build()
        .add_plugins(DefaultPlugins)
        .add_plugin(MuddlePlugin)
        .run();
}
