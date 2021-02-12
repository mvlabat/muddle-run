use crate::{
    net::{
        process_network_events, send_network_updates, startup, NetworkReader, PlayerConnections,
    },
    player_updates::{AcknowledgedInputs, DeferredUpdates},
};
use bevy::prelude::*;
use bevy_networking_turbulence::NetworkingPlugin;
use mr_shared_lib::{
    game::{
        commands::{GameCommands, SpawnLevelObject},
        level::{LevelObject, LevelObjectDesc},
        level_objects::PlaneDesc,
    },
    net::{EntityNetId, PlayerInput, PlayerNetId},
    registry::IncrementId,
    MuddleSharedPlugin, PLANE_SIZE,
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

        builder.add_startup_system(init_level.system());
        builder.add_startup_system(startup.system());
        builder.add_system(process_network_events.system());
        builder.add_system(send_network_updates.system());

        // Game.
        builder.add_plugin(MuddleSharedPlugin::default());

        let resources = builder.resources_mut();
        resources.get_or_insert_with(EntityNetId::default);
        resources.get_or_insert_with(PlayerNetId::default);
        resources.get_or_insert_with(NetworkReader::default);
        resources.get_or_insert_with(PlayerConnections::default);
        resources.get_or_insert_with(DeferredUpdates::<PlayerInput>::default);
        resources.get_or_insert_with(AcknowledgedInputs::default);
    }
}

pub fn init_level(
    mut entity_net_id_counter: ResMut<EntityNetId>,
    mut spawn_level_object_commands: ResMut<GameCommands<SpawnLevelObject>>,
) {
    spawn_level_object_commands.push(SpawnLevelObject {
        object: LevelObject {
            net_id: entity_net_id_counter.increment(),
            desc: LevelObjectDesc::Plane(PlaneDesc { size: PLANE_SIZE }),
        },
    });
}
