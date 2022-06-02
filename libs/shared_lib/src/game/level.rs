use crate::{
    collider_flags::level_object_collision_groups,
    framebuffer::FrameNumber,
    game::{
        client_factories::ROUTE_POINT_BASE_EDGE_HALF_LEN,
        components::{LevelObjectTag, PhysicsBundle},
        level_objects::*,
        spawn::ColliderShapeSender,
    },
    messages::EntityNetId,
    registry::EntityRegistry,
};
use bevy::{
    ecs::{
        entity::Entity,
        query::Added,
        system::{Query, Res, ResMut, SystemParam},
    },
    math::Vec2,
    tasks::AsyncComputeTaskPool,
    utils::HashMap,
};
use bevy_rapier2d::{
    dynamics::{LockedAxes, RigidBody},
    geometry::{Sensor, VHACDParameters},
    na::Point2,
    rapier::geometry::ColliderShape,
};
use serde::{Deserialize, Serialize};
use std::marker::PhantomData;

#[derive(SystemParam)]
pub struct LevelParams<'w, 's> {
    pub level_state: Res<'w, LevelState>,
    pub entity_registry: Res<'w, EntityRegistry<EntityNetId>>,
    #[system_param(ignore)]
    marker: PhantomData<&'s ()>,
}

impl<'w, 's> LevelParams<'w, 's> {
    pub fn level_object_by_entity(&self, entity: Entity) -> Option<&LevelObject> {
        self.entity_registry
            .get_id(entity)
            .and_then(|net_id| self.level_object_by_net_id(net_id))
    }

    pub fn level_object_by_net_id(&self, entity_net_id: EntityNetId) -> Option<&LevelObject> {
        self.level_state.objects.get(&entity_net_id)
    }
}

