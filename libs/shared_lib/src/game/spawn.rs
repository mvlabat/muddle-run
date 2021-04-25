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
    player::Player,
    registry::EntityRegistry,
    util::dedup_by_key_unsorted,
    GameTime, SimulationTime, PLAYER_SIZE,
};
use bevy::{log, prelude::*};
use bevy_rapier3d::rapier::{dynamics::RigidBodyBuilder, geometry::ColliderBuilder};
use std::collections::HashMap;

pub fn spawn_players(
    mut commands: Commands,
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
                .insert(time.server_frame, Some(Vec2::ZERO));
            spawned.set_respawned_at(time.server_frame);

            continue;
        }

        log::info!(
            "Spawning a new player (frame {}): {}",
            time.server_frame,
            command.net_id.0
        );
        let mut entity_commands = commands.spawn();
        PlayerClientFactory::insert_components(
            &mut entity_commands,
            &mut pbr_client_params,
            &command.is_player_frame_simulated,
        );
        entity_commands
            .insert(
                RigidBodyBuilder::new_dynamic()
                    .translation(0.0, PLAYER_SIZE / 2.0, 0.0)
                    .lock_rotations(),
            )
            .insert(ColliderBuilder::cuboid(
                PLAYER_SIZE / 2.0,
                PLAYER_SIZE / 2.0,
                PLAYER_SIZE / 2.0,
            ))
            .insert(Position::new(
                command.start_position,
                time.server_frame,
                frames_ahead + 1,
            ))
            .insert(PlayerDirection::new(
                Vec2::ZERO,
                time.server_frame,
                frames_ahead + 1,
            ))
            .insert(Spawned::new(time.server_frame));
        player_entities.register(command.net_id, entity_commands.id());
    }
}

pub fn despawn_players(
    mut commands: Commands,
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
        let mut spawned = match players.get_mut(entity) {
            Ok((_, spawned, _)) => spawned,
            Err(err) => {
                // TODO: investigate.
                log::error!("A despawned entity doesn't exist: {:?}", err);
                continue;
            }
        };
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
        PlayerClientFactory::remove_components(&mut commands.entity(entity));
        spawned.set_despawned_at(command.frame_number);
    }
}

pub fn spawn_level_objects(
    mut commands: Commands,
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
        let mut entity_commands = commands.spawn();
        match command.object.desc {
            LevelObjectDesc::Plane(plane) => PlaneClientFactory::insert_components(
                &mut entity_commands,
                &mut pbr_client_params,
                &(plane, cfg!(feature = "render")),
            ),
        };
        entity_commands.insert(Spawned::new(command.frame_number));
        object_entities.register(command.object.net_id, entity_commands.id());
    }
}

pub fn process_spawned_entities(
    mut commands: Commands,
    game_time: Res<GameTime>,
    mut players: ResMut<HashMap<PlayerNetId, Player>>,
    mut player_entities: ResMut<EntityRegistry<PlayerNetId>>,
    mut object_entities: ResMut<EntityRegistry<EntityNetId>>,
    mut spawned_entities: Query<(Entity, &mut Spawned)>,
) {
    for (entity, mut spawned) in spawned_entities.iter_mut() {
        spawned.mark_if_mature(game_time.frame_number);
        if spawned.can_be_removed(game_time.frame_number) {
            log::debug!("Despawning entity {:?}", entity);
            commands.entity(entity).despawn();
            if let Some(player_net_id) = player_entities.remove_by_entity(entity) {
                players.remove(&player_net_id);
            }
            object_entities.remove_by_entity(entity);
        }
    }
}
