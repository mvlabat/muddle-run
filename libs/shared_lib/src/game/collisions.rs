use crate::{
    game::{
        components::{
            LevelObjectServerGhostParent, LevelObjectTag, PlayerFrameSimulated, PlayerSensor,
            PlayerSensorState, PlayerSensors, Position, Spawned,
        },
        events::{CollisionLogicChanged, PlayerDeath, PlayerFinish},
        level::LevelParams,
    },
    util::get_item,
    SimulationTime,
};
use bevy::{
    ecs::{
        entity::Entity,
        event::{EventReader, EventWriter},
        query::QueryEntityError,
        system::{In, Query, RemovedComponents, Res, SystemParam},
    },
    log,
    utils::HashSet,
};
use bevy_rapier2d::pipeline::CollisionEvent;

#[derive(SystemParam)]
pub struct CollisionQueries<'w, 's> {
    players: Query<
        'w,
        's,
        (
            Entity,
            &'static Spawned,
            Option<&'static PlayerFrameSimulated>,
            &'static mut PlayerSensors,
        ),
    >,
    player_sensors: Query<'w, 's, (Entity, &'static PlayerSensor)>,
    all_entities: Query<'w, 's, Entity>,
}

/// The system returns player entities whose intersections were changed.
pub fn process_collision_events_system(
    time: Res<SimulationTime>,
    mut collision_events: EventReader<CollisionEvent>,
    mut collision_logic_changed_events: EventReader<CollisionLogicChanged>,
    mut queries: CollisionQueries,
    removed_level_objects: RemovedComponents<LevelObjectTag>,
    level_object_server_ghost_parents: Query<&LevelObjectServerGhostParent>,
    level: LevelParams,
) -> Vec<Entity> {
    let mut changed_players = HashSet::default();
    let removed_level_objects = removed_level_objects.iter().collect::<Vec<_>>();

    for event in collision_events.iter() {
        let (contacting, mut entity1, mut entity2) = match event {
            CollisionEvent::Started(e1, e2, _flags) => (true, *e1, *e2),
            CollisionEvent::Stopped(e1, e2, _flags) => (false, *e1, *e2),
        };
        if let Err(QueryEntityError::NoSuchEntity(e)) =
            queries.all_entities.get_many([entity1, entity2])
        {
            // This is a valid case, happens when the game is restarted.
            log::debug!("Entity {e:?} doesn't exists, skipping the collision event {event:?}");
            continue;
        }

        if let Some(LevelObjectServerGhostParent(level_object_entity)) =
            get_item(&level_object_server_ghost_parents, entity1)
        {
            entity1 = *level_object_entity;
        }
        if let Some(LevelObjectServerGhostParent(level_object_entity)) =
            get_item(&level_object_server_ghost_parents, entity2)
        {
            entity2 = *level_object_entity;
        }

        let (level_object_entity, level_object, other_entity) = match (
            level.level_object_by_entity(entity1),
            level.level_object_by_entity(entity2),
        ) {
            (Some(level_object), None) => (entity1, level_object, entity2),
            (None, Some(level_object)) => (entity2, level_object, entity1),
            _ => {
                log::error!("None of the intersected entities is a level object: {event:?}");
                continue;
            }
        };
        log::trace!(
            "Contact event: {:?}, {:?}, {:?}",
            contacting,
            entity1,
            entity2
        );

        if let Ok((player_sensor_entity, PlayerSensor(player_entity))) =
            queries.player_sensors.get(other_entity)
        {
            let (_, spawned, player_frame_simulated, mut player_sensors) = queries
                .players
                .get_mut(*player_entity)
                .expect("Expected a player for an existing sensor");
            let (_, sensor_state) = player_sensors
                .sensors
                .iter_mut()
                .find(|(sensor_entity, _)| *sensor_entity == player_sensor_entity)
                .expect("Player is expected to know a sensor connected to it");

            if contacting {
                sensor_state
                    .contacting
                    .push((level_object_entity, level_object.collision_logic));
            } else {
                sensor_state
                    .contacting
                    .drain_filter(|(entity, _)| *entity == level_object_entity);
            }
            if spawned.is_spawned(time.entity_simulation_frame(player_frame_simulated)) {
                changed_players.insert(*player_entity);
            }
        } else if let Ok((_, spawned, player_frame_simulated, mut player_sensors)) =
            queries.players.get_mut(other_entity)
        {
            let player_entity = other_entity;
            // Intersection with a player collider itself.
            if contacting {
                player_sensors
                    .main
                    .contacting
                    .push((level_object_entity, level_object.collision_logic));
            } else {
                player_sensors
                    .main
                    .contacting
                    .drain_filter(|(entity, _)| *entity == level_object_entity);
            }
            if spawned.is_spawned(time.entity_simulation_frame(player_frame_simulated)) {
                changed_players.insert(player_entity);
            }
        } else {
            log::warn!(
                "Contact event for neither a player, nor a player sensor: {:?}",
                other_entity
            );
        }
    }

    let changed_collision_logic: Vec<_> = collision_logic_changed_events.iter().collect();
    if !changed_collision_logic.is_empty() || !removed_level_objects.is_empty() {
        for (player_entity, spawned, player_frame_simulated, mut player_sensors) in
            queries.players.iter_mut()
        {
            let mut update_collision_logic = |sensor_state: &mut PlayerSensorState| {
                sensor_state
                    .contacting
                    .drain_filter(|(contacted_entity, logic)| {
                        if removed_level_objects.contains(contacted_entity) {
                            return true;
                        }

                        if let Some(changed) = changed_collision_logic
                            .iter()
                            .find(|changed| changed.level_object_entity == *contacted_entity)
                        {
                            *logic = changed.collision_logic;
                            if spawned
                                .is_spawned(time.entity_simulation_frame(player_frame_simulated))
                            {
                                changed_players.insert(player_entity);
                            }
                        }
                        false
                    });
            };

            update_collision_logic(&mut player_sensors.main);
            for (_, sensor) in &mut player_sensors.sensors {
                update_collision_logic(sensor);
            }
        }
    }

    changed_players.into_iter().collect()
}

pub fn process_players_with_new_collisions_system(
    In(players_with_new_collisions): In<Vec<Entity>>,
    time: Res<SimulationTime>,
    players: Query<(&Position, Option<&PlayerFrameSimulated>, &PlayerSensors)>,
    mut player_death_events: EventWriter<PlayerDeath>,
    mut player_finish_events: EventWriter<PlayerFinish>,
) {
    for entity in players_with_new_collisions {
        let (player_position_buffer, player_frame_simulated, player_sensors) = players
            .get(entity)
            .expect("Expected an existing player for a collision event");
        let frame_number = time.entity_simulation_frame(player_frame_simulated);
        let _player_position = match player_position_buffer.buffer.get(frame_number) {
            Some(position) => position,
            None => {
                log::warn!("Player position doesn't exist for frame {} (the entity {:?} is likely despawned), ignoring", frame_number, entity);
                continue;
            }
        };

        if player_sensors.player_is_dead() {
            #[cfg(not(feature = "client"))]
            log::debug!(
                "Player {:?} has died at position {:?}",
                entity,
                _player_position
            );
            player_death_events.send(PlayerDeath(entity));
        } else if player_sensors.player_has_finished() {
            #[cfg(not(feature = "client"))]
            log::debug!(
                "Player {:?} has finished at position {:?}",
                entity,
                _player_position
            );
            player_finish_events.send(PlayerFinish(entity));
        }
    }
}
