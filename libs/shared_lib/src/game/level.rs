use crate::{game::level_objects::*, messages::EntityNetId};
use bevy::math::Vec2;
use bevy_rapier3d::rapier::{dynamics::RigidBodyBuilder, geometry::ColliderBuilder};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Default)]
pub struct LevelState {
    pub objects: HashMap<EntityNetId, LevelObject>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct LevelObject {
    pub net_id: EntityNetId,
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

    pub fn physics_body(&self) -> (RigidBodyBuilder, ColliderBuilder) {
        match self {
            Self::Plane(plane) => (
                RigidBodyBuilder::new_kinematic().translation(
                    self.position().x,
                    0.0,
                    self.position().y,
                ),
                ColliderBuilder::cuboid(plane.size, 0.01, plane.size).sensor(true),
            ),
            Self::Cube(cube) => (
                RigidBodyBuilder::new_kinematic().translation(
                    self.position().x,
                    cube.size,
                    self.position().y,
                ),
                ColliderBuilder::cuboid(cube.size, cube.size, cube.size),
            ),
        }
    }
}
