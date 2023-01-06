#[cfg(not(feature = "client"))]
use crate::{
    game::commands::DespawnReason,
    messages::{DeferredMessagesQueue, SwitchRole},
    player::PlayerRole,
    server::level_spawn_location_service::LevelSpawnLocationService,
};
use crate::{
    game::{
        commands::{
            DeferredQueue, DespawnLevelObject, DespawnPlayer, RestartGame, SpawnPlayer,
            SwitchPlayerRole, UpdateLevelObject,
        },
        components::{LevelObjectServerGhostParent, LevelObjectStaticGhostParent, PlayerSensor},
    },
    messages::{EntityNetId, PlayerNetId},
    player::{PlayerEvent, PlayerUpdates, Players},
    registry::EntityRegistry,
    util::dedup_by_key_unsorted,
    SimulationTime,
};
use bevy::{
    ecs::{
        entity::Entity,
        query::With,
        system::{Res, ResMut, Resource},
        world::World,
    },
    log,
    prelude::{Deref, DerefMut},
};

pub mod client_factories;
pub mod collisions;
pub mod commands;
pub mod components;
pub mod events;
pub mod level;
pub mod level_objects;
pub mod movement;
pub mod spawn;

#[derive(Resource, Deref, DerefMut)]
pub struct PlayerEventSender(pub Option<tokio::sync::mpsc::UnboundedSender<PlayerEvent>>);

// TODO: track https://github.com/bevyengine/rfcs/pull/16.
pub fn restart_game(world: &mut World) {
    let mut restart_game_commands = world
        .get_resource_mut::<DeferredQueue<RestartGame>>()
        .unwrap();
    if restart_game_commands.drain(&Default::default()).is_empty() {
        return;
    }

    log::info!("Restarting the game");

    let mut players = world.get_resource_mut::<Players>().unwrap();
    players.clear();

    let mut entities_to_despawn = Vec::new();

    let mut player_registry = world
        .get_resource_mut::<EntityRegistry<PlayerNetId>>()
        .unwrap();
    for (net_id, player_entity) in player_registry.iter() {
        log::debug!(
            "Despawning player (entity: {:?}, player_net_id: {})",
            player_entity,
            net_id.0
        );
        entities_to_despawn.push(*player_entity);
    }
    player_registry.clear();

    for player_sensor_entity in world
        .query_filtered::<Entity, With<PlayerSensor>>()
        .iter(world)
    {
        entities_to_despawn.push(player_sensor_entity);
    }

    for (net_id, object_entity) in world
        .get_resource::<EntityRegistry<EntityNetId>>()
        .unwrap()
        .clone()
        .iter()
    {
        log::debug!(
            "Despawning object (entity: {:?}, entity_net_id: {})",
            object_entity,
            net_id.0
        );
        entities_to_despawn.push(*object_entity);
        #[cfg(feature = "client")]
        {
            let handle = world
                .query::<&bevy::asset::Handle<bevy::render::mesh::Mesh>>()
                .get(world, *object_entity)
                .unwrap()
                .clone();
            world
                .get_resource_mut::<bevy::asset::Assets<bevy::render::mesh::Mesh>>()
                .unwrap()
                .remove(handle);
        }
    }
    world
        .get_resource_mut::<EntityRegistry<EntityNetId>>()
        .unwrap()
        .clear();

    for static_ghost_entity in world
        .query_filtered::<Entity, With<LevelObjectStaticGhostParent>>()
        .iter(world)
    {
        entities_to_despawn.push(static_ghost_entity);
    }

    for server_ghost_entity in world
        .query_filtered::<Entity, With<LevelObjectServerGhostParent>>()
        .iter(world)
    {
        entities_to_despawn.push(server_ghost_entity);
    }

    for entity in entities_to_despawn {
        world.despawn(entity);
    }

    *world
        .get_resource_mut::<DeferredQueue<SpawnPlayer>>()
        .unwrap() = Default::default();
    *world
        .get_resource_mut::<DeferredQueue<DespawnPlayer>>()
        .unwrap() = Default::default();
    *world
        .get_resource_mut::<DeferredQueue<UpdateLevelObject>>()
        .unwrap() = Default::default();
    *world
        .get_resource_mut::<DeferredQueue<DespawnLevelObject>>()
        .unwrap() = Default::default();
    *world.get_resource_mut::<PlayerUpdates>().unwrap() = PlayerUpdates::default();
}

