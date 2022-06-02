use crate::{
    collider_flags::{player_collision_groups, player_sensor_collision_groups},
    framebuffer::FrameNumber,
    game::{
        client_factories::{
            ClientFactory, CubeClientFactory, LevelObjectInput, PbrClientParams,
            PlaneClientFactory, PlayerClientFactory, PlayerSensorClientFactory,
            RoutePointClientFactory,
        },
        commands::{
            DeferredQueue, DespawnLevelObject, DespawnPlayer, SpawnPlayer, UpdateLevelObject,
        },
        components::{
            LevelObjectLabel, LevelObjectServerGhostChild, LevelObjectServerGhostParent,
            LevelObjectStaticGhostChild, LevelObjectStaticGhostParent, LevelObjectTag, LockPhysics,
            PhysicsBundle, PlayerDirection, PlayerFrameSimulated, PlayerSensor, PlayerSensorState,
            PlayerSensors, PlayerTag, Position, SpawnCommand, Spawned,
        },
        level::{ColliderShapeResponse, LevelObject, LevelObjectDesc, LevelState},
    },
    messages::{EntityNetId, PlayerNetId},
    registry::EntityRegistry,
    util::{dedup_by_key_unsorted, player_sensor_outline},
    GameTime, SimulationTime, PLAYER_RADIUS, PLAYER_SENSOR_RADIUS,
};
use bevy::{
    ecs::{
        query::WorldQuery,
        system::{EntityCommands, SystemParam},
    },
    log,
    prelude::*,
    tasks::AsyncComputeTaskPool,
};
use bevy_rapier2d::{
    dynamics::{LockedAxes, RigidBody, Velocity},
    geometry::{ActiveEvents, Collider, Sensor},
    prelude::CollisionGroups,
    rapier::geometry::ColliderShape,
};
use std::fmt::Debug;

#[derive(WorldQuery)]
#[world_query(mutable)]
pub struct SpawnedQuery<'w, T: WorldQuery> {
    pub spawned: &'w Spawned,
    pub player_frame_simulated: Option<&'w PlayerFrameSimulated>,
    pub item: T,
}

pub fn iter_spawned<'w, 's, T: WorldQuery>(
    query: impl Iterator<Item = SpawnedQueryItem<'w, T>> + 'w,
    time: &'w SimulationTime,
) -> impl Iterator<Item = SpawnedQueryItem<'w, T>> + 'w {
    query.filter(
        |SpawnedQueryItem {
             spawned,
             player_frame_simulated,
             ..
         }| spawned.is_spawned(time.entity_simulation_frame(*player_frame_simulated)),
    )
}

pub fn iter_spawned_read_only<'w, 's, T: WorldQuery>(
    query: impl Iterator<Item = SpawnedQueryReadOnlyItem<'w, T>> + 'w,
    time: &'w SimulationTime,
) -> impl Iterator<Item = SpawnedQueryReadOnlyItem<'w, T>> + 'w {
    query.filter(
        |SpawnedQueryReadOnlyItem {
             spawned,
             player_frame_simulated,
             ..
         }| spawned.is_spawned(time.entity_simulation_frame(*player_frame_simulated)),
    )
}

pub type ColliderShapePromiseResult = (Entity, Option<ColliderShape>);
pub type ColliderShapeSender = crossbeam_channel::Sender<ColliderShapePromiseResult>;
pub type ColliderShapeReceiver = crossbeam_channel::Receiver<ColliderShapePromiseResult>;

#[derive(WorldQuery)]
#[world_query(mutable, derive(Debug))]
pub struct PlayerQuery<'w> {
    entity: Entity,
    position: &'w mut Position,
    player_direction: &'w mut PlayerDirection,
    collision_groups: &'w mut CollisionGroups,
    sensors: &'w PlayerSensors,
    spawned: &'w mut Spawned,
    _tag: Without<PlayerSensor>,
}

