use crate::{
    game::{
        client_factories::{
            ClientFactory, PbrClientParams, PlaneClientFactory, PlayerClientFactory,
        },
        commands::{GameCommands, SpawnLevelObject, SpawnPlayer},
        level::{LevelObjectDesc, LevelState},
    },
    net::{EntityNetId, PlayerNetId},
    registry::EntityRegistry,
};
use bevy::{log, prelude::*};

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
    mut level_state: ResMut<LevelState>,
) {
    for command in spawn_level_object_commands.drain() {
        log::info!("Spawning an object: {:?}", command);
        level_state.objects.push(command.object.clone());
        let object_entity = match command.object.desc {
            LevelObjectDesc::Plane(plane) => {
                PlaneClientFactory::create(commands, &mut pbr_client_params, &plane)
            }
        };
        object_entities.register(command.object.net_id, object_entity);
    }
}

#[cfg(test)]
mod tests {
    use crate::game::client_factories::ClientFactory;
    use bevy::ecs::{Commands, EntityReserver, Resources, World};

    pub struct TestFactory;

    impl<'a> ClientFactory<'a> for TestFactory {
        type Dependencies = ();
        type Input = ();
    }

    #[test]
    fn test_empty_factory() {
        let mut world = World::default();
        let mut resources = Resources::default();
        let mut command_buffer = Commands::default();
        command_buffer.set_entity_reserver(world.get_entity_reserver());

        let entity_a = TestFactory::create(&mut command_buffer, &mut (), &());
        let entity_b = TestFactory::create(&mut command_buffer, &mut (), &());
        assert_ne!(entity_a, entity_b);
    }
}
