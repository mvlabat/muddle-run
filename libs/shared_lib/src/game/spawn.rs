use crate::{
    game::{
        client_factories::{
            ClientFactory, CubeClientFactory, PbrClientParams, PlaneClientFactory,
            PlayerClientFactory, RoutePointClientFactory,
        },
        commands::{
            DeferredQueue, DespawnLevelObject, DespawnPlayer, SpawnPlayer, UpdateLevelObject,
        },
        components::{
            LevelObjectLabel, LevelObjectTag, PlayerDirection, PlayerTag, Position, SpawnCommand,
            Spawned,
        },
        level::{LevelObjectDesc, LevelState},
    },
    messages::{EntityNetId, PlayerNetId},
    registry::EntityRegistry,
    util::dedup_by_key_unsorted,
    GameTime, SimulationTime, PLAYER_SIZE,
};
use bevy::{log, prelude::*};
use bevy_rapier3d::{
    physics::{ColliderBundle, ColliderPositionSync, RigidBodyBundle},
    rapier::{
        dynamics::{RigidBodyMassProps, RigidBodyMassPropsFlags},
        geometry::ColliderShape,
    },
};

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
            let mut entity_commands = commands.entity(entity);
            // TODO: double-check that we send a respawn command indeed and it's correct.
            log::info!(
                "Respawning player ({}) entity (frame: {}): {:?}",
                command.net_id.0,
                time.server_frame,
                entity
            );

            let (_, mut spawned, mut position, mut player_direction) =
                players.get_mut(entity).unwrap();
            for frame_number in time.server_frame..=time.player_frame {
                position
                    .buffer
                    .insert(frame_number, command.start_position);
                player_direction
                    .buffer
                    .insert(frame_number, Some(Vec2::ZERO));
            }
            PlayerClientFactory::insert_components(
                &mut entity_commands,
                &mut pbr_client_params,
                &(command.start_position, command.is_player_frame_simulated),
            );
            spawned.push_command(time.server_frame, SpawnCommand::Spawn);

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
            .insert_bundle(RigidBodyBundle {
                position: [0.0, 0.0, PLAYER_SIZE].into(),
                mass_properties: RigidBodyMassProps {
                    flags: RigidBodyMassPropsFlags::ROTATION_LOCKED,
                    ..RigidBodyMassProps::default()
                },
                ..RigidBodyBundle::default()
            })
            .insert_bundle(ColliderBundle {
                shape: ColliderShape::cuboid(PLAYER_SIZE, PLAYER_SIZE, PLAYER_SIZE),
                ..ColliderBundle::default()
            })
            .insert(Position::new(
                command.start_position,
                time.server_frame,
                frames_ahead + 1,
            ))
            .insert(ColliderPositionSync::Discrete)
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
        spawned.push_command(command.frame_number, SpawnCommand::Despawn);
    }
}

pub fn update_level_objects(
    mut commands: Commands,
    time: Res<SimulationTime>,
    mut pbr_client_params: PbrClientParams,
    mut update_level_object_commands: ResMut<DeferredQueue<UpdateLevelObject>>,
    mut object_entities: ResMut<EntityRegistry<EntityNetId>>,
    mut level_state: ResMut<LevelState>,
    mut level_objects: Query<(Entity, Option<&mut Position>, &mut Spawned, &LevelObjectTag)>,
) {
    // There may be several updates of the same entity per frame. We need to dedup them,
    // otherwise we crash when trying to clone from the entities that haven't been created yet
    // (because of not yet flushed command buffer).
    let mut update_level_object_commands = update_level_object_commands.drain();
    dedup_by_key_unsorted(&mut update_level_object_commands, |command| {
        command.object.net_id
    });

    for command in update_level_object_commands {
        let mut spawned_component = Spawned::new(command.frame_number);
        let mut position_component: Option<Position> = None;
        if let Some(existing_entity) = object_entities.get_entity(command.object.net_id) {
            log::debug!(
                "Replacing an object ({}): {:?}",
                command.object.net_id.0,
                command.object
            );
            object_entities.remove_by_id(command.object.net_id);
            commands.entity(existing_entity).despawn();
            let (_, position, spawned, _) = level_objects
                .get_mut(existing_entity)
                .expect("Expected a registered level object entity to exist");
            if let Some(mut position) = position {
                position_component = Some(position.take());
            }
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
            LevelObjectDesc::RoutePoint(route_point) => RoutePointClientFactory::insert_components(
                &mut entity_commands,
                &mut pbr_client_params,
                route_point,
            ),
        };
        if let Some(position) = command.object.desc.position() {
            let position_component = if let Some(mut position_component) = position_component {
                for frame_number in
                    time.server_frame..=position_component.buffer.end_frame().max(time.server_frame)
                {
                    position_component.buffer.insert(frame_number, position);
                }
                position_component
            } else {
                Position::new(position, time.server_frame, time.player_frames_ahead() + 1)
            };
            entity_commands.insert(position_component);
        }
        let (rigid_body, collider) = command.object.desc.physics_body();
        entity_commands
            .insert(LevelObjectTag)
            .insert(LevelObjectLabel(command.object.label))
            .insert_bundle(rigid_body)
            .insert_bundle(collider)
            .insert(ColliderPositionSync::Discrete)
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
            LevelObjectDesc::RoutePoint(_) => {
                RoutePointClientFactory::remove_components(&mut commands.entity(entity))
            }
        }
        spawned.push_command(command.frame_number, SpawnCommand::Despawn);
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
        spawned.pop_outdated_commands(game_time.frame_number);
        if spawned.can_be_removed(game_time.frame_number) {
            log::debug!("Despawning entity {:?}", entity);
            commands.entity(entity).despawn();
            player_entities.remove_by_entity(entity);
            object_entities.remove_by_entity(entity);
        }
    }
}