pub fn spawn_players(
    mut commands: Commands,
    time: Res<SimulationTime>,
    mut pbr_client_params: PbrClientParams,
    mut spawn_player_commands: ResMut<DeferredQueue<SpawnPlayer>>,
    mut player_entities: ResMut<EntityRegistry<PlayerNetId>>,
    mut players: Query<PlayerQuery>,
    mut player_sensors: Query<&mut CollisionGroups, With<PlayerSensor>>,
) {
    #[cfg(feature = "profiler")]
    puffin::profile_function!();
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

            let mut player = players.get_mut(entity).unwrap();
            log::debug!(
                "Filling the player's position and direction buffer from {} to {}",
                time.server_frame,
                time.player_frame
            );
            for frame_number in time.server_frame..=time.player_frame {
                player
                    .position
                    .buffer
                    .insert(frame_number, command.start_position);
                player
                    .player_direction
                    .buffer
                    .insert(frame_number, Some(Vec2::ZERO));
            }
            PlayerClientFactory::insert_components(
                &mut entity_commands,
                &mut pbr_client_params,
                command.start_position,
            );
            *player.collision_groups = player_collision_groups(!command.is_player_frame_simulated);
            for ((player_sensor_entity, _), sensor_position) in
                player.sensors.sensors.iter().zip(player_sensor_outline())
            {
                let mut collision_groups = player_sensors.get_mut(*player_sensor_entity).unwrap();
                *collision_groups =
                    player_sensor_collision_groups(!command.is_player_frame_simulated);
                let mut sensor_commands = entity_commands.commands().entity(*player_sensor_entity);
                PlayerSensorClientFactory::insert_components(
                    &mut sensor_commands,
                    &mut pbr_client_params,
                    (),
                );
                sensor_commands.insert(Transform::from_translation(sensor_position.extend(0.0)));
            }
            player
                .spawned
                .push_command(time.server_frame, SpawnCommand::Spawn);

            continue;
        }

        let mut entity_commands = commands.spawn();
        let player_entity = entity_commands.id();

        let mut sensors = Vec::new();
        entity_commands.with_children(|parent| {
            for sensor_position in player_sensor_outline() {
                let mut sensor_commands = parent.spawn();
                PlayerSensorClientFactory::insert_components(
                    &mut sensor_commands,
                    &mut pbr_client_params,
                    (),
                );
                sensor_commands
                    .insert(Collider::ball(PLAYER_SENSOR_RADIUS))
                    .insert(Sensor(true))
                    .insert(player_sensor_collision_groups(
                        !command.is_player_frame_simulated,
                    ))
                    .insert(ActiveEvents::COLLISION_EVENTS)
                    .insert(Transform::from_translation(sensor_position.extend(0.0)))
                    .insert(GlobalTransform::identity())
                    .insert(PlayerSensor(player_entity));
                #[cfg(feature = "client")]
                if command.is_player_frame_simulated {
                    sensor_commands.insert(PlayerFrameSimulated);
                } else {
                    sensor_commands.insert(LockPhysics(false));
                }
                sensors.push((sensor_commands.id(), PlayerSensorState::default()));
            }
        });

        entity_commands
            .insert(PlayerTag)
            .insert_bundle(PhysicsBundle {
                rigid_body: RigidBody::Dynamic,
                collider: Collider::ball(PLAYER_RADIUS),
                collision_groups: player_collision_groups(!command.is_player_frame_simulated),
                sensor: Sensor(false),
                locked_axes: LockedAxes::ROTATION_LOCKED,
            })
            .insert(ActiveEvents::COLLISION_EVENTS)
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
            .insert(Transform::from_translation(
                command.start_position.extend(0.0),
            ))
            .insert(GlobalTransform::identity())
            .insert(Velocity::zero())
            .insert(Spawned::new(time.server_frame));

        // Insert client components later, as they can overwrite some of them (z
        // coordinates of translations for instance).
        PlayerClientFactory::insert_components(
            &mut entity_commands,
            &mut pbr_client_params,
            command.start_position,
        );

        #[cfg(feature = "client")]
        if command.is_player_frame_simulated {
            log::debug!(
                "Tagging player ({}) entity as PlayerFrameSimulated",
                command.net_id.0
            );
            entity_commands.insert(PlayerFrameSimulated);
        } else {
            entity_commands.insert(LockPhysics(false));
        }
        entity_commands.insert(PlayerSensors {
            main: PlayerSensorState::default(),
            sensors,
        });
        log::info!(
            "Spawning a new player (entity: {:?}, frame {}): {}",
            player_entity,
            time.server_frame,
            command.net_id.0
        );
        player_entities.register(command.net_id, entity_commands.id());
    }
}

