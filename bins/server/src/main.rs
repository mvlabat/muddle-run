use bevy::app::App;
use mr_server_lib::MuddleServerPlugin;

fn main() {
    env_logger::init();
    App::build().add_plugin(MuddleServerPlugin).run();
}
