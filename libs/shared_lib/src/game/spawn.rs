#[cfg(feature = "client")]
use crate::game::components::PlayerFrameSimulated;
use crate::{
    collider_flags::{player_interaction_groups, player_sensor_interaction_groups},
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
            LevelObjectLabel, LevelObjectStaticGhost, LevelObjectStaticGhostParent, LevelObjectTag,
            PlayerDirection, PlayerSensor, PlayerSensorState, PlayerSensors, PlayerTag, Position,
            SpawnCommand, Spawned,
        },
        level::{ColliderShapeResponse, LevelObject, LevelObjectDesc, LevelState},
    },
    messages::{EntityNetId, PlayerNetId},
    registry::EntityRegistry,
    util::{dedup_by_key_unsorted, player_sensor_outline},
    GameTime, SimulationTime, PLAYER_RADIUS, PLAYER_SENSOR_RADIUS,
};
use bevy::{
    ecs::system::{EntityCommands, SystemParam},
    log,
    prelude::*,
    tasks::AsyncComputeTaskPool,
};
use bevy_rapier2d::{
    physics::{ColliderBundle, ColliderPositionSync, IntoHandle, RigidBodyBundle},
    rapier::{
        dynamics::{
            RigidBodyHandle, RigidBodyMassProps, RigidBodyMassPropsFlags, RigidBodyPosition,
        },
        geometry::{ColliderFlags, ColliderParent, ColliderShape, ColliderType, InteractionGroups},
        pipeline::ActiveEvents,
    },
};

pub type ColliderShapePromiseResult = (Entity, Option<ColliderShape>);
pub type ColliderShapeSender = crossbeam_channel::Sender<ColliderShapePromiseResult>;
pub type ColliderShapeReceiver = crossbeam_channel::Receiver<ColliderShapePromiseResult>;

pub type PlayersQuery<'s> = Query<
    's,
    (
        Entity,
        &'static mut Spawned,
        &'static mut Position,
        &'static mut PlayerDirection,
        &'static mut ColliderFlags,
        &'static PlayerSensors,
    ),
    Without<PlayerSensor>,
>;

pub fn spawn_players(
    mut commands: Commands,
    time: Res<SimulationTime>,
    mut pbr_client_params: PbrClientParams,
    mut spawn_player_commands: ResMut<DeferredQueue<SpawnPlayer>>,
    mut player_entities: ResMut<EntityRegistry<PlayerNetId>>,
    mut players: PlayersQuery,
    mut player_sensors: Query<&mut ColliderFlags, With<PlayerSensor>>,
) {
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

            let (
                _,
                mut spawned,
                mut position,
                mut player_direction,
                mut collider_flags,
                PlayerSensors { main: _, sensors },
            ) = players.get_mut(entity).unwrap();
            for frame_number in time.server_frame..=time.player_frame {
                position.buffer.insert(frame_number, command.start_position);
                player_direction
                    .buffer
                    .insert(frame_number, Some(Vec2::ZERO));
            }
            PlayerClientFactory::insert_components(
                &mut entity_commands,
                &mut pbr_client_params,
                command.start_position,
            );
            collider_flags.collision_groups = player_interaction_groups();
            for (player_sensor_entity, _) in sensors {
                let mut collider_flags = player_sensors.get_mut(*player_sensor_entity).unwrap();
                collider_flags.collision_groups = player_sensor_interaction_groups();
                PlayerSensorClientFactory::insert_components(
                    &mut entity_commands.commands().entity(*player_sensor_entity),
                    &mut pbr_client_params,
                    (),
                );
            }

            #[cfg(feature = "client")]
            if command.is_player_frame_simulated {
                entity_commands.insert(PlayerFrameSimulated);
            }
            spawned.push_command(time.server_frame, SpawnCommand::Spawn);

            continue;
        }

        let mut entity_commands = commands.spawn();
        let player_entity = entity_commands.id();

        let mut sensors = Vec::new();
        for sensor_position in player_sensor_outline() {
            let mut sensor_commands = entity_commands.commands().spawn();
            sensor_commands
                .insert_bundle(ColliderBundle {
                    shape: ColliderShape::ball(PLAYER_SENSOR_RADIUS),
                    collider_type: ColliderType::Sensor,
                    flags: ColliderFlags {
                        collision_groups: player_sensor_interaction_groups(),
                        solver_groups: InteractionGroups::none(),
                        active_events: ActiveEvents::INTERSECTION_EVENTS,
                        ..ColliderFlags::default()
                    },
                    ..ColliderBundle::default()
                })
                .insert(ColliderParent {
                    handle: RigidBodyHandle(player_entity.handle()),
                    pos_wrt_parent: sensor_position.into(),
                })
                .insert(ColliderPositionSync::Discrete)
                .insert(PlayerSensor(player_entity));
            PlayerSensorClientFactory::insert_components(
                &mut sensor_commands,
                &mut pbr_client_params,
                (),
            );
            sensors.push((sensor_commands.id(), PlayerSensorState::default()));
        }

        PlayerClientFactory::insert_components(
            &mut entity_commands,
            &mut pbr_client_params,
            command.start_position,
        );
        entity_commands
            .insert(PlayerTag)
            .insert_bundle(RigidBodyBundle {
                position: [0.0, 0.0].into(),
                mass_properties: RigidBodyMassProps {
                    flags: RigidBodyMassPropsFlags::ROTATION_LOCKED,
                    ..RigidBodyMassProps::default()
                },
                ..RigidBodyBundle::default()
            })
            .insert_bundle(ColliderBundle {
                shape: ColliderShape::ball(PLAYER_RADIUS),
                flags: ColliderFlags {
                    collision_groups: player_interaction_groups(),
                    solver_groups: player_interaction_groups(),
                    active_events: ActiveEvents::all(),
                    ..ColliderFlags::default()
                },
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
        (Entity, &mut Spawned, &mut ColliderFlags, &mut PlayerSensors),
        Without<PlayerSensor>,
    >,
    mut player_sensors: Query<&mut ColliderFlags, With<PlayerSensor>>,
) {
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
        collider_flags.collision_groups.memberships = 0;
        PlayerClientFactory::remove_components(&mut entity_commands, &mut pbr_client_params);
        for (player_sensor_entity, _state) in &mut sensors.sensors {
            let mut collider_flags = player_sensors.get_mut(*player_sensor_entity).unwrap();
            collider_flags.collision_groups.memberships = 0;
            PlayerSensorClientFactory::remove_components(
                &mut commands.entity(*player_sensor_entity),
                &mut pbr_client_params,
            );
        }
        spawned.push_command(command.frame_number, SpawnCommand::Despawn);
    }
}

