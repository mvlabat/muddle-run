use bevy::math::Vec2;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct PlaneDesc {
    pub size: f32,
    pub position: Vec2,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct CubeDesc {
    pub size: f32,
    pub position: Vec2,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct PivotPointDesc {
    pub position: Vec2,
}
