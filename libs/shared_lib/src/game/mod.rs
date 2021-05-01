use crate::{
    game::commands::{
        DespawnLevelObject, DespawnPlayer, GameCommands, RestartGame, SpawnLevelObject, SpawnPlayer,
    },
    messages::{EntityNetId, PlayerNetId},
    player::{Player, PlayerUpdates},
    registry::EntityRegistry,
};
use bevy::{ecs::world::World, log};
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
        .get_resource_mut::<GameCommands<RestartGame>>()
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

    for entity in entities_to_despawn {
        world.despawn(entity);
    }

    world
        .get_resource_mut::<GameCommands<SpawnPlayer>>()
        .unwrap()
        .drain();
    world
        .get_resource_mut::<GameCommands<DespawnPlayer>>()
        .unwrap()
        .drain();
    world
        .get_resource_mut::<GameCommands<SpawnLevelObject>>()
        .unwrap()
        .drain();
    world
        .get_resource_mut::<GameCommands<DespawnLevelObject>>()
        .unwrap()
        .drain();
    *world.get_resource_mut::<PlayerUpdates>().unwrap() = PlayerUpdates::default();
}
