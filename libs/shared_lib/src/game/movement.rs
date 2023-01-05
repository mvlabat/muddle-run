use crate::{
    collider_flags::{
        level_object_collision_groups, player_collision_groups, player_sensor_collision_groups,
    },
    framebuffer::FrameNumber,
    game::{
        components::{
            LevelObjectServerGhostChild, LevelObjectTag, LockPhysics, PlayerDirection,
            PlayerFrameSimulated, PlayerSensor, PlayerTag, Position, PredictedPosition, Spawned,
        },
        spawn::{iter_spawned, SpawnedQuery, SpawnedQueryItem},
    },
    messages::PlayerNetId,
    player::PlayerUpdates,
    registry::EntityRegistry,
    GameTime, SimulationTime, COMPONENT_FRAMEBUFFER_LIMIT, SIMULATIONS_PER_SECOND,
};
use bevy::{
    ecs::{
        entity::Entity,
        query::{With, Without, WorldQuery},
        system::{Query, Res, ResMut},
    },
    log,
    math::Vec2,
    transform::components::Transform,
};
use bevy_rapier2d::{
    dynamics::{RigidBody, Velocity},
    geometry::Group,
    prelude::CollisionGroups,
};

/// Positions should align in 0.25 seconds.
fn lerp_factor() -> f32 {
    1.0 / SIMULATIONS_PER_SECOND * 4.0
}

/// The scaling factor for the player's linear velocity.
fn player_movement_speed() -> f32 {
    360.0 / SIMULATIONS_PER_SECOND
}

pub fn read_movement_updates(
    time: Res<GameTime>,
    simulation_time: Res<SimulationTime>,
    mut player_updates: ResMut<PlayerUpdates>,
    player_registry: Res<EntityRegistry<PlayerNetId>>,
    mut players: Query<(
        Entity,
        &mut Position,
        &mut PlayerDirection,
        &Spawned,
        Option<&PlayerFrameSimulated>,
    )>,
) {
    #[cfg(feature = "profiler")]
    puffin::profile_function!();
    for (entity, mut position, mut player_direction, spawned, player_frame_simulated) in
        players.iter_mut()
    {
        let player_net_id = player_registry
            .get_id(entity)
            .expect("Expected a registered player");

        let range = if player_frame_simulated.is_some() {
            simulation_time.player_frame..=time.frame_number
        } else {
            simulation_time.server_frame
                ..=(time.frame_number - FrameNumber::new(simulation_time.player_frames_ahead()))
        };
        log::trace!(
            "Reading updates for player {}: {:?}",
            player_net_id.0,
            range
        );
        for frame_number in range {
            if !spawned.is_spawned(frame_number) {
                continue;
            }

            if let Some(position_update) = player_updates
                .position
                .get(&player_net_id)
                .and_then(|buffer| buffer.get(frame_number))
                .and_then(|p| *p)
            {
                log::trace!(
                    "Position update for player {} (frame_number: {}): {}",
                    player_net_id.0,
                    frame_number,
                    position_update
                );
                position.buffer.insert(frame_number, position_update);
            } else {
                log::trace!(
                    "No updates for player {} (frame_number: {})",
                    player_net_id.0,
                    frame_number
                );
            }

            let direction_update = player_updates
                .get_direction_mut(player_net_id, frame_number, COMPONENT_FRAMEBUFFER_LIMIT)
                .get_mut(frame_number);
            let current_direction = player_direction
                .buffer
                .get(frame_number)
                .and_then(|update| *update);
            log::trace!(
                "Inserting player (entity: {:?}) direction update for frame {} (current frame: {}): {:?} (current: {:?})",
                entity,
                frame_number,
                time.frame_number,
                direction_update,
                current_direction,
            );
            let update = direction_update
                .and_then(|direction_update| {
                    direction_update.as_mut().map(|direction_update| {
                        if cfg!(feature = "client") {
                            direction_update.is_processed_client_input = Some(true);
                        }
                        direction_update.direction
                    })
                })
                // Avoid replacing initial updates with None.
                .or(current_direction);
            player_direction.buffer.insert(frame_number, update);
        }
    }
}

#[derive(WorldQuery)]
#[world_query(mutable, derive(Debug))]
pub struct PlayerQuery<'w> {
    entity: Entity,
    transform: &'w mut Transform,
    velocity: &'w mut Velocity,
    direction: &'w PlayerDirection,
    position: &'w Position,
}

