#![feature(bool_to_option)]
#![feature(hash_drain_filter)]

use crate::{
    net::{process_network_events, send_network_updates, startup, PlayerConnections},
    player_updates::{
        process_despawn_level_object_requests, process_player_input_updates,
        process_spawn_level_object_requests, process_switch_role_requests,
        process_update_level_object_requests,
    },
};
use bevy::{core::FixedTimestep, prelude::*};
use mr_shared_lib::{
    framebuffer::FrameNumber,
    game::{
        commands::{DeferredPlayerQueues, DeferredQueue, DespawnLevelObject, UpdateLevelObject},
        level::{LevelObject, LevelObjectDesc},
        level_objects::PlaneDesc,
    },
    messages::{
        self, DeferredMessagesQueue, EntityNetId, PlayerNetId, RunnerInput, SpawnLevelObject,
        SpawnLevelObjectRequest,
    },
    net::ConnectionState,
    player::PlayerRole,
    registry::IncrementId,
    MuddleSharedPlugin, PLANE_SIZE, SIMULATIONS_PER_SECOND,
};
use std::collections::HashMap;

mod net;
mod player_updates;

pub struct MuddleServerPlugin;

impl Plugin for MuddleServerPlugin {
    fn build(&self, builder: &mut AppBuilder) {
        // The minimal set of Bevy plugins needed for the game logic.
        builder.add_plugin(bevy::log::LogPlugin::default());
        builder.add_plugin(bevy::core::CorePlugin::default());
        builder.add_plugin(bevy::transform::TransformPlugin::default());
        builder.add_plugin(bevy::diagnostic::DiagnosticsPlugin::default());
        builder.add_plugin(bevy::app::ScheduleRunnerPlugin::default());

        builder.add_startup_system(init_level.system());
        builder.add_startup_system(startup.system());

        let input_stage = SystemStage::parallel()
            .with_system(process_network_events.system().label("net"))
            .with_system(process_player_input_updates.system().after("net"))
            .with_system(process_switch_role_requests.system().after("net"))
            // It's ok to run the following in random order since object updates aren't possible
            // on the client before an authoritative confirmation that an object has been spawned.
            .with_system(process_spawn_level_object_requests.system().after("net"))
            .with_system(process_update_level_object_requests.system().after("net"))
            .with_system(process_despawn_level_object_requests.system().after("net"));
        let broadcast_updates_stage =
            SystemStage::parallel().with_system(send_network_updates.system());

        // Game.
        builder.add_plugin(MuddleSharedPlugin::new(
            FixedTimestep::steps_per_second(SIMULATIONS_PER_SECOND as f64),
            input_stage,
            broadcast_updates_stage,
            SystemStage::parallel(),
            None,
        ));

        let resources = builder.world_mut();
        resources.get_resource_or_insert_with(EntityNetId::default);
        resources.get_resource_or_insert_with(PlayerNetId::default);
        resources.get_resource_or_insert_with(PlayerConnections::default);
        resources.get_resource_or_insert_with(Vec::<(PlayerNetId, u32)>::default);
        resources.get_resource_or_insert_with(HashMap::<u32, ConnectionState>::default);
        resources.get_resource_or_insert_with(DeferredPlayerQueues::<RunnerInput>::default);
        resources.get_resource_or_insert_with(DeferredPlayerQueues::<PlayerRole>::default);
        resources.get_resource_or_insert_with(
            DeferredPlayerQueues::<messages::SpawnLevelObjectRequestBody>::default,
        );
        resources
            .get_resource_or_insert_with(DeferredPlayerQueues::<SpawnLevelObjectRequest>::default);
        resources.get_resource_or_insert_with(DeferredPlayerQueues::<LevelObject>::default);
        resources.get_resource_or_insert_with(DeferredPlayerQueues::<EntityNetId>::default);
        resources.get_resource_or_insert_with(DeferredMessagesQueue::<SpawnLevelObject>::default);
        resources.get_resource_or_insert_with(DeferredMessagesQueue::<UpdateLevelObject>::default);
        resources.get_resource_or_insert_with(DeferredMessagesQueue::<DespawnLevelObject>::default);
    }
}

pub fn init_level(
    mut entity_net_id_counter: ResMut<EntityNetId>,
    mut spawn_level_object_commands: ResMut<DeferredQueue<UpdateLevelObject>>,
) {
    spawn_level_object_commands.push(UpdateLevelObject {
        frame_number: FrameNumber::new(0),
        object: LevelObject {
            net_id: entity_net_id_counter.increment(),
            label: "Ground".to_owned(),
            desc: LevelObjectDesc::Plane(PlaneDesc {
                position: Vec2::ZERO,
                size: PLANE_SIZE,
            }),
            route: None,
        },
    });
}
