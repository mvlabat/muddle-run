use crate::{
    game::{
        components::{
            LevelObjectTag, PlayerFrameSimulated, PlayerSensor, PlayerSensorState, PlayerSensors,
            Position, Spawned,
        },
        events::{CollisionLogicChanged, PlayerDeath, PlayerFinish},
        level::LevelParams,
    },
    SimulationTime,
};
use bevy::{
    app::{EventReader, EventWriter},
    ecs::{
        entity::Entity,
        system::{In, Query, RemovedComponents, Res, SystemParam},
    },
    log,
    utils::HashSet,
};
use bevy_rapier2d::{
    physics::IntoEntity,
    rapier::geometry::{ContactEvent, IntersectionEvent},
};

#[derive(SystemParam)]
pub struct CollisionQueries<'a> {
    players: Query<
        'a,
        (
            Entity,
            &'static Spawned,
            Option<&'static PlayerFrameSimulated>,
            &'static mut PlayerSensors,
        ),
    >,
    player_sensors: Query<'a, (Entity, &'static PlayerSensor)>,
}

/// The system returns player entities whose intersections were changed.
pub fn process_collision_events(
    time: Res<SimulationTime>,
    mut contact_events: EventReader<ContactEvent>,
    mut intersection_events: EventReader<IntersectionEvent>,
    mut collision_logic_changed_events: EventReader<CollisionLogicChanged>,
    mut queries: CollisionQueries,
    removed_level_objects: RemovedComponents<LevelObjectTag>,
    level: LevelParams,
) -> Vec<Entity> {
    let mut changed_players = HashSet::default();
    let removed_level_objects = removed_level_objects.iter().collect::<Vec<_>>();

    let mut all_events: Vec<(bool, Entity, Entity)> = Vec::new();
    all_events.extend(intersection_events.iter().map(|event| {
        (
            event.intersecting,
            event.collider1.entity(),
            event.collider2.entity(),
        )
    }));
    all_events.extend(contact_events.iter().map(|event| match event {
        ContactEvent::Started(c1, c2) => (true, c1.entity(), c2.entity()),
        ContactEvent::Stopped(c1, c2) => (false, c1.entity(), c2.entity()),
    }));

    for (contacting, entity1, entity2) in all_events.into_iter() {
        let (level_object_entity, level_object, other_entity) = match (
            level.level_object_by_entity(entity1),
            level.level_object_by_entity(entity2),
        ) {
            (Some(level_object), None) => (entity1, level_object, entity2),
            (None, Some(level_object)) => (entity2, level_object, entity1),
            _ => {
                log::error!(
                    "None of the intersected entities is a level object: {:?}, {:?}",
                    entity1,
                    entity2
                );
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
            log::error!(
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

pub fn process_players_with_new_collisions(
    In(players_with_new_collisions): In<Vec<Entity>>,
    time: Res<SimulationTime>,
    players: Query<(&Position, Option<&PlayerFrameSimulated>, &PlayerSensors)>,
    mut player_death_events: EventWriter<PlayerDeath>,
    mut player_finish_events: EventWriter<PlayerFinish>,
) {
    for entity in players_with_new_collisions {
        let (player_position, player_frame_simulated, player_sensors) = players
            .get(entity)
            .expect("Expected an existing player for a collision event");
        let _player_position = player_position
            .buffer
            .get(time.entity_simulation_frame(player_frame_simulated))
            .unwrap();

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
