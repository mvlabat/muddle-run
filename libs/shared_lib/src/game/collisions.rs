use crate::game::{
    components::{PlayerSensor, PlayerSensorState, PlayerSensors},
    events::CollisionLogicChanged,
    level::LevelParams,
};
use bevy::{
    app::EventReader,
    ecs::{
        entity::Entity,
        system::{In, Query, SystemParam},
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
    players: Query<'a, (Entity, &'static mut PlayerSensors)>,
    player_sensors: Query<'a, (Entity, &'static PlayerSensor)>,
}

/// The system returns player entities whose intersections were changed.
pub fn process_collision_events(
    mut contact_events: EventReader<ContactEvent>,
    mut intersection_events: EventReader<IntersectionEvent>,
    mut collision_logic_changed_events: EventReader<CollisionLogicChanged>,
    mut queries: CollisionQueries,
    level: LevelParams,
) -> Vec<Entity> {
    let mut changed_players = HashSet::default();

    for event in intersection_events.iter() {
        let player_sensor = queries
            .player_sensors
            .get(event.collider1.entity())
            .ok()
            .or_else(|| queries.player_sensors.get(event.collider2.entity()).ok())
            .map(|(entity, sensor)| (entity, sensor.clone()));

        let (level_object_entity, level_object) = match (
            level.level_object_by_entity(event.collider1.entity()),
            level.level_object_by_entity(event.collider2.entity()),
        ) {
            (Some(level_object), None) => (event.collider1.entity(), level_object),
            (None, Some(level_object)) => (event.collider2.entity(), level_object),
            _ => continue,
        };

        if let Some((player_sensor_entity, PlayerSensor(player_entity))) = player_sensor {
            let mut player_sensors = queries
                .players
                .get_component_mut::<PlayerSensors>(player_entity)
                .expect("Expected a player for an existing sensor");
            let (_, sensor_state) = player_sensors
                .sensors
                .iter_mut()
                .find(|(sensor_entity, _)| *sensor_entity == player_sensor_entity)
                .expect("Player is expected to know a sensor connected to it");

            if event.intersecting {
                sensor_state
                    .contacting
                    .push((level_object_entity, level_object.collision_logic));
            } else {
                sensor_state
                    .contacting
                    .drain_filter(|(entity, _)| *entity == level_object_entity);
            }
            changed_players.insert(player_entity);
        } else if let Ok((_, mut player_sensors)) =
            queries.players.get_mut(event.collider1.entity())
        {
            let player_entity = event.collider1.entity();
            // Intersection with a player collider itself.
            if event.intersecting {
                player_sensors
                    .main
                    .contacting
                    .push((player_entity, level_object.collision_logic));
            } else {
                player_sensors
                    .main
                    .contacting
                    .drain_filter(|(entity, _)| *entity == event.collider2.entity());
            }
            changed_players.insert(player_entity);
        }
    }

    for event in contact_events.iter() {
        let (contacting, entity1, entity2) = match event {
            ContactEvent::Started(entity1, entity2) => (true, entity1.entity(), entity2.entity()),
            ContactEvent::Stopped(entity1, entity2) => (false, entity1.entity(), entity2.entity()),
        };

        let (player_entity, level_object_entity, level_object) = match (
            level.level_object_by_entity(entity1),
            level.level_object_by_entity(entity2),
        ) {
            (Some(level_object), None) => (entity2, entity1, level_object),
            (None, Some(level_object)) => (entity1, entity2, level_object),
            _ => continue,
        };

        // We expect contact events to happen only to players.
        if let Ok((_, mut player_sensors)) = queries.players.get_mut(player_entity) {
            if contacting {
                player_sensors
                    .main
                    .contacting
                    .push((player_entity, level_object.collision_logic));
            } else {
                player_sensors
                    .main
                    .contacting
                    .drain_filter(|(entity, _)| *entity == level_object_entity);
            }
            changed_players.insert(player_entity);
        }
    }

    let changed_collision_logic: Vec<_> = collision_logic_changed_events.iter().collect();
    if !changed_collision_logic.is_empty() {
        for (player_entity, mut player_sensors) in queries.players.iter_mut() {
            for CollisionLogicChanged {
                level_object_entity,
                collision_logic,
            } in changed_collision_logic.iter()
            {
                let mut update_collision_logic = |sensor_state: &mut PlayerSensorState| {
                    for (contacted_entity, logic) in &mut sensor_state.contacting {
                        if contacted_entity == level_object_entity {
                            *logic = *collision_logic;
                            changed_players.insert(player_entity);
                        }
                    }
                };

                update_collision_logic(&mut player_sensors.main);
                for (_, sensor) in &mut player_sensors.sensors {
                    update_collision_logic(sensor);
                }
            }
        }
    }

    changed_players.into_iter().collect()
}

pub fn process_players_with_new_collisions(
    In(players_with_new_collisions): In<Vec<Entity>>,
    players: Query<&PlayerSensors>,
) {
    for entity in players_with_new_collisions {
        let player_sensors = players
            .get(entity)
            .expect("Expected an existing player for a collision event");

        if player_sensors.player_is_dead() {
            log::debug!("Player {:?} has died", entity);
        } else if player_sensors.player_has_finished() {
            log::debug!("Player {:?} has finished", entity);
        }
    }
}