#[derive(Default)]
pub struct LevelState {
    pub objects: HashMap<EntityNetId, LevelObject>,
    pub spawn_areas: Vec<EntityNetId>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct LevelObject {
    pub net_id: EntityNetId,
    pub label: String,
    pub desc: LevelObjectDesc,
    /// Absence of this field means that an object is stationary.
    pub route: Option<ObjectRoute>,
    pub collision_logic: CollisionLogic,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct ObjectRoute {
    pub period: FrameNumber,
    pub start_frame_offset: FrameNumber,
    pub desc: ObjectRouteDesc,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub enum ObjectRouteDesc {
    Attached(Option<EntityNetId>),
    Radial(Option<EntityNetId>),
    ForwardCycle(Vec<EntityNetId>),
    ForwardBackwardsCycle(Vec<EntityNetId>),
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum LevelObjectDesc {
    Plane(PlaneDesc),
    Cube(CubeDesc),
    RoutePoint(RoutePointDesc),
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum CollisionLogic {
    Finish,
    Death,
    None,
}

pub enum ColliderShapeResponse {
    Immediate(ColliderShape),
    Promise,
}

impl LevelObjectDesc {
    pub fn label(&self) -> String {
        match self {
            Self::Plane(_) => "Plane",
            Self::Cube(_) => "Cube",
            Self::RoutePoint(_) => "Route Point",
        }
        .to_owned()
    }

    pub fn is_movable_with_mouse(&self) -> bool {
        !matches!(self, Self::Plane(_))
    }

    pub fn position(&self) -> Option<Vec2> {
        match self {
            Self::Plane(plane) => Some(plane.position),
            Self::Cube(cube) => Some(cube.position),
            Self::RoutePoint(route_point) => Some(route_point.position),
        }
    }

    pub fn position_mut(&mut self) -> Option<&mut Vec2> {
        match self {
            Self::Plane(plane) => Some(&mut plane.position),
            Self::Cube(cube) => Some(&mut cube.position),
            Self::RoutePoint(route_point) => Some(&mut route_point.position),
        }
    }

    pub fn calculate_collider_shape(
        &self,
        task_pool: &AsyncComputeTaskPool,
        entity: Entity,
        collider_shape_sender: ColliderShapeSender,
    ) -> ColliderShapeResponse {
        ColliderShapeResponse::Immediate(match self {
            Self::Plane(plane) => match &plane.form_desc {
                PlaneFormDesc::Circle { radius } => ColliderShape::ball(*radius),
                PlaneFormDesc::Rectangle { size } => {
                    let hsize = *size / 2.0;
                    ColliderShape::cuboid(hsize.x, hsize.y)
                }
                PlaneFormDesc::Concave { points } => {
                    assert!(points.len() > 2);
                    let vertices = points
                        .iter()
                        .enumerate()
                        .filter_map(|(i, point)| {
                            if i > 0 && points[i - 1] == *point {
                                None
                            } else {
                                Some(Point2::new(point.x, point.y))
                            }
                        })
                        .collect::<Vec<_>>();
                    let mut indices = (0..vertices.len() - 1)
                        .map(|i| [i as u32, i as u32 + 1])
                        .collect::<Vec<_>>();
                    indices.push([indices.last().unwrap()[1], 0]);
                    task_pool
                        .spawn(async move {
                            let r = std::panic::catch_unwind(|| {
                                ColliderShape::convex_decomposition_with_params(
                                    &vertices,
                                    &indices,
                                    &VHACDParameters {
                                        concavity: 0.01,
                                        resolution: 64,
                                        ..Default::default()
                                    },
                                )
                            })
                            .ok();
                            collider_shape_sender.send((entity, r)).unwrap();
                        })
                        .detach();
                    return ColliderShapeResponse::Promise;
                }
            },
            Self::Cube(cube) => ColliderShape::cuboid(cube.size, cube.size),
            Self::RoutePoint(_) => ColliderShape::cuboid(
                ROUTE_POINT_BASE_EDGE_HALF_LEN * 2.0,
                ROUTE_POINT_BASE_EDGE_HALF_LEN * 2.0,
            ),
        })
    }

    pub fn physics_bundle(&self, shape: ColliderShape, server_simulated: bool) -> PhysicsBundle {
        match self {
            Self::Plane(_) | Self::RoutePoint(_) => PhysicsBundle {
                rigid_body: RigidBody::KinematicPositionBased,
                collider: shape.into(),
                sensor: Sensor(true),
                collision_groups: level_object_collision_groups(server_simulated),
                locked_axes: LockedAxes::TRANSLATION_LOCKED_Z,
            },
            Self::Cube(_) => PhysicsBundle {
                rigid_body: RigidBody::KinematicPositionBased,
                collider: shape.into(),
                sensor: Sensor(false),
                collision_groups: level_object_collision_groups(server_simulated),
                locked_axes: LockedAxes::TRANSLATION_LOCKED_Z,
            },
        }
    }

    pub fn control_points(&self) -> Vec<Vec2> {
        match self {
            Self::Plane(PlaneDesc {
                form_desc: PlaneFormDesc::Concave { points },
                ..
            }) => points.clone(),
            _ => Vec::new(),
        }
    }

    pub fn set_control_point(&mut self, index: usize, point: Vec2) {
        match self {
            Self::Plane(PlaneDesc {
                form_desc: PlaneFormDesc::Concave { ref mut points },
                ..
            }) => {
                points[index] = point;
            }
            _ => unimplemented!(),
        }
    }

    pub fn possible_collision_logic(&self) -> Vec<CollisionLogic> {
        // `CollisionLogic::None` is implied by default.
        match self {
            Self::Plane(_) => vec![CollisionLogic::Finish, CollisionLogic::Death],
            Self::Cube(_) => vec![CollisionLogic::Death],
            Self::RoutePoint(_) => vec![],
        }
    }
}

pub fn maintain_available_spawn_areas(
    mut level_state: ResMut<LevelState>,
    updated_level_objects: Query<&EntityNetId, Added<LevelObjectTag>>,
) {
    for level_object_net_id in updated_level_objects.iter().copied() {
        let is_spawn_area =
            level_state
                .objects
                .get(&level_object_net_id)
                .map_or(false, |level_object| match &level_object.desc {
                    LevelObjectDesc::Plane(plane_desc) => plane_desc.is_spawn_area,
                    _ => false,
                });

        if let Some(i) = level_state
            .spawn_areas
            .iter()
            .position(|net_id| *net_id == level_object_net_id)
        {
            if !is_spawn_area {
                level_state.spawn_areas.remove(i);
            }
        } else if is_spawn_area {
            level_state.spawn_areas.push(level_object_net_id);
        }
    }
}