pub fn despawn_players(
    mut commands: Commands,
    mut pbr_client_params: PbrClientParams,
    mut despawn_player_commands: ResMut<DeferredQueue<DespawnPlayer>>,
    player_entities: Res<EntityRegistry<PlayerNetId>>,
    mut players: Query<
        (
            Entity,
            &mut Spawned,
            &mut CollisionGroups,
            &mut PlayerSensors,
        ),
        Without<PlayerSensor>,
    >,
    mut player_sensors: Query<&mut CollisionGroups, With<PlayerSensor>>,
) {
    #[cfg(feature = "profiler")]
    puffin::profile_function!();
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
        let (mut spawned, mut collider_flags, mut sensors) = match players.get_mut(entity) {
            Ok((_, spawned, collider_flags, sensors)) => (spawned, collider_flags, sensors),
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
        let mut entity_commands = commands.entity(entity);
        collider_flags.memberships = 0;
        PlayerClientFactory::remove_components(&mut entity_commands, &mut pbr_client_params);
        for (player_sensor_entity, _state) in &mut sensors.sensors {
            let mut collider_flags = player_sensors.get_mut(*player_sensor_entity).unwrap();
            collider_flags.memberships = 0;
            PlayerSensorClientFactory::remove_components(
                &mut commands.entity(*player_sensor_entity),
                &mut pbr_client_params,
            );
        }
        spawned.push_command(command.frame_number, SpawnCommand::Despawn);
    }
}

#[derive(WorldQuery)]
#[world_query(mutable)]
pub struct UpdateLevelObjectQuery<'w> {
    entity: Entity,
    position: Option<&'w mut Position>,
    spawned: &'w mut Spawned,
    static_ghost_entity: Option<&'w LevelObjectStaticGhostChild>,
    server_ghost_entity: Option<&'w LevelObjectServerGhostChild>,
}

#[derive(SystemParam)]
pub struct LevelObjectsParams<'w, 's> {
    object_entities: ResMut<'w, EntityRegistry<EntityNetId>>,
    level_state: ResMut<'w, LevelState>,
    level_object_query: Query<'w, 's, UpdateLevelObjectQuery<'static>>,
}

