use crate::{
    game::{
        client_factories::{PIVOT_POINT_BASE_EDGE_HALF_LEN, PIVOT_POINT_HEIGHT},
        level_objects::*,
    },
    messages::EntityNetId,
};
use bevy::math::Vec2;
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

#[derive(Default)]
pub struct LevelState {
    pub objects: HashMap<EntityNetId, LevelObject>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct LevelObject {
    pub net_id: EntityNetId,
    pub label: String,
    pub desc: LevelObjectDesc,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum LevelObjectDesc {
    Plane(PlaneDesc),
    Cube(CubeDesc),
    PivotPoint(PivotPointDesc),
}

impl LevelObjectDesc {
    pub fn label(&self) -> String {
        match self {
            Self::Plane(_) => "Plane",
            Self::Cube(_) => "Cube",
            Self::PivotPoint(_) => "Pivot Point",
        }
        .to_owned()
    }

    pub fn position(&self) -> Option<Vec2> {
        match self {
            Self::Plane(plane) => Some(plane.position),
            Self::Cube(cube) => Some(cube.position),
            Self::PivotPoint(pivot_point) => Some(pivot_point.position),
        }
    }

    pub fn position_mut(&mut self) -> Option<&mut Vec2> {
        match self {
            Self::Plane(plane) => Some(&mut plane.position),
            Self::Cube(cube) => Some(&mut cube.position),
            Self::PivotPoint(pivot_point) => Some(&mut pivot_point.position),
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
            Self::PivotPoint(_) => (
                RigidBodyBundle {
                    body_type: RigidBodyType::KinematicPositionBased,
                    position: [self.position().unwrap().x, self.position().unwrap().y, 0.0].into(),
                    ..RigidBodyBundle::default()
                },
                ColliderBundle {
                    collider_type: ColliderType::Sensor,
                    shape: ColliderShape::convex_hull(&[
                        Point::new(
                            -PIVOT_POINT_BASE_EDGE_HALF_LEN,
                            -PIVOT_POINT_BASE_EDGE_HALF_LEN,
                            0.0,
                        ),
                        Point::new(
                            -PIVOT_POINT_BASE_EDGE_HALF_LEN,
                            PIVOT_POINT_BASE_EDGE_HALF_LEN,
                            0.0,
                        ),
                        Point::new(
                            PIVOT_POINT_BASE_EDGE_HALF_LEN,
                            PIVOT_POINT_BASE_EDGE_HALF_LEN,
                            0.0,
                        ),
                        Point::new(
                            PIVOT_POINT_BASE_EDGE_HALF_LEN,
                            -PIVOT_POINT_BASE_EDGE_HALF_LEN,
                            0.0,
                        ),
                        Point::new(0.0, 0.0, PIVOT_POINT_HEIGHT),
                    ])
                    .unwrap(),
                    ..ColliderBundle::default()
                },
            ),
        }
    }
}
