use std::net::{IpAddr, SocketAddr};

use bevy::{log, prelude::*};

use crate::{
    net::{process_network_events, startup, NetworkReader, PlayerConnections},
    player_updates::{AcknowledgedInputs, DeferredUpdates},
};
use bevy_networking_turbulence::NetworkingPlugin;
use mr_shared_lib::{
    game::spawn::EmptySpawner,
    net::{deserialize, serialize, ClientMessage, PlayerInput},
    MuddleSharedPlugin,
};

mod net;
mod player_updates;

pub struct MuddleServerPlugin;

impl Plugin for MuddleServerPlugin {
    fn build(&self, builder: &mut AppBuilder) {
        // The minimal set of Bevy plugins needed for the game logic.
        builder.add_plugin(bevy::log::LogPlugin::default());
        builder.add_plugin(bevy::reflect::ReflectPlugin::default());
        builder.add_plugin(bevy::core::CorePlugin::default());
        builder.add_plugin(bevy::transform::TransformPlugin::default());
        builder.add_plugin(bevy::diagnostic::DiagnosticsPlugin::default());
        builder.add_plugin(bevy::app::ScheduleRunnerPlugin::default());

        // Networking.
        builder.add_plugin(NetworkingPlugin);

        builder.add_startup_system(startup.system());
        builder.add_system(process_network_events.system());

        // Game.
        builder.add_plugin(MuddleSharedPlugin::<EmptySpawner>::default());

        let resources = builder.resources_mut();
        resources.get_or_insert_with(NetworkReader::default);
        resources.get_or_insert_with(PlayerConnections::default);
        resources.get_or_insert_with(DeferredUpdates::<PlayerInput>::default);
        resources.get_or_insert_with(AcknowledgedInputs::default);
    }
}