pub fn switch_player_role(
    mut switch_role_commands: ResMut<DeferredQueue<SwitchPlayerRole>>,
    mut players: ResMut<Players>,
    time: Res<SimulationTime>,
    #[cfg(not(feature = "client"))] mut despawn_player_commands: ResMut<
        DeferredQueue<DespawnPlayer>,
    >,
    #[cfg(not(feature = "client"))] mut switch_role_messages: ResMut<
        DeferredMessagesQueue<SwitchRole>,
    >,
    #[cfg(not(feature = "client"))] mut spawn_player_commands: ResMut<DeferredQueue<SpawnPlayer>>,
    #[cfg(not(feature = "client"))] level_spawn_location_service: LevelSpawnLocationService,
) {
    #[cfg(feature = "profiler")]
    puffin::profile_function!();
    let mut switch_role_commands = switch_role_commands.drain(&time);
    // We want to keep the last command instead of the first one.
    switch_role_commands.reverse();
    dedup_by_key_unsorted(&mut switch_role_commands, |command| command.net_id);
    switch_role_commands.reverse();

    for switch_role_command in switch_role_commands {
        let player = match players.get_mut(&switch_role_command.net_id) {
            Some(player) => player,
            None => {
                log::warn!(
                    "Can't switch role for player ({}) that doesn't exist",
                    switch_role_command.net_id.0
                );
                continue;
            }
        };

        if player.role == switch_role_command.role {
            log::warn!(
                "Player {} already has role {:?}",
                switch_role_command.net_id.0,
                player.role
            );
            continue;
        }

        player.role = switch_role_command.role;
        log::info!(
            "Switching player ({}) role to {:?}",
            switch_role_command.net_id.0,
            player.role
        );

        // If a player is going to be respawned due to a Finish or Death event, we want
        // to prevent it, as players shouldn't be respawned when in Builder
        // mode.
        player.respawning_at = None;

        #[cfg(not(feature = "client"))]
        {
            match player.role {
                PlayerRole::Runner => {
                    spawn_player_commands.push(SpawnPlayer {
                        net_id: switch_role_command.net_id,
                        start_position: level_spawn_location_service
                            .spawn_position(time.server_frame),
                        is_player_frame_simulated: switch_role_command.is_player_frame_simulated,
                    });
                }
                PlayerRole::Builder => {
                    despawn_player_commands.push(DespawnPlayer {
                        net_id: switch_role_command.net_id,
                        frame_number: switch_role_command.frame_number,
                        reason: DespawnReason::SwitchRole,
                    });
                }
            }

            switch_role_messages.push(SwitchRole {
                net_id: switch_role_command.net_id,
                role: switch_role_command.role,
                frame_number: switch_role_command.frame_number,
            });
        }
    }
}

pub fn remove_disconnected_players(
    player_entities: Res<EntityRegistry<PlayerNetId>>,
    mut players: ResMut<Players>,
    #[cfg(not(feature = "client"))] mut players_tracking_channel: ResMut<PlayerEventSender>,
) {
    #[cfg(feature = "profiler")]
    puffin::profile_function!();
    players.drain_filter(|player_net_id, player| {
        let remove = !player.is_connected && player_entities.get_entity(*player_net_id).is_none();
        if remove {
            log::info!("Player {} is disconnected and removed", player_net_id.0);

            #[cfg(not(feature = "client"))]
            if let Some(players_tracking_channel) = &mut **players_tracking_channel {
                if let Err(err) =
                    players_tracking_channel.send(PlayerEvent::Disconnected(player.uuid.clone()))
                {
                    log::error!("Failed to send PlayerEvent: {:?}", err);
                }
            }
        }
        remove
    });
}
