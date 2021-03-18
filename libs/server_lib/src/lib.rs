use crate::{
    net::{
        process_network_events, send_network_updates, startup, NetworkReader, PlayerConnections,
    },
    player_updates::{process_player_input_updates, DeferredUpdates},
};
use bevy::prelude::*;
use mr_shared_lib::{
    framebuffer::FrameNumber,
    game::{
        commands::{GameCommands, SpawnLevelObject},
        level::{LevelObject, LevelObjectDesc},
        level_objects::PlaneDesc,
    },
    messages::{EntityNetId, PlayerInput, PlayerNetId},
    net::ConnectionState,
    registry::IncrementId,
    MuddleSharedPlugin, PLANE_SIZE,
};
use std::collections::HashMap;

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

        builder.add_startup_system(init_level.system());
        builder.add_startup_system(startup.system());

        let input_stage = SystemStage::serial()
            .with_system(process_network_events.system())
            .with_system(process_player_input_updates.system());
        let broadcast_updates_stage =
            SystemStage::parallel().with_system(send_network_updates.system());

        // Game.
        builder.add_plugin(MuddleSharedPlugin::new(
            input_stage,
            broadcast_updates_stage,
        ));

        let resources = builder.resources_mut();
        resources.get_or_insert_with(EntityNetId::default);
        resources.get_or_insert_with(PlayerNetId::default);
        resources.get_or_insert_with(NetworkReader::default);
        resources.get_or_insert_with(PlayerConnections::default);
        resources.get_or_insert_with(HashMap::<u32, ConnectionState>::default);
        resources.get_or_insert_with(DeferredUpdates::<PlayerInput>::default);
    }
}

pub fn init_level(
    mut entity_net_id_counter: ResMut<EntityNetId>,
    mut spawn_level_object_commands: ResMut<GameCommands<SpawnLevelObject>>,
) {
    spawn_level_object_commands.push(SpawnLevelObject {
        frame_number: FrameNumber::new(0),
        object: LevelObject {
            net_id: entity_net_id_counter.increment(),
            desc: LevelObjectDesc::Plane(PlaneDesc { size: PLANE_SIZE }),
        },
    });
}