pub fn update_level_objects(
    mut commands: Commands,
    time: Res<SimulationTime>,
    mut pbr_client_params: PbrClientParams,
    mut update_level_object_commands: ResMut<DeferredQueue<UpdateLevelObject>>,
    mut level_object_params: LevelObjectsParams,
    task_pool: Res<AsyncComputeTaskPool>,
    shape_sender: Res<ColliderShapeSender>,
) {
    #[cfg(feature = "profiler")]
    puffin::profile_function!();
    // There may be several updates of the same entity per frame. We need to dedup
    // them, otherwise we crash when trying to clone from the entities that
    // haven't been created yet (because of not yet flushed command buffer).
    let mut update_level_object_commands = update_level_object_commands.drain();
    dedup_by_key_unsorted(&mut update_level_object_commands, |command| {
        command.object.net_id
    });

    for command in update_level_object_commands {
        let mut spawned_component = Spawned::new(command.frame_number);
        let mut position_component: Option<Position> = None;

        // Unlike with players, here we just despawn level objects if they are updated
        // and re-create from scratch.
        if let Some(existing_entity) = level_object_params
            .object_entities
            .get_entity(command.object.net_id)
        {
            log::debug!(
                "Replacing an object ({}): {:?}",
                command.object.net_id.0,
                command.object
            );
            level_object_params
                .object_entities
                .remove_by_id(command.object.net_id);
            commands.entity(existing_entity).despawn();
            let updated_level_object = level_object_params
                .level_object_query
                .get_mut(existing_entity)
                .expect("Expected a registered level object entity to exist");
            if let Some(LevelObjectStaticGhostChild(static_ghost_entity)) =
                &updated_level_object.static_ghost_entity
            {
                commands.entity(*static_ghost_entity).despawn();
            }
            if let Some(LevelObjectServerGhostChild(server_ghost_entity)) =
                &updated_level_object.server_ghost_entity
            {
                commands.entity(*server_ghost_entity).despawn();
            }
            if let Some(mut position) = updated_level_object.position {
                position_component = Some(position.take());
            }
            spawned_component = updated_level_object.spawned.clone();
        }

        log::info!("Spawning an object: {:?}", command);
        level_object_params
            .level_state
            .objects
            .insert(command.object.net_id, command.object.clone());
        let mut entity_commands = commands.spawn();
        let shape = match command.object.desc.calculate_collider_shape(
            &task_pool,
            entity_commands.id(),
            shape_sender.clone(),
        ) {
            ColliderShapeResponse::Immediate(shape) => Some(shape),
            ColliderShapeResponse::Promise => None,
        };

        if let Some(position) = command.object.desc.position() {
            let position_component = if let Some(mut position_component) = position_component {
                for frame_number in command.frame_number
                    ..=position_component
                        .buffer
                        .end_frame()
                        .max(command.frame_number + FrameNumber::new(time.player_frames_ahead()))
                {
                    position_component.buffer.insert(frame_number, position);
                }
                position_component
            } else {
                Position::new(
                    position,
                    command.frame_number,
                    time.player_frames_ahead() + 1,
                )
            };
            entity_commands.insert(position_component);
        }

        let transform = Transform::from_translation(
            command
                .object
                .desc
                .position()
                .unwrap_or_default()
                .extend(0.0),
        );
        entity_commands
            .insert(command.object.net_id)
            .insert(LevelObjectTag)
            .insert(LevelObjectLabel(command.object.label.clone()))
            .insert(GlobalTransform::identity())
            .insert(transform)
            .insert(GlobalTransform::identity())
            .insert(spawned_component);

        if let Some(ref shape) = shape {
            let physics_bundle = command
                .object
                .desc
                .physics_bundle(shape.clone(), cfg!(not(feature = "client")));
            // Insert client components later, as they can overwrite some of them (z
            // coordinates of translations for instance).
            insert_client_components(
                &mut entity_commands,
                &command.object,
                false,
                &physics_bundle.collider.raw,
                &mut pbr_client_params,
            );
            entity_commands.insert_bundle(physics_bundle);
        }

        let level_object_entity = entity_commands.id();
        level_object_params
            .object_entities
            .register(command.object.net_id, level_object_entity);

        if cfg!(feature = "client") {
            // Spawning the ghost objects.
            let static_ghost = entity_commands.commands().spawn();
            let static_ghost_entity = static_ghost.id();
            let server_ghost = entity_commands.commands().spawn();
            let server_ghost_entity = server_ghost.id();

            entity_commands
                .insert(PlayerFrameSimulated)
                .insert(LevelObjectStaticGhostChild(static_ghost_entity))
                .insert(LevelObjectServerGhostChild(server_ghost_entity));

            let mut static_ghost_commands = commands.entity(static_ghost_entity);
            static_ghost_commands
                // Even if we don't necessarily render an object, we still want nested transforms
                // to work.
                .insert(transform)
                .insert(GlobalTransform::identity())
                .insert(LevelObjectStaticGhostParent(level_object_entity));
            if let Some(ref shape) = shape {
                insert_client_components(
                    &mut static_ghost_commands,
                    &command.object,
                    true,
                    shape,
                    &mut pbr_client_params,
                );
            }

            let mut server_ghost_commands = commands.entity(server_ghost_entity);
            if let Some(shape) = shape {
                let physics_bundle = command.object.desc.physics_bundle(shape, true);
                server_ghost_commands
                    .insert(LockPhysics(false))
                    .insert_bundle(physics_bundle)
                    .insert(LevelObjectServerGhostParent(level_object_entity))
                    .insert(transform)
                    .insert(GlobalTransform::identity());
            }
        }
    }
}