pub fn player_movement(time: Res<SimulationTime>, mut players: Query<SpawnedQuery<PlayerQuery>>) {
    #[cfg(feature = "profiler")]
    puffin::profile_function!();
    log::trace!(
        "Moving players (frame {}, {})",
        time.server_frame,
        time.player_frame
    );
    for SpawnedQueryItem {
        item: mut player,
        player_frame_simulated,
        spawned,
    } in iter_spawned(players.iter_mut(), &time)
    {
        // Skip non-local entities if we are correcting client's mispredictions.
        if time.player_frame_simulated_only() && player_frame_simulated.is_none() {
            continue;
        }

        let frame_number = time.entity_simulation_frame(player_frame_simulated);

        let body_position = &mut player.transform;
        let current_position = player.position.buffer.get(frame_number).unwrap_or_else(|| {
            // This can happen only if our `sync_position` haven't created a new position for
            // the current frame. If we are catching this, it's definitely a bug.
            panic!(
                "Expected player (entity: {:?}) position for frame {} (start frame: {}, len: {}, spawned: {:?})",
                player.entity,
                frame_number,
                player.position.buffer.start_frame(),
                player.position.buffer.len(),
                spawned,
            );
        });
        body_position.translation.x = current_position.x;
        body_position.translation.y = current_position.y;

        let zero_vec = Vec2::new(0.0, 0.0);
        let (_, current_direction) = player
            .direction
            .buffer
            .get_with_extrapolation(frame_number)
            .unwrap_or_else(|| {
                // We haven't received updates about a player for too long, so we assume that it
                // stopped moving.
                log::debug!(
                    "No player (entity: {:?}) direction for frame {}",
                    player.entity,
                    frame_number
                );
                (FrameNumber::new(0), &zero_vec)
            });
        player.velocity.linvel = current_direction.normalize_or_zero() * player_movement_speed();
    }
}

#[derive(WorldQuery)]
#[world_query(mutable, derive(Debug))]
pub struct LevelObjectQuery<'w> {
    entity: Entity,
    transform: &'w mut Transform,
    position: &'w Position,
    server_ghost: Option<&'w LevelObjectServerGhostChild>,
    frame_simulated: Option<&'w PlayerFrameSimulated>,
    _tag: With<LevelObjectTag>,
}

#[derive(WorldQuery)]
#[world_query(mutable)]
pub struct SimulatedObjectQuery<'w> {
    entity: Entity,
    lock_physics: &'w mut LockPhysics,
    rigid_body: Option<&'w mut RigidBody>,
    collision_groups: &'w mut CollisionGroups,
    player_tag: Option<&'w PlayerTag>,
    player_sensor: Option<&'w PlayerSensor>,
}

/// Exclude objects that won't affect a local player by simulations if a client
/// is correcting mispredictions.
pub fn isolate_client_mispredicted_world(
    time: Res<SimulationTime>,
    mut objects: Query<SimulatedObjectQuery, Without<PlayerFrameSimulated>>,
) {
    #[cfg(feature = "profiler")]
    puffin::profile_function!();

    if time.player_frame_simulated_only() {
        for mut item in objects.iter_mut().filter(|i| !i.lock_physics.0) {
            log::trace!("Lock physics for {:?}", item.entity);
            if let Some(mut rigid_body) = item.rigid_body {
                *rigid_body = RigidBody::Fixed;
            }
            *item.collision_groups = CollisionGroups {
                memberships: Group::NONE,
                filters: Group::NONE,
            };
            item.lock_physics.0 = true;
        }
    } else {
        for mut item in objects.iter_mut().filter(|i| i.lock_physics.0) {
            log::trace!("Unlock physics for {:?}", item.entity);
            if item.player_tag.is_some() {
                *item.rigid_body.unwrap() = RigidBody::Dynamic;
                *item.collision_groups = player_collision_groups(true);
            } else if item.player_sensor.is_some() {
                *item.collision_groups = player_sensor_collision_groups(true);
            } else {
                *item.rigid_body.unwrap() = RigidBody::KinematicPositionBased;
                *item.collision_groups = level_object_collision_groups(true);
            }
            item.lock_physics.0 = false;
        }
    }
}

