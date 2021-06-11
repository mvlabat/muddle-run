use crate::{
    game::{
        client_factories::{
            ClientFactory, CubeClientFactory, PbrClientParams, PlaneClientFactory,
            PlayerClientFactory,
        },
        commands::{
            DeferredQueue, DespawnLevelObject, DespawnPlayer, SpawnPlayer, UpdateLevelObject,
        },
        components::{
            LevelObjectLabel, LevelObjectTag, PlayerDirection, PlayerTag, Position, Spawned,
        },
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
    mut commands: Commands,
    time: Res<SimulationTime>,
    mut pbr_client_params: PbrClientParams,
    mut spawn_player_commands: ResMut<DeferredQueue<SpawnPlayer>>,
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

        let mut entity_commands = commands.spawn();
        PlayerClientFactory::insert_components(
            &mut entity_commands,
            &mut pbr_client_params,
            &(command.start_position, command.is_player_frame_simulated),
        );
        entity_commands
            .insert(PlayerTag)
            .insert(
                RigidBodyBuilder::new_dynamic()
                    .translation(0.0, PLAYER_SIZE, 0.0)
                    .lock_rotations(),
            )
            .insert(ColliderBuilder::cuboid(
                PLAYER_SIZE,
                PLAYER_SIZE,
                PLAYER_SIZE,
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
        log::info!(
            "Spawning a new player (entity: {:?}, frame {}): {}",
            entity_commands.id(),
            time.server_frame,
            command.net_id.0
        );
        player_entities.register(command.net_id, entity_commands.id());
    }
}

pub fn despawn_players(
    mut commands: Commands,
    mut despawn_player_commands: ResMut<DeferredQueue<DespawnPlayer>>,
    player_entities: Res<EntityRegistry<PlayerNetId>>,
    mut players: Query<(Entity, &mut Spawned, &PlayerTag)>,
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
                log::error!("A despawned player entity doesn't exist: {:?}", err);
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

pub fn update_level_objects(
    mut commands: Commands,
    time: Res<SimulationTime>,
    mut pbr_client_params: PbrClientParams,
    mut update_level_object_commands: ResMut<DeferredQueue<UpdateLevelObject>>,
    mut object_entities: ResMut<EntityRegistry<EntityNetId>>,
    mut level_state: ResMut<LevelState>,
    mut level_objects: Query<(Entity, &mut Spawned, &LevelObjectTag)>,
) {
    for command in update_level_object_commands.drain() {
        let mut spawned_component = Spawned::new(command.frame_number);
        if let Some(existing_entity) = object_entities.get_entity(command.object.net_id) {
            log::trace!(
                "Replacing an object ({}): {:?}",
                command.object.net_id.0,
                command.object
            );
            object_entities.remove_by_id(command.object.net_id);
            commands.entity(existing_entity).despawn();
            let (_, spawned, _) = level_objects
                .get_mut(existing_entity)
                .expect("Expected a registered level object entity to exist");
            spawned_component = spawned.clone();
        }

        log::info!("Spawning an object: {:?}", command);
        level_state
            .objects
            .insert(command.object.net_id, command.object.clone());
        let mut entity_commands = commands.spawn();
        match &command.object.desc {
            LevelObjectDesc::Plane(plane) => PlaneClientFactory::insert_components(
                &mut entity_commands,
                &mut pbr_client_params,
                plane,
            ),
            LevelObjectDesc::Cube(cube) => CubeClientFactory::insert_components(
                &mut entity_commands,
                &mut pbr_client_params,
                cube,
            ),
        };
        let (rigid_body, collider) = command.object.desc.physics_body();
        entity_commands
            .insert(LevelObjectTag)
            .insert(LevelObjectLabel(format!(
                "{} {}",
                command.object.desc.label(),
                command.object.net_id.0
            )))
            .insert(rigid_body)
            .insert(collider)
            .insert(Position::new(
                command.object.desc.position(),
                time.server_frame,
                time.player_frames_ahead() + 1,
            ))
            .insert(spawned_component);
        object_entities.register(command.object.net_id, entity_commands.id());
    }
}

pub fn despawn_level_objects(
    mut commands: Commands,
    mut despawn_level_object_commands: ResMut<DeferredQueue<DespawnLevelObject>>,
    object_entities: Res<EntityRegistry<EntityNetId>>,
    mut level_state: ResMut<LevelState>,
    mut level_objects: Query<(Entity, &mut Spawned, &LevelObjectTag)>,
) {
    for command in despawn_level_object_commands.drain() {
        let entity = match object_entities.get_entity(command.net_id) {
            Some(entity) => entity,
            None => {
                log::error!(
                    "Level object ({}) entity doesn't exist, skipping (frame: {})",
                    command.net_id.0,
                    command.frame_number
                );
                continue;
            }
        };
        let mut spawned = match level_objects.get_mut(entity) {
            Ok((_, spawned, _)) => spawned,
            Err(err) => {
                log::error!("A despawned level object entity doesn't exist: {:?}", err);
                continue;
            }
        };
        if !spawned.is_spawned(command.frame_number) {
            log::debug!(
                "Level object ({}) is not spawned at frame {}, skipping the despawn command",
                command.net_id.0,
                command.frame_number
            );
            continue;
        }

        log::info!(
            "Despawning level object {} (frame {})",
            command.net_id.0,
            command.frame_number
        );
        match level_state
            .objects
            .remove(&command.net_id)
            .expect("Expected a removed level object to exist in the level state")
            .desc
        {
            LevelObjectDesc::Plane(_) => {
                PlaneClientFactory::remove_components(&mut commands.entity(entity))
            }
            LevelObjectDesc::Cube(_) => {
                CubeClientFactory::remove_components(&mut commands.entity(entity))
            }
        }
        spawned.set_despawned_at(command.frame_number);
    }
}

pub fn process_spawned_entities(
    mut commands: Commands,
    game_time: Res<GameTime>,
    mut player_entities: ResMut<EntityRegistry<PlayerNetId>>,
    mut object_entities: ResMut<EntityRegistry<EntityNetId>>,
    mut spawned_entities: Query<(Entity, &mut Spawned)>,
) {
    for (entity, mut spawned) in spawned_entities.iter_mut() {
        spawned.mark_if_mature(game_time.frame_number);
        if spawned.can_be_removed(game_time.frame_number) {
            log::debug!("Despawning entity {:?}", entity);
            commands.entity(entity).despawn();
            player_entities.remove_by_entity(entity);
            object_entities.remove_by_entity(entity);
        }
    }
}