type GhostEntites = Option<(
    &'static LevelObjectStaticGhostChild,
    &'static LevelObjectServerGhostChild,
)>;

pub fn poll_calculating_shapes(
    mut commands: Commands,
    time: Res<GameTime>,
    level_state: Res<LevelState>,
    mut pbr_client_params: PbrClientParams,
    level_objects_query: Query<(&EntityNetId, &Spawned, GhostEntites)>,
    collider_shape_receiver: Res<ColliderShapeReceiver>,
) {
    while let Ok((entity, shape_result)) = collider_shape_receiver.try_recv() {
        let (entity_net_id, spawned, ghost_entities) = match level_objects_query.get(entity) {
            Ok(r) => r,
            Err(_) => continue,
        };

        if !spawned.is_spawned(time.frame_number) {
            continue;
        }

        let level_object = level_state.objects.get(entity_net_id).unwrap();

        let mut entity_commands = commands.entity(entity);

        let shape = match shape_result {
            Some(shape) => {
                log::debug!(
                    "Calculating shape for {:?} has finished (frame: {})",
                    entity,
                    time.frame_number
                );
                shape
            }
            None => {
                log::error!(
                    "Calculating shape for {:?} has failed (frame: {})",
                    entity,
                    time.frame_number
                );
                continue;
            }
        };

        let physics_bundle = level_object
            .desc
            .physics_bundle(shape.clone(), cfg!(not(feature = "client")));
        insert_client_components(
            &mut entity_commands,
            level_object,
            false,
            &physics_bundle.collider.raw,
            &mut pbr_client_params,
        );
        entity_commands.insert_bundle(physics_bundle);

        if let Some((
            LevelObjectStaticGhostChild(static_ghost_entity),
            LevelObjectServerGhostChild(server_ghost_entity),
        )) = ghost_entities
        {
            let mut static_ghost_commands = commands.entity(*static_ghost_entity);
            insert_client_components(
                &mut static_ghost_commands,
                level_object,
                true,
                &shape,
                &mut pbr_client_params,
            );

            let mut server_ghost_commands = commands.entity(*server_ghost_entity);
            let physics_bundle = level_object.desc.physics_bundle(shape, true);
            server_ghost_commands.insert_bundle(physics_bundle);
        }
    }
}

fn insert_client_components(
    entity_commands: &mut EntityCommands,
    level_object: &LevelObject,
    is_ghost: bool,
    collider_shape: &ColliderShape,
    pbr_client_params: &mut PbrClientParams,
) {
    match &level_object.desc {
        LevelObjectDesc::Plane(plane) => PlaneClientFactory::insert_components(
            entity_commands,
            pbr_client_params,
            (
                LevelObjectInput {
                    desc: plane.clone(),
                    collision_logic: level_object.collision_logic,
                    is_ghost,
                },
                Some(collider_shape.clone()),
            ),
        ),
        LevelObjectDesc::Cube(cube) => CubeClientFactory::insert_components(
            entity_commands,
            pbr_client_params,
            LevelObjectInput {
                desc: cube.clone(),
                collision_logic: level_object.collision_logic,
                is_ghost,
            },
        ),
        LevelObjectDesc::RoutePoint(route_point) => RoutePointClientFactory::insert_components(
            entity_commands,
            pbr_client_params,
            LevelObjectInput {
                desc: route_point.clone(),
                collision_logic: level_object.collision_logic,
                is_ghost,
            },
        ),
    };
}

