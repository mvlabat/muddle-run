use crate::{
    game::{
        commands::{
            DeferredQueue, DespawnLevelObject, DespawnPlayer, RestartGame, SpawnPlayer,
            SwitchPlayerRole, UpdateLevelObject,
        },
        components::LevelObjectStaticGhost,
    },
    messages::{DeferredMessagesQueue, EntityNetId, PlayerNetId, SwitchRole},
    player::{Player, PlayerRole, PlayerUpdates},
    registry::EntityRegistry,
    util::dedup_by_key_unsorted,
};
use bevy::{
    ecs::{
        entity::Entity,
        query::With,
        system::{Res, ResMut},
        world::World,
    },
    log,
};
use std::collections::HashMap;

pub mod client_factories;
pub mod commands;
pub mod components;
pub mod level;
pub mod level_objects;
pub mod movement;
pub mod spawn;

// TODO: track https://github.com/bevyengine/rfcs/pull/16.
pub fn restart_game(world: &mut World) {
    let mut restart_game_commands = world
        .get_resource_mut::<DeferredQueue<RestartGame>>()
        .unwrap();
    if restart_game_commands.drain().is_empty() {
        return;
    }

    log::info!("Restarting the game");

    let mut players = world
        .get_resource_mut::<HashMap<PlayerNetId, Player>>()
        .unwrap();
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

    let mut objects_registry = world
        .get_resource_mut::<EntityRegistry<EntityNetId>>()
        .unwrap();
    for (net_id, object_entity) in objects_registry.iter() {
        log::debug!(
            "Despawning object (entity: {:?}, entity_net_id: {})",
            object_entity,
            net_id.0
        );
        entities_to_despawn.push(*object_entity);
    }
    objects_registry.clear();

    for ghost_entity in world
        .query_filtered::<Entity, With<LevelObjectStaticGhost>>()
        .iter(world)
    {
        entities_to_despawn.push(ghost_entity);
    }

    for entity in entities_to_despawn {
        world.despawn(entity);
    }

    world
        .get_resource_mut::<DeferredQueue<SpawnPlayer>>()
        .unwrap()
        .drain();
    world
        .get_resource_mut::<DeferredQueue<DespawnPlayer>>()
        .unwrap()
        .drain();
    world
        .get_resource_mut::<DeferredQueue<UpdateLevelObject>>()
        .unwrap()
        .drain();
    world
        .get_resource_mut::<DeferredQueue<DespawnLevelObject>>()
        .unwrap()
        .drain();
    *world.get_resource_mut::<PlayerUpdates>().unwrap() = PlayerUpdates::default();
}

pub fn switch_player_role(
    mut switch_role_commands: ResMut<DeferredQueue<SwitchPlayerRole>>,
    mut players: ResMut<HashMap<PlayerNetId, Player>>,
    mut spawn_player_commands: ResMut<DeferredQueue<SpawnPlayer>>,
    mut despawn_player_commands: ResMut<DeferredQueue<DespawnPlayer>>,
    mut switch_role_messages: ResMut<DeferredMessagesQueue<SwitchRole>>,
) {
    let mut switch_role_commands = switch_role_commands.drain();
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
        // This will likely make a duplicate command, as we might as well spawn the player while
        // processing delta updates, but commands get de-duped anyway.
        match player.role {
            PlayerRole::Runner => {
                spawn_player_commands.push(SpawnPlayer {
                    net_id: switch_role_command.net_id,
                    start_position: Default::default(),
                    is_player_frame_simulated: switch_role_command.is_player_frame_simulated,
                });
            }
            PlayerRole::Builder => {
                despawn_player_commands.push(DespawnPlayer {
                    net_id: switch_role_command.net_id,
                    frame_number: switch_role_command.frame_number,
                });
            }
        }

        if cfg!(not(feature = "client")) {
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
    mut players: ResMut<HashMap<PlayerNetId, Player>>,
) {
    players.drain_filter(|player_net_id, player| {
        let remove = !player.is_connected && player_entities.get_entity(*player_net_id).is_none();
        if remove {
            log::info!("Player {} is disconnected and removed", player_net_id.0);
        }
        remove
    });
}
