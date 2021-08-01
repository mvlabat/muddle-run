use crate::{
    framebuffer::FrameNumber,
    game::{client_factories::ROUTE_POINT_BASE_EDGE_HALF_LEN, level_objects::*},
    messages::EntityNetId,
    registry::EntityRegistry,
    GHOST_SIZE_MULTIPLIER,
};
use bevy::{
    ecs::{
        entity::Entity,
        system::{Res, SystemParam},
    },
    math::Vec2,
    utils::HashMap,
};
use bevy_rapier2d::{
    physics::{ColliderBundle, RigidBodyBundle},
    rapier::{
        dynamics::RigidBodyType,
        geometry::{ColliderShape, ColliderType},
    },
};
use serde::{Deserialize, Serialize};

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

    pub fn physics_body(&self, is_ghost: bool) -> (RigidBodyBundle, ColliderBundle) {
        let ghost_multiplier = if is_ghost { GHOST_SIZE_MULTIPLIER } else { 1.0 };
        match self {
            Self::Plane(plane) => (
                RigidBodyBundle {
                    body_type: RigidBodyType::KinematicPositionBased,
                    position: self.position().unwrap().into(),
                    ..RigidBodyBundle::default()
                },
                ColliderBundle {
                    collider_type: ColliderType::Sensor,
                    shape: ColliderShape::cuboid(
                        plane.size * ghost_multiplier,
                        plane.size * ghost_multiplier,
                    ),
                    ..ColliderBundle::default()
                },
            ),
            Self::Cube(cube) => (
                RigidBodyBundle {
                    body_type: RigidBodyType::KinematicPositionBased,
                    position: [self.position().unwrap().x, self.position().unwrap().y].into(),
                    ..RigidBodyBundle::default()
                },
                ColliderBundle {
                    collider_type: if is_ghost {
                        ColliderType::Sensor
                    } else {
                        ColliderType::Solid
                    },
                    shape: ColliderShape::cuboid(
                        cube.size * ghost_multiplier,
                        cube.size * ghost_multiplier,
                    ),
                    ..ColliderBundle::default()
                },
            ),
            Self::RoutePoint(_) => (
                RigidBodyBundle {
                    body_type: RigidBodyType::KinematicPositionBased,
                    position: [self.position().unwrap().x, self.position().unwrap().y].into(),
                    ..RigidBodyBundle::default()
                },
                ColliderBundle {
                    collider_type: ColliderType::Sensor,
                    shape: ColliderShape::cuboid(
                        ROUTE_POINT_BASE_EDGE_HALF_LEN * 2.0 * ghost_multiplier,
                        ROUTE_POINT_BASE_EDGE_HALF_LEN * 2.0 * ghost_multiplier,
                    ),
                    ..ColliderBundle::default()
                },
            ),
        }
    }
}
