use crate::{
    game::{
        client_factories::{
            ClientFactory, PbrClientParams, PlaneClientFactory, PlayerClientFactory,
        },
        commands::{GameCommands, SpawnLevelObject, SpawnPlayer},
        components::{PlayerDirection, Position},
        level::{LevelObjectDesc, LevelState},
    },
    messages::{EntityNetId, PlayerNetId},
    registry::EntityRegistry,
    GameTime, PLAYER_SIZE,
};
use bevy::{log, prelude::*};
use bevy_rapier3d::rapier::{dynamics::RigidBodyBuilder, geometry::ColliderBuilder};

pub fn spawn_players(
    commands: &mut Commands,
    time: Res<GameTime>,
    mut pbr_client_params: PbrClientParams,
    mut spawn_player_commands: ResMut<GameCommands<SpawnPlayer>>,
    mut player_entities: ResMut<EntityRegistry<PlayerNetId>>,
) {
    for command in spawn_player_commands.drain() {
        if player_entities.get_entity(command.net_id).is_some() {
            log::debug!(
                "Player ({}) entity already exists, skipping",
                command.net_id.0
            );
            continue;
        }

        log::info!(
            "Spawning a new player (frame {}): {}",
            time.game_frame,
            command.net_id.0
        );
        let player_entity = PlayerClientFactory::create(commands, &mut pbr_client_params, &());
        commands
            .with(RigidBodyBuilder::new_dynamic().translation(0.0, PLAYER_SIZE / 2.0, 0.0))
            .with(ColliderBuilder::cuboid(
                PLAYER_SIZE,
                PLAYER_SIZE,
                PLAYER_SIZE,
            ))
            .with(Position::new(command.start_position, time.game_frame))
            .with(PlayerDirection::new(Vec2::zero(), time.game_frame));
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
        if object_entities.get_entity(command.object.net_id).is_some() {
            log::debug!(
                "Object ({}) entity is already registered, skipping",
                command.object.net_id.0
            );
            continue;
        }

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
    use bevy::ecs::{Commands, EntityReserver, Resources, World};

    use crate::game::client_factories::ClientFactory;

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
