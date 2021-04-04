use crate::{
    game::{
        client_factories::{
            ClientFactory, PbrClientParams, PlaneClientFactory, PlayerClientFactory,
        },
        commands::{DespawnPlayer, GameCommands, SpawnLevelObject, SpawnPlayer},
        components::{PlayerDirection, Position, Spawned},
        level::{LevelObjectDesc, LevelState},
    },
    messages::{EntityNetId, PlayerNetId},
    registry::EntityRegistry,
    util::dedup_by_key_unsorted,
    GameTime, SimulationTime, PLAYER_SIZE,
};
use bevy::{log, prelude::*};
use bevy_rapier3d::rapier::{dynamics::RigidBodyBuilder, geometry::ColliderBuilder};

pub fn spawn_players(
    commands: &mut Commands,
    time: Res<SimulationTime>,
    mut pbr_client_params: PbrClientParams,
    mut spawn_player_commands: ResMut<GameCommands<SpawnPlayer>>,
    mut player_entities: ResMut<EntityRegistry<PlayerNetId>>,
    mut players: Query<(Entity, &mut Spawned, &mut Position, &mut PlayerDirection)>,
) {
    let mut spawn_player_commands = spawn_player_commands.drain();
    dedup_by_key_unsorted(&mut spawn_player_commands, |command| command.net_id);

    for command in spawn_player_commands {
        let frames_ahead = if command.is_player_frame_simulated {
            (time.player_frame - time.server_frame).value()
        } else {
            0
        };

        if let Some(entity) = player_entities.get_entity(command.net_id) {
            // TODO: double-check that we send a respawn command indeed and it's correct.
            log::info!(
                "Respawning player ({}) entity (frame: {}): {:?}",
                command.net_id.0,
                time.server_frame,
                entity
            );

            let (_, mut spawned, mut position, mut player_direction) =
                players.get_mut(entity).unwrap();
            position
                .buffer
                .insert(time.server_frame, command.start_position);
            player_direction
                .buffer
                .insert(time.server_frame, Some(Vec2::zero()));
            spawned.set_respawned_at(time.server_frame);

            continue;
        }

        log::info!(
            "Spawning a new player (frame {}): {}",
            time.server_frame,
            command.net_id.0
        );
        let player_entity = PlayerClientFactory::create(
            commands,
            &mut pbr_client_params,
            &command.is_player_frame_simulated,
        );
        commands
            .with(
                RigidBodyBuilder::new_dynamic()
                    .translation(0.0, PLAYER_SIZE / 2.0, 0.0)
                    .lock_rotations(),
            )
            .with(ColliderBuilder::cuboid(
                PLAYER_SIZE / 2.0,
                PLAYER_SIZE / 2.0,
                PLAYER_SIZE / 2.0,
            ))
            .with(Position::new(
                command.start_position,
                time.server_frame,
                frames_ahead + 1,
            ))
            .with(PlayerDirection::new(
                Vec2::zero(),
                time.server_frame,
                frames_ahead + 1,
            ))
            .with(Spawned::new(time.server_frame));
        player_entities.register(command.net_id, player_entity);
    }
}

pub fn despawn_players(
    commands: &mut Commands,
    mut despawn_player_commands: ResMut<GameCommands<DespawnPlayer>>,
    player_entities: Res<EntityRegistry<PlayerNetId>>,
    mut players: Query<(Entity, &mut Spawned, &PlayerDirection)>,
) {
    for command in despawn_player_commands.drain() {
        let entity = match player_entities.get_entity(command.net_id) {
            Some(entity) => entity,
            None => {
                log::error!(
                    "Player ({}) entity doesn't exist, skipping (frame: {})",
                    command.net_id.0,
                    command.frame_number
                );
                continue;
            }
        };
        let (_, mut spawned, _) = players.get_mut(entity).unwrap();
        if !spawned.is_spawned(command.frame_number) {
            log::debug!(
                "Player ({}) is not spawned at frame {}, skipping the despawn command",
                command.net_id.0,
                command.frame_number
            );
            continue;
        }

        log::info!(
            "Despawning player {} (frame {})",
            command.net_id.0,
            command.frame_number
        );
        PlayerClientFactory::remove_renderables(commands, entity);
        spawned.set_despawned_at(command.frame_number);
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
            LevelObjectDesc::Plane(plane) => PlaneClientFactory::create(
                commands,
                &mut pbr_client_params,
                &(plane, cfg!(feature = "render")),
            ),
        };
        commands.with(Spawned::new(command.frame_number));
        object_entities.register(command.object.net_id, object_entity);
    }
}

pub fn process_spawned_entities(
    commands: &mut Commands,
    game_time: Res<GameTime>,
    mut player_entities: ResMut<EntityRegistry<PlayerNetId>>,
    mut object_entities: ResMut<EntityRegistry<EntityNetId>>,
    mut spawned_entities: Query<(Entity, &mut Spawned)>,
) {
    for (entity, mut spawned) in spawned_entities.iter_mut() {
        spawned.mark_if_mature(game_time.frame_number);
        if spawned.can_be_removed(game_time.frame_number) {
            log::debug!("Despawning entity {:?}", entity);
            commands.despawn(entity);
            player_entities.remove_by_entity(entity);
            object_entities.remove_by_entity(entity);
        }
    }
}

#[cfg(test)]
mod tests {
    use bevy::ecs::{Commands, World};

    use crate::game::client_factories::ClientFactory;

    pub struct TestFactory;

    impl<'a> ClientFactory<'a> for TestFactory {
        type Dependencies = ();
        type Input = ();
    }

    #[test]
    fn test_empty_factory() {
        let world = World::default();
        let mut command_buffer = Commands::default();
        command_buffer.set_entity_reserver(world.get_entity_reserver());

        let entity_a = TestFactory::create(&mut command_buffer, &mut (), &());
        let entity_b = TestFactory::create(&mut command_buffer, &mut (), &());
        assert_ne!(entity_a, entity_b);
    }
}