type UpdateLevelObjectsQuery<'a> = Query<
    'a,
    (
        Entity,
        Option<&'static mut Position>,
        &'static mut Spawned,
        Option<&'static LevelObjectStaticGhostParent>,
    ),
    With<LevelObjectTag>,
>;

#[derive(SystemParam)]
pub struct LevelObjectsParams<'a> {
    object_entities: ResMut<'a, EntityRegistry<EntityNetId>>,
    level_state: ResMut<'a, LevelState>,
    level_objects: UpdateLevelObjectsQuery<'a>,
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
    puffin::profile_function!();
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

        // Unlike with players, here we just despawn level objects if they are updated and re-create
        // from scratch.
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
            let (_, position, spawned, ghost_parent) = level_object_params
                .level_objects
                .get_mut(existing_entity)
                .expect("Expected a registered level object entity to exist");
            if let Some(LevelObjectStaticGhostParent(ghost_entity)) = &ghost_parent {
                commands.entity(*ghost_entity).despawn();
            }
            if let Some(mut position) = position {
                position_component = Some(position.take());
            }
            spawned_component = spawned.clone();
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
        entity_commands.insert(Transform::from_translation(
            command
                .object
                .desc
                .position()
                .unwrap_or_default()
                .extend(0.0),
        ));
        if let Some(ref shape) = shape {
            let (rigid_body, collider) = command.object.desc.physics_body(shape.clone(), false);
            insert_client_components(
                &mut entity_commands,
                &command.object,
                false,
                &collider.shape,
                &mut pbr_client_params,
            );
            entity_commands
                .insert_bundle(rigid_body)
                .insert_bundle(collider)
                .insert(ColliderPositionSync::Discrete);
        }

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
        entity_commands
            .insert(command.object.net_id)
            .insert(LevelObjectTag)
            .insert(LevelObjectLabel(command.object.label.clone()))
            .insert(RigidBodyPosition::from(
                command.object.desc.position().unwrap(),
            ))
            .insert(spawned_component);

        #[cfg(feature = "client")]
        entity_commands.insert(PlayerFrameSimulated);

        let level_object_entity = entity_commands.id();
        level_object_params
            .object_entities
            .register(command.object.net_id, level_object_entity);

        if cfg!(feature = "client") {
            // Spawning the ghost object.
            let ghost = entity_commands.commands().spawn();
            let ghost_entity = ghost.id();
            entity_commands.insert(LevelObjectStaticGhostParent(ghost_entity));
            let mut ghost_commands = commands.entity(ghost_entity);
            ghost_commands
                .insert(Transform::from_translation(
                    command
                        .object
                        .desc
                        .position()
                        .unwrap_or_default()
                        .extend(0.0),
                ))
                .insert(RigidBodyPosition::from(
                    command.object.desc.position().unwrap(),
                ));
            if let Some(shape) = shape {
                let (rigid_body, collider) = command.object.desc.physics_body(shape, true);
                insert_client_components(
                    &mut ghost_commands,
                    &command.object,
                    true,
                    &collider.shape,
                    &mut pbr_client_params,
                );
                ghost_commands
                    .insert_bundle(rigid_body)
                    .insert_bundle(collider)
                    .insert(ColliderPositionSync::Discrete);
            }
            ghost_commands.insert(LevelObjectStaticGhost(level_object_entity));
        }
    }
}

