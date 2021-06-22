use crate::{
    framebuffer::FrameNumber,
    game::{
        client_factories::{ROUTE_POINT_BASE_EDGE_HALF_LEN, ROUTE_POINT_HEIGHT},
        level_objects::*,
    },
    messages::EntityNetId,
    registry::EntityRegistry,
};
use bevy::{
    ecs::{
        entity::Entity,
        system::{Res, SystemParam},
    },
    math::Vec2,
};
use bevy_rapier3d::{
    physics::{ColliderBundle, RigidBodyBundle},
    rapier::{
        dynamics::RigidBodyType,
        geometry::{ColliderShape, ColliderType},
        math::Point,
    },
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(SystemParam)]
pub struct LevelParams<'a> {
    pub level_state: Res<'a, LevelState>,
    pub entity_registry: Res<'a, EntityRegistry<EntityNetId>>,
}

impl<'a> LevelParams<'a> {
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
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct LevelObject {
    pub net_id: EntityNetId,
    pub label: String,
    pub desc: LevelObjectDesc,
    /// Absence of this field means that an object is stationary.
    pub route: Option<ObjectRoute>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ObjectRoute {
    pub period: FrameNumber,
    pub start_frame_offset: FrameNumber,
    pub desc: ObjectRouteDesc,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
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

impl LevelObjectDesc {
    pub fn label(&self) -> String {
        match self {
            Self::Plane(_) => "Plane",
            Self::Cube(_) => "Cube",
            Self::RoutePoint(_) => "Route Point",
        }
        .to_owned()
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

    pub fn physics_body(&self) -> (RigidBodyBundle, ColliderBundle) {
        match self {
            Self::Plane(plane) => (
                RigidBodyBundle {
                    body_type: RigidBodyType::KinematicPositionBased,
                    position: [self.position().unwrap().x, self.position().unwrap().y, 0.0].into(),
                    ..RigidBodyBundle::default()
                },
                ColliderBundle {
                    collider_type: ColliderType::Sensor,
                    shape: ColliderShape::cuboid(plane.size, plane.size, 0.01),
                    ..ColliderBundle::default()
                },
            ),
            Self::Cube(cube) => (
                RigidBodyBundle {
                    body_type: RigidBodyType::KinematicPositionBased,
                    position: [
                        self.position().unwrap().x,
                        self.position().unwrap().y,
                        cube.size,
                    ]
                    .into(),
                    ..RigidBodyBundle::default()
                },
                ColliderBundle {
                    shape: ColliderShape::cuboid(cube.size, cube.size, cube.size),
                    ..ColliderBundle::default()
                },
            ),
            Self::RoutePoint(_) => (
                RigidBodyBundle {
                    body_type: RigidBodyType::KinematicPositionBased,
                    position: [self.position().unwrap().x, self.position().unwrap().y, 0.0].into(),
                    ..RigidBodyBundle::default()
                },
                ColliderBundle {
                    collider_type: ColliderType::Sensor,
                    shape: ColliderShape::convex_hull(&[
                        Point::new(
                            -ROUTE_POINT_BASE_EDGE_HALF_LEN,
                            -ROUTE_POINT_BASE_EDGE_HALF_LEN,
                            0.0,
                        ),
                        Point::new(
                            -ROUTE_POINT_BASE_EDGE_HALF_LEN,
                            ROUTE_POINT_BASE_EDGE_HALF_LEN,
                            0.0,
                        ),
                        Point::new(
                            ROUTE_POINT_BASE_EDGE_HALF_LEN,
                            ROUTE_POINT_BASE_EDGE_HALF_LEN,
                            0.0,
                        ),
                        Point::new(
                            ROUTE_POINT_BASE_EDGE_HALF_LEN,
                            -ROUTE_POINT_BASE_EDGE_HALF_LEN,
                            0.0,
                        ),
                        Point::new(0.0, 0.0, ROUTE_POINT_HEIGHT),
                    ])
                    .unwrap(),
                    ..ColliderBundle::default()
                },
            ),
        }
    }
}
