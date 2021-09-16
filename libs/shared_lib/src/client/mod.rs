pub mod assets;

use crate::game::components::rotate;
use bevy::{
    math::{Vec2, Vec3},
    render::{
        mesh::{Indices, Mesh},
        pipeline::PrimitiveTopology,
    },
};

/// A circle on the XZ plane.
#[derive(Debug, Copy, Clone)]
pub struct XyCircle {
    pub radius: f32,
}

impl Default for XyCircle {
    fn default() -> Self {
        Self { radius: 1.0 }
    }
}

impl From<XyCircle> for Mesh {
    fn from(plane: XyCircle) -> Self {
        let segments = plane.optimal_segments_count();
        let radius = Vec2::new(plane.radius, 0.0);

        let mut positions = vec![[0.0, 0.0, 0.0]];
        let mut indices = Vec::new();
        for i in 0..segments {
            positions.push(
                rotate(
                    radius,
                    2.0 * std::f32::consts::PI / segments as f32 * i as f32,
                )
                .extend(0.0)
                .into(),
            );
            indices.push(0u32);
            indices.push(i + 1);
            indices.push((i + 1) % segments + 1);
        }
        let normals = positions
            .iter()
            .map(|_| [0.0, 0.0, 1.0])
            .collect::<Vec<_>>();
        let uvs = positions.iter().map(|_| [0.0, 0.0]).collect::<Vec<_>>();

        let indices = Indices::U32(indices);

        let mut mesh = Mesh::new(PrimitiveTopology::TriangleList);
        mesh.set_indices(Some(indices));
        mesh.set_attribute(Mesh::ATTRIBUTE_POSITION, positions);
        mesh.set_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
        mesh.set_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
        mesh
    }
}

impl XyCircle {
    fn optimal_segments_count(&self) -> u32 {
        (self.radius.sqrt() * 24.0) as u32
    }
}

/// A rectangle on the XZ plane.
#[derive(Debug, Copy, Clone)]
pub struct XyPlane {
    /// The total side length of the square.
    pub size: Vec2,
}

impl Default for XyPlane {
    fn default() -> Self {
        Self {
            size: Vec2::new(1.0, 1.0),
        }
    }
}

impl From<XyPlane> for Mesh {
    fn from(plane: XyPlane) -> Self {
        let extent = plane.size / 2.0;

        let vertices = [
            ([extent.x, -extent.y, 0.0], [0.0, 0.0, 1.0], [1.0, 1.0]),
            ([extent.x, extent.y, 0.0], [0.0, 0.0, 1.0], [1.0, 0.0]),
            ([-extent.x, extent.y, 0.0], [0.0, 0.0, 1.0], [0.0, 0.0]),
            ([-extent.x, -extent.y, 0.0], [0.0, 0.0, 1.0], [0.0, 1.0]),
        ];

        let indices = Indices::U32(vec![0, 1, 2, 0, 2, 3]);

        let mut positions = Vec::new();
        let mut normals = Vec::new();
        let mut uvs = Vec::new();
        for (position, normal, uv) in vertices.iter() {
            positions.push(*position);
            normals.push(*normal);
            uvs.push(*uv);
        }

        let mut mesh = Mesh::new(PrimitiveTopology::TriangleList);
        mesh.set_indices(Some(indices));
        mesh.set_attribute(Mesh::ATTRIBUTE_POSITION, positions);
        mesh.set_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
        mesh.set_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
        mesh
    }
}

#[derive(Copy, Clone)]
pub struct Pyramid {
    pub height: f32,
    pub base_edge_half_len: f32,
}

impl Pyramid {
    pub fn positions(&self) -> Vec<Vec3> {
        let Self {
            height,
            base_edge_half_len,
        } = *self;
        vec![
            // Bottom.
            Vec3::new(-base_edge_half_len, -base_edge_half_len, 0.0),
            Vec3::new(-base_edge_half_len, base_edge_half_len, 0.0),
            Vec3::new(base_edge_half_len, base_edge_half_len, 0.0),
            Vec3::new(base_edge_half_len, -base_edge_half_len, 0.0),
            // West.
            Vec3::new(-base_edge_half_len, base_edge_half_len, 0.0),
            Vec3::new(-base_edge_half_len, -base_edge_half_len, 0.0),
            Vec3::new(0.0, 0.0, height),
            // North.
            Vec3::new(base_edge_half_len, base_edge_half_len, 0.0),
            Vec3::new(-base_edge_half_len, base_edge_half_len, 0.0),
            Vec3::new(0.0, 0.0, height),
            // East.
            Vec3::new(base_edge_half_len, -base_edge_half_len, 0.0),
            Vec3::new(base_edge_half_len, base_edge_half_len, 0.0),
            Vec3::new(0.0, 0.0, height),
            // South.
            Vec3::new(-base_edge_half_len, -base_edge_half_len, 0.0),
            Vec3::new(base_edge_half_len, -base_edge_half_len, 0.0),
            Vec3::new(0.0, 0.0, height),
        ]
    }

    #[rustfmt::skip]
    pub fn indices(&self) -> Vec<u32> {
        vec![
            // Bottom 1.
            0, 1, 3,
            // Bottom 2.
            1, 2, 3,
            // West.
            4, 5, 6,
            // North.
            7, 8, 9,
            // East.
            10, 11, 12,
            // South.
            13, 14, 15,
        ]
    }
}

impl From<Pyramid> for Mesh {
    fn from(pyramid: Pyramid) -> Self {
        let positions = pyramid.positions();
        let indices = pyramid.indices();

        let west_norm = (positions[0] - positions[1]).cross(positions[6] - positions[0]);
        let north_norm = (positions[1] - positions[2]).cross(positions[6] - positions[1]);
        let east_norm = (positions[2] - positions[3]).cross(positions[6] - positions[2]);
        let south_norm = (positions[3] - positions[0]).cross(positions[6] - positions[3]);
        let normals = vec![
            Vec3::new(0.0, 0.0, -1.0),
            Vec3::new(0.0, 0.0, -1.0),
            Vec3::new(0.0, 0.0, -1.0),
            Vec3::new(0.0, 0.0, -1.0),
            west_norm,
            west_norm,
            west_norm,
            north_norm,
            north_norm,
            north_norm,
            east_norm,
            east_norm,
            east_norm,
            south_norm,
            south_norm,
            south_norm,
        ];

        let mut mesh = Mesh::new(PrimitiveTopology::TriangleList);
        mesh.set_indices(Some(Indices::U32(indices)));
        mesh.set_attribute(
            Mesh::ATTRIBUTE_POSITION,
            positions.iter().map(|p| *p.as_ref()).collect::<Vec<_>>(),
        );
        mesh.set_attribute(
            Mesh::ATTRIBUTE_NORMAL,
            normals.iter().map(|p| *p.as_ref()).collect::<Vec<_>>(),
        );
        mesh.set_attribute(Mesh::ATTRIBUTE_UV_0, vec![[0.0, 0.0]; 16]);
        mesh
    }
}