pub fn poll_calculating_shapes(
    mut commands: Commands,
    time: Res<SimulationTime>,
    level_state: Res<LevelState>,
    mut pbr_client_params: PbrClientParams,
    level_objects_query: Query<(
        &EntityNetId,
        &Spawned,
        Option<&LevelObjectStaticGhostParent>,
    )>,
    collider_shape_receiver: Res<ColliderShapeReceiver>,
) {
    while let Ok((entity, shape_result)) = collider_shape_receiver.try_recv() {
        let (entity_net_id, spawned, ghost_parent) = match level_objects_query.get(entity) {
            Ok(r) => r,
            Err(_) => continue,
        };

        if !spawned.is_spawned(time.player_frame) {
            continue;
        }

        let level_object = level_state.objects.get(entity_net_id).unwrap();

        let mut entity_commands = commands.entity(entity);

        let shape = match shape_result {
            Some(shape) => {
                log::debug!(
                    "Calculating shape for {:?} has finished (frame: {})",
                    entity,
                    time.player_frame
                );
                shape
            }
            None => {
                log::error!(
                    "Calculating shape for {:?} has failed (frame: {})",
                    entity,
                    time.player_frame
                );
                // Even if we don't render an object, we still want nested transforms to work.
                if let Some(position) = level_object.desc.position() {
                    entity_commands.insert(GlobalTransform::from_translation(position.extend(0.0)));
                    entity_commands.insert(Transform::from_translation(position.extend(0.0)));
                    if let Some(LevelObjectStaticGhostParent(ghost_entity)) = ghost_parent {
                        let mut entity_commands = commands.entity(*ghost_entity);
                        entity_commands
                            .insert(GlobalTransform::from_translation(position.extend(0.0)));
                        entity_commands.insert(Transform::from_translation(position.extend(0.0)));
                    }
                }
                continue;
            }
        };

        let (rigid_body, collider) = level_object.desc.physics_body(shape.clone(), false);
        insert_client_components(
            &mut entity_commands,
            level_object,
            false,
            &collider.shape,
            &mut pbr_client_params,
        );
        entity_commands
            .insert_bundle(rigid_body)
            .insert_bundle(collider)
            .insert(ColliderPositionSync::Discrete);

        if let Some(LevelObjectStaticGhostParent(ghost_entity)) = ghost_parent {
            let mut entity_commands = commands.entity(*ghost_entity);
            let (rigid_body, collider) = level_object.desc.physics_body(shape, true);
            insert_client_components(
                &mut entity_commands,
                level_object,
                true,
                &collider.shape,
                &mut pbr_client_params,
            );
            entity_commands
                .insert_bundle(rigid_body)
                .insert_bundle(collider)
                .insert(ColliderPositionSync::Discrete);
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
            &mut ColliderFlags,
            Option<&LevelObjectStaticGhostParent>,
        ),
        With<LevelObjectTag>,
    >,
) {
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
            "Despawning level object {} (frame {})",
            command.net_id.0,
            command.frame_number
        );
        collider_flags.collision_groups.memberships = 0;
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
                if let Some(LevelObjectStaticGhostParent(ghost_entity)) = ghost_parent {
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
                if let Some(LevelObjectStaticGhostParent(ghost_entity)) = ghost_parent {
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
                if let Some(LevelObjectStaticGhostParent(ghost_entity)) = ghost_parent {
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
    mut spawned_entities: Query<(
        Entity,
        &mut Spawned,
        Option<&LevelObjectStaticGhostParent>,
        Option<&PlayerSensors>,
    )>,
) {
    puffin::profile_function!();
    for (entity, mut spawned, ghost, player_sensors) in spawned_entities.iter_mut() {
        spawned.pop_outdated_commands(game_time.frame_number);
        if spawned.can_be_removed(game_time.frame_number) {
            log::debug!("Despawning entity {:?}", entity);
            commands.entity(entity).despawn();
            if let Some(LevelObjectStaticGhostParent(ghost_entity)) = ghost {
                commands.entity(*ghost_entity).despawn();
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
