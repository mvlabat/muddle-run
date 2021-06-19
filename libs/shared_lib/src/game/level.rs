use crate::{game::level_objects::*, messages::EntityNetId};
use bevy::math::Vec2;
use bevy_rapier3d::{
    physics::{ColliderBundle, RigidBodyBundle},
    rapier::{
        dynamics::RigidBodyType,
        geometry::{ColliderShape, ColliderType},
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
}

impl LevelObjectDesc {
    pub fn label(&self) -> String {
        match self {
            Self::Plane(_) => "Plane",
            Self::Cube(_) => "Cube",
        }
        .to_owned()
    }

    pub fn position(&self) -> Vec2 {
        match self {
            Self::Plane(plane) => plane.position,
            Self::Cube(cube) => cube.position,
        }
    }

    pub fn position_mut(&mut self) -> &mut Vec2 {
        match self {
            Self::Plane(plane) => &mut plane.position,
            Self::Cube(cube) => &mut cube.position,
        }
    }

    pub fn physics_body(&self) -> (RigidBodyBundle, ColliderBundle) {
        match self {
            Self::Plane(plane) => (
                RigidBodyBundle {
                    body_type: RigidBodyType::KinematicPositionBased,
                    position: [self.position().x, self.position().y, 0.0].into(),
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
                    position: [self.position().x, self.position().y, cube.size].into(),
                    ..RigidBodyBundle::default()
                },
                ColliderBundle {
                    shape: ColliderShape::cuboid(cube.size, cube.size, cube.size),
                    ..ColliderBundle::default()
                },
            ),
        }
    }
}