pub fn load_object_positions(
    time: Res<SimulationTime>,
    mut level_objects: Query<SpawnedQuery<LevelObjectQuery>>,
    #[cfg_attr(not(feature = "client"), allow(unused_variables, unused_mut))]
    mut server_ghost_level_objects: Query<&mut Transform, Without<LevelObjectTag>>,
) {
    #[cfg(feature = "profiler")]
    puffin::profile_function!();
    log::trace!(
        "Loading object positions (frame {}, {})",
        time.server_frame,
        time.player_frame
    );

    for SpawnedQueryItem {
        item: mut level_object,
        player_frame_simulated,
        ..
    } in iter_spawned(level_objects.iter_mut(), &time)
    {
        let frame_number = time.entity_simulation_frame(player_frame_simulated);

        let body_position = &mut level_object.transform;
        let current_position = level_object
            .position
            .buffer
            .get(frame_number)
            .unwrap_or_else(|| {
                // This can happen only if our `sync_position` haven't created a new position for
                // the current frame. If we are catching this, it's definitely a bug.
                panic!(
                    "Expected object (entity: {:?}) position for frame {} (start frame: {}, len: {})",
                    level_object.entity,
                    time.entity_simulation_frame(player_frame_simulated),
                    level_object.position.buffer.start_frame(),
                    level_object.position.buffer.len()
                );
            });
        body_position.translation.x = current_position.x;
        body_position.translation.y = current_position.y;

        #[cfg(feature = "client")]
        if let Some(LevelObjectServerGhostChild(server_ghost)) = level_object.server_ghost {
            let mut body_position =
                crate::util::get_item_mut(&mut server_ghost_level_objects, *server_ghost).unwrap();
            let frame_number = time.entity_simulation_frame(None);
            let current_position = level_object
                .position
                .buffer
                .get(frame_number)
                .or_else(|| level_object.position.buffer.first())
                .unwrap();
            body_position.translation.x = current_position.x;
            body_position.translation.y = current_position.y;
        }
    }
}

#[derive(WorldQuery)]
#[world_query(mutable, derive(Debug))]
pub struct SimulatedEntityQuery<'w> {
    entity: Entity,
    position: &'w mut Position,
    transform: &'w mut Transform,
    predicted_position: Option<&'w mut PredictedPosition>,
}

pub fn sync_position(
    game_time: Res<GameTime>,
    time: Res<SimulationTime>,
    mut simulated_entities: Query<SpawnedQuery<SimulatedEntityQuery>>,
) {
    #[cfg(feature = "profiler")]
    puffin::profile_function!();
    log::trace!(
        "Syncing positions (frame {}, {})",
        time.server_frame,
        time.player_frame
    );
    for SpawnedQueryItem {
        item: mut simulated_entity,
        player_frame_simulated,
        ..
    } in iter_spawned(simulated_entities.iter_mut(), &time)
    {
        let frame_number = time.entity_simulation_frame(player_frame_simulated);
        let body_position = simulated_entity.transform.translation;
        let new_position = body_position.truncate();
        if let Some(predicted_position) = simulated_entity.predicted_position.as_mut() {
            let needs_lerping_predicted_position = time.player_frame == game_time.frame_number;
            if needs_lerping_predicted_position {
                let current_position = *simulated_entity
                    .position
                    .buffer
                    .get(frame_number)
                    .expect("Expected the current position");

                let real_diff = new_position - current_position;
                let new_predicted_position = predicted_position.value + real_diff;
                let lerp = new_predicted_position
                    + (new_position - new_predicted_position) * lerp_factor();
                log::trace!(
                    "Lerping position (e: {:?}, frame: {}, current: {}, new: {}, lerp: {}, player frame: {:?}, positions: {:?})",
                    simulated_entity.entity,
                    frame_number,
                    current_position,
                    new_predicted_position,
                    lerp,
                    player_frame_simulated,
                    simulated_entity.position,
                );

                // The `Transform` component will be updated before the next physics simulation
                // to contain the real (server authoritative) position. This is why we store
                // lerped position in the `PredictedPosition` component as well.
                predicted_position.value = lerp;
                simulated_entity.transform.translation.x = lerp.x;
                simulated_entity.transform.translation.y = lerp.y;
            }
        }

        // Positions buffer represents start positions before moving entities, so this
        // is why we save the new position in the next frame.
        simulated_entity.position.buffer.insert(
            frame_number + FrameNumber::new(1),
            Vec2::new(body_position.x, body_position.y),
        );
    }
}
