pub mod mesh;
pub mod systems;

use bevy::prelude::*;

#[derive(Resource, Deref, DerefMut)]
pub struct SelectedRoad(pub Entity);

#[derive(Component, Clone)]
pub struct Road {
    pub edge_ring_count: usize,
}

#[derive(Component, Clone)]
pub struct ControlPointIndex(pub usize);

#[derive(Component)]
pub struct VisualiseT {
    pub value: f32,
}

pub struct OrientedPoint {
    pub pos: Vec3,
    pub rot: Quat,
}

impl OrientedPoint {
    pub fn new(pos: Vec3, rot: Quat) -> Self {
        Self { pos, rot }
    }

    pub fn from_forward(pos: Vec3, forward: Vec3) -> Self {
        let up = Vec3::Y;
        let right = up.cross(forward).normalize();
        let up = forward.cross(right);
        let rot = Quat::from_mat3(&Mat3::from_cols(right, up, forward));
        Self::new(pos, rot)
    }

    pub fn local_to_world_point(&self, local: Vec3) -> Vec3 {
        self.pos + self.rot * local
    }

    pub fn local_to_world_vector(&self, local: Vec3) -> Vec3 {
        self.rot * local
    }
}

pub struct RoadModel {
    pub control_points: Vec<Vec3>,
}

impl RoadModel {
    fn get_bezier_point(&self, t: f32) -> OrientedPoint {
        assert!(self.control_points.len() > 1);

        let mut control_points = self.control_points.clone();

        while control_points.len() > 2 {
            let mut new_control_points = Vec::with_capacity(control_points.len() - 1);
            for i in 1..control_points.len() {
                new_control_points.push(control_points[i - 1].lerp(control_points[i], t));
            }
            control_points = new_control_points;
        }

        let pos = control_points[0].lerp(control_points[1], t);
        let tangent = (control_points[1] - control_points[0]).normalize_or_zero();

        OrientedPoint::from_forward(pos, tangent)
    }
}