pub fn despawn_level_objects(
    mut commands: Commands,
    mut pbr_client_params: PbrClientParams,
    mut despawn_level_object_commands: ResMut<DeferredQueue<DespawnLevelObject>>,
    object_entities: Res<EntityRegistry<EntityNetId>>,
    mut level_state: ResMut<LevelState>,
    mut level_objects: Query<
        (
            &mut Spawned,
            &mut CollisionGroups,
            Option<&LevelObjectStaticGhostChild>,
        ),
        With<LevelObjectTag>,
    >,
) {
    #[cfg(feature = "profiler")]
    puffin::profile_function!();
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
        let (mut spawned, mut collider_flags, ghost_parent) = match level_objects.get_mut(entity) {
            Ok(r) => r,
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
            "Despawning level object {} (entity: {:?}, frame {})",
            command.net_id.0,
            entity,
            command.frame_number
        );
        collider_flags.memberships = 0;
        match level_state
            .objects
            .remove(&command.net_id)
            .expect("Expected a removed level object to exist in the level state")
            .desc
        {
            LevelObjectDesc::Plane(_) => {
                PlaneClientFactory::remove_components(
                    &mut commands.entity(entity),
                    &mut pbr_client_params,
                );
                if let Some(LevelObjectStaticGhostChild(ghost_entity)) = ghost_parent {
                    PlaneClientFactory::remove_components(
                        &mut commands.entity(*ghost_entity),
                        &mut pbr_client_params,
                    );
                }
            }
            LevelObjectDesc::Cube(_) => {
                CubeClientFactory::remove_components(
                    &mut commands.entity(entity),
                    &mut pbr_client_params,
                );
                if let Some(LevelObjectStaticGhostChild(ghost_entity)) = ghost_parent {
                    CubeClientFactory::remove_components(
                        &mut commands.entity(*ghost_entity),
                        &mut pbr_client_params,
                    );
                }
            }
            LevelObjectDesc::RoutePoint(_) => {
                RoutePointClientFactory::remove_components(
                    &mut commands.entity(entity),
                    &mut pbr_client_params,
                );
                if let Some(LevelObjectStaticGhostChild(ghost_entity)) = ghost_parent {
                    RoutePointClientFactory::remove_components(
                        &mut commands.entity(*ghost_entity),
                        &mut pbr_client_params,
                    );
                }
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
    mut spawned_entities: Query<(Entity, &mut Spawned, GhostEntites, Option<&PlayerSensors>)>,
) {
    #[cfg(feature = "profiler")]
    puffin::profile_function!();
    for (entity, mut spawned, ghost_entities, player_sensors) in spawned_entities.iter_mut() {
        spawned.pop_outdated_commands(game_time.frame_number);
        if spawned.can_be_removed(game_time.frame_number) {
            log::debug!("Despawning entity {:?}", entity);
            commands.entity(entity).despawn();
            if let Some((
                LevelObjectStaticGhostChild(static_ghost_entity),
                LevelObjectServerGhostChild(server_ghost_entity),
            )) = ghost_entities
            {
                commands.entity(*static_ghost_entity).despawn();
                commands.entity(*server_ghost_entity).despawn();
            }
            if let Some(PlayerSensors { main: _, sensors }) = player_sensors {
                for (sensor_entity, _) in sensors {
                    commands.entity(*sensor_entity).despawn();
                }
            }
            player_entities.remove_by_entity(entity);
            object_entities.remove_by_entity(entity);
        }
    }
}
