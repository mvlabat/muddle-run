use crate::{
    game::{
        commands::{GameCommands, SpawnLevelObject, SpawnLevelObjectDesc, SpawnPlayer},
        client_factories::{ClientFactory, PbrClientParams, PlaneClientFactory, PlayerClientFactory},
    },
    net::{EntityNetId, PlayerNetId},
    registry::EntityRegistry,
};
use bevy::{prelude::*};

pub fn spawn_player(
    commands: &mut Commands,
    mut pbr_client_params: PbrClientParams,
    mut spawn_player_commands: ResMut<GameCommands<SpawnPlayer>>,
    mut player_entities: ResMut<EntityRegistry<PlayerNetId>>,
) {
    for command in spawn_player_commands.drain() {
        let player_entity = PlayerClientFactory::create(commands, &mut pbr_client_params, &());
        player_entities.register(command.net_id, player_entity);
    }
}

pub fn spawn_level_objects(
    commands: &mut Commands,
    mut pbr_client_params: PbrClientParams,
    mut spawn_level_object_commands: ResMut<GameCommands<SpawnLevelObject>>,
    mut object_entities: ResMut<EntityRegistry<EntityNetId>>,
) {
    for command in spawn_level_object_commands.drain() {
        let object_entity = match command.desc {
            SpawnLevelObjectDesc::Plane(plane) => {
                PlaneClientFactory::create(commands, &mut pbr_client_params, &plane)
            }
        };
        object_entities.register(command.net_id, object_entity);
    }
}

#[cfg(test)]
mod tests {
    use crate::game::spawn::{EmptySpawner, Spawner};
    use bevy::ecs::{Commands, EntityReserver, Resources, World};

    #[test]
    fn test_empty_spawner() {
        let mut world = World::default();
        let mut resources = Resources::default();
        let mut command_buffer = Commands::default();
        command_buffer.set_entity_reserver(world.get_entity_reserver());

        let entity_a = EmptySpawner::spawn(&mut command_buffer, &mut (), &());
        let entity_b = EmptySpawner::spawn(&mut command_buffer, &mut (), &());
        assert_ne!(entity_a, entity_b);
    }
}
