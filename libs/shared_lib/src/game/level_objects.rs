use crate::{
    framebuffer::FrameNumber,
    game::{
        components::{
            LevelObjectMovement, LevelObjectMovementPoint, LevelObjectMovementType, LevelObjectTag,
            Position, Spawned,
        },
        level::{LevelState, ObjectRouteDesc},
    },
    messages::EntityNetId,
    registry::EntityRegistry,
    SimulationTime,
};
use bevy::{
    ecs::{
        entity::Entity,
        query::With,
        system::{Commands, Query, QuerySet, Res},
    },
    math::Vec2,
    utils::HashMap,
};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct PlaneDesc {
    pub position: Vec2,
    pub form_desc: PlaneFormDesc,
    pub is_spawn_area: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum PlaneFormDesc {
    Circle { radius: f32 },
    Rectangle { size: Vec2 },
    Concave { points: Vec<Vec2> },
}

impl std::fmt::Display for PlaneFormDesc {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PlaneFormDesc::Circle { .. } => write!(f, "Circle"),
            PlaneFormDesc::Rectangle { .. } => write!(f, "Rectangle"),
            PlaneFormDesc::Concave { .. } => write!(f, "Concave"),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct CubeDesc {
    pub size: f32,
    pub position: Vec2,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct RoutePointDesc {
    pub position: Vec2,
}

pub fn update_level_object_movement_route_settings(
    mut commands: Commands,
    time: Res<SimulationTime>,
    level: Res<LevelState>,
    objects_registry: Res<EntityRegistry<EntityNetId>>,
    mut level_objects: Query<
        (
            Entity,
            Option<&mut LevelObjectMovement>,
            &mut Position,
            &Spawned,
        ),
        With<LevelObjectTag>,
    >,
) {
    #[cfg(feature = "profiler")]
    puffin::profile_function!();
    for (entity, movement, mut position, _) in level_objects
        .iter_mut()
        .filter(|(_, _, _, spawned)| spawned.is_spawned(time.player_frame))
    {
        let level_def = match level.objects.get(&objects_registry.get_id(entity).unwrap()) {
            Some(level_def) => level_def.clone(),
            // The object is might be removed from the level when we are rewinding to the frame
            // when it still existed. We can skip it, it's not the end of the world.
            None => continue,
        };

        let initial_object_position = level_def
            .desc
            .position()
            .expect("Objects without a position are not yet supported");

        let mut is_invalid = false;
        let new_level_object_movement = level_def.route.and_then(|route| {
            let (movement_type, points) = match route.desc {
                ObjectRouteDesc::Attached(None) | ObjectRouteDesc::Radial(None) => return None,
                ObjectRouteDesc::Attached(Some(point)) | ObjectRouteDesc::Radial(Some(point)) => {
                    (LevelObjectMovementType::Radial, vec![point])
                }
                ObjectRouteDesc::ForwardCycle(points) => {
                    if points.is_empty() {
                        return None;
                    }
                    (LevelObjectMovementType::Linear, points)
                }
                ObjectRouteDesc::ForwardBackwardsCycle(mut points) => {
                    if points.is_empty() {
                        return None;
                    }
                    let mut cycle = points.iter().rev().skip(1).cloned().collect::<Vec<_>>();
                    points.append(&mut cycle);
                    (LevelObjectMovementType::Linear, points)
                }
            };

            let has_invalid_point = points
                .iter()
                .any(|point| objects_registry.get_entity(*point).is_none());
            if has_invalid_point {
                is_invalid = true;
                return None;
            }

            let frame_started = closest_start_frame_to_time(
                time.player_generation,
                time.player_frame,
                route.start_frame_offset,
                route.period,
            );
            let mut points_progress = points
                .into_iter()
                .map(|point| LevelObjectMovementPoint {
                    progress: 0.0,
                    position: Vec2::ZERO,
                    entity: objects_registry.get_entity(point).unwrap(),
                })
                .collect::<Vec<_>>();
            points_progress.last_mut().unwrap().progress = 1.0;
            let init_vec = match movement_type {
                LevelObjectMovementType::Radial => {
                    let attached_point = level
                        .objects
                        .get(&objects_registry.get_id(points_progress[0].entity).unwrap())
                        .map_or(initial_object_position, |level_object| {
                            level_object
                                .desc
                                .position()
                                .expect("Object without a position can't be an attachable point")
                        });
                    initial_object_position - attached_point
                }
                LevelObjectMovementType::Linear => Vec2::ZERO,
            };
            Some(LevelObjectMovement {
                frame_started,
                init_vec,
                period: route.period,
                points_progress,
                movement_type,
            })
        });

        if is_invalid {
            continue;
        }

        let needs_updating = match (new_level_object_movement.as_ref(), movement.as_deref()) {
            (None, None) => false,
            (
                Some(LevelObjectMovement {
                    frame_started,
                    init_vec,
                    period,
                    points_progress,
                    movement_type,
                }),
                Some(movement),
            ) => {
                let mut yes = *frame_started != movement.frame_started
                    || *period != movement.period
                    || *init_vec != movement.init_vec
                    || *movement_type != movement.movement_type;

                if !yes && points_progress.len() != movement.points_progress.len() {
                    yes = true;
                }

                if !yes {
                    for (p1, p2) in points_progress.iter().zip(movement.points_progress.iter()) {
                        if p1.entity != p2.entity {
                            yes = true;
                            break;
                        }
                    }
                }

                yes
            }
            _ => true,
        };

        if needs_updating {
            if let Some(new_level_object_movement) = new_level_object_movement {
                commands.entity(entity).insert(new_level_object_movement);
            } else {
                commands.entity(entity).remove::<LevelObjectMovement>();
                position
                    .buffer
                    .insert(time.player_frame, initial_object_position);
            }
        }
    }
}

type LevelObjectsQuerySet<'a, 'b, 'c> = QuerySet<(
    Query<
        'a,
        (
            &'b mut Position,
            Option<&'b mut LevelObjectMovement>,
            &'b Spawned,
        ),
        With<LevelObjectTag>,
    >,
    Query<
        'a,
        (
            Entity,
            &'c Position,
            Option<&'c LevelObjectMovement>,
            &'c Spawned,
        ),
        With<LevelObjectTag>,
    >,
)>;

pub fn process_objects_route_graph(
    time: Res<SimulationTime>,
    mut level_objects: LevelObjectsQuerySet,
) {
    #[cfg(feature = "profiler")]
    puffin::profile_function!();
    let mut new_positions = HashMap::<Entity, (Vec2, Vec<LevelObjectMovementPoint>)>::default();
    let level_objects_readonly = level_objects.q1();
    for (entity, _, _, _) in level_objects_readonly.iter() {
        resolve_new_path_recursive(
            &time,
            vec![entity],
            level_objects_readonly,
            &mut new_positions,
        );
    }

    let level_objects = level_objects.q0_mut();
    for (entity, (new_position, points_progress)) in new_positions {
        let (mut position, movement, _) = level_objects.get_mut(entity).unwrap();
        position.buffer.insert(time.player_frame, new_position);
        if let Some(mut movement) = movement {
            assert_eq!(movement.points_progress.len(), points_progress.len());
            movement.points_progress = points_progress;
        }
    }
}

fn resolve_new_path_recursive(
    time: &Res<SimulationTime>,
    entities_stack: Vec<Entity>,
    level_objects: &Query<
        (Entity, &Position, Option<&LevelObjectMovement>, &Spawned),
        With<LevelObjectTag>,
    >,
    object_updates: &mut HashMap<Entity, (Vec2, Vec<LevelObjectMovementPoint>)>,
) {
    let entity = *entities_stack.last().unwrap();
    let (_entity, _position, movement, spawned): (
        Entity,
        &Position,
        Option<&LevelObjectMovement>,
        &Spawned,
    ) = level_objects.get(entity).unwrap();

    if !spawned.is_spawned(time.player_frame) {
        return;
    }

    if object_updates.contains_key(&entity) {
        return;
    }

    let movement = match movement {
        Some(movement) => movement,
        None => {
            return;
        }
    };

    let dependencies = movement.dependencies();
    for dependency in dependencies {
        let is_spawned = level_objects
            .get_component::<Spawned>(dependency)
            .map_or(false, |spawned| spawned.is_spawned(time.player_frame));
        if entities_stack.contains(&dependency) || !is_spawned {
            return;
        }
        let mut new_stack = entities_stack.clone();
        new_stack.push(dependency);
        resolve_new_path_recursive(time, new_stack, level_objects, object_updates);
    }

    #[allow(clippy::float_cmp)]
    let points_progress = if movement.points_progress.len() == 1 {
        let mut point = movement.points_progress[0].clone();
        assert_eq!(point.progress, 1.0f32);
        point.position = object_updates.get(&point.entity).map_or_else(
            || {
                *level_objects
                    .get_component::<Position>(point.entity)
                    .unwrap()
                    .buffer
                    .get(time.player_frame)
                    .unwrap()
            },
            |(position, _)| *position,
        );
        vec![point]
    } else {
        // This can currently work only under the assumption that only linear movement can have
        // more than 1 route point.
        assert_eq!(movement.movement_type, LevelObjectMovementType::Linear);

        let mut points_progress = Vec::new();
        let mut total_distance = 0.0;
        let mut prev_point_position = movement
            .points_progress
            .first()
            .map_or(Vec2::ZERO, |point| point.position);
        for point in &movement.points_progress {
            let position: Vec2 = object_updates.get(&point.entity).map_or_else(
                || {
                    *level_objects
                        .get_component::<Position>(point.entity)
                        .unwrap()
                        .buffer
                        .get(time.player_frame)
                        .unwrap()
                },
                |(position, _)| *position,
            );
            total_distance += (position - prev_point_position).length();
            prev_point_position = position;

            points_progress.push(LevelObjectMovementPoint {
                progress: 0.0,
                position,
                entity: point.entity,
            });
        }
        let mut current_distance = 0.0;
        let mut prev_point_position = movement
            .points_progress
            .first()
            .map_or(Vec2::ZERO, |point| point.position);
        for i in 1..points_progress.len() - 1 {
            let point = points_progress.get_mut(i).unwrap();
            current_distance += (point.position - prev_point_position).length();
            prev_point_position = point.position;
            point.progress = current_distance / total_distance;
        }
        points_progress.last_mut().unwrap().progress = 1.0;

        points_progress
    };

    let movement = LevelObjectMovement {
        points_progress,
        ..movement.clone()
    };

    object_updates.insert(
        entity,
        (
            movement.current_position(time.player_frame),
            movement.points_progress,
        ),
    );
}

fn closest_start_frame_to_time(
    generation: u64,
    frame_number: FrameNumber,
    start_frame_offset: FrameNumber,
    period: FrameNumber,
) -> FrameNumber {
    if period == FrameNumber::new(0) {
        return frame_number;
    }

    assert!(start_frame_offset.value() < period.value());
    if generation == 0 && frame_number.value() < start_frame_offset.value() {
        return start_frame_offset;
    }

    let mut result = start_frame_offset;
    let runs_per_generation = u16::MAX / period.value();
    for _ in 0..generation {
        let (r, overflown) = result.add(FrameNumber::new(runs_per_generation * period.value()));
        result = if overflown { r } else { r + period }
    }

    if frame_number.value() < result.value() {
        let d = result.value() - frame_number.value();
        let d_runs = d / period.value();
        let d_remainder = d % period.value();
        result -= FrameNumber::new(period.value() * d_runs);
        if d_remainder > 0 {
            result -= period;
        }
    } else {
        let d = frame_number.value() - result.value();
        let d_runs = d / period.value();
        result += FrameNumber::new(period.value() * d_runs);
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_closest_start_frame_to_time_zero_generation_less_than_period() {
        assert_eq!(
            closest_start_frame_to_time(
                0,
                FrameNumber::new(0),
                FrameNumber::new(3),
                FrameNumber::new(10),
            ),
            FrameNumber::new(3)
        );
    }

    #[test]
    fn test_closest_start_frame_to_time_zero_generation_equals_offset() {
        assert_eq!(
            closest_start_frame_to_time(
                0,
                FrameNumber::new(3),
                FrameNumber::new(3),
                FrameNumber::new(10),
            ),
            FrameNumber::new(3)
        );
    }

    #[test]
    fn test_closest_start_frame_to_time_zero_generation_close() {
        assert_eq!(
            closest_start_frame_to_time(
                0,
                FrameNumber::new(15),
                FrameNumber::new(3),
                FrameNumber::new(10),
            ),
            FrameNumber::new(13)
        );
    }

    #[test]
    fn test_closest_start_frame_to_time_zero_generation_several_periods_away() {
        assert_eq!(
            closest_start_frame_to_time(
                0,
                FrameNumber::new(40),
                FrameNumber::new(3),
                FrameNumber::new(10),
            ),
            FrameNumber::new(33)
        );
    }

    #[test]
    fn test_closest_start_frame_to_time_zero_generation_several_periods_away_start() {
        assert_eq!(
            closest_start_frame_to_time(
                0,
                FrameNumber::new(43),
                FrameNumber::new(3),
                FrameNumber::new(10),
            ),
            FrameNumber::new(43)
        );
    }

    #[test]
    fn test_closest_start_frame_to_time_reach_generation_without_overflow() {
        assert_eq!(
            closest_start_frame_to_time(
                1,
                FrameNumber::new(0),
                FrameNumber::new(3),
                FrameNumber::new(10000),
            ),
            FrameNumber::new(60003)
        );
    }

    #[test]
    fn test_closest_start_frame_to_time_reach_generation_with_overflow() {
        assert_eq!(
            closest_start_frame_to_time(
                1,
                FrameNumber::new(5000),
                FrameNumber::new(10000),
                FrameNumber::new(20000),
            ),
            FrameNumber::new(4464)
        );
    }

    #[test]
    fn test_closest_start_frame_to_time_several_generations_away() {
        assert_eq!(
            closest_start_frame_to_time(
                1,
                FrameNumber::new(5000),
                FrameNumber::new(10000),
                FrameNumber::new(20000),
            ),
            FrameNumber::new(4464)
        );
    }
}
