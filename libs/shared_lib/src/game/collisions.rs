use crate::game::{
    components::{LevelObjectTag, PlayerSensor, PlayerSensorState, PlayerSensors},
    events::CollisionLogicChanged,
    level::LevelParams,
};
use bevy::{
    app::EventReader,
    ecs::{
        entity::Entity,
        system::{In, Query, RemovedComponents, SystemParam},
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

        if let Ok((player_sensor_entity, PlayerSensor(player_entity))) =
            queries.player_sensors.get(other_entity)
        {
            let mut player_sensors = queries
                .players
                .get_component_mut::<PlayerSensors>(*player_entity)
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
            changed_players.insert(*player_entity);
        } else if let Ok((_, mut player_sensors)) = queries.players.get_mut(other_entity) {
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
            changed_players.insert(player_entity);
        }
    }

    let changed_collision_logic: Vec<_> = collision_logic_changed_events.iter().collect();
    if !changed_collision_logic.is_empty() || !removed_level_objects.is_empty() {
        for (player_entity, mut player_sensors) in queries.players.iter_mut() {
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
                            changed_players.insert(player_entity);
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
