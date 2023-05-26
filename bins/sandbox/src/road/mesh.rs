use crate::road::RoadModel;
use bevy::{
    prelude::*,
    reflect::Array,
    render::mesh::{Indices, PrimitiveTopology},
};

pub struct RoadMesh {
    pub edge_ring_count: usize,
    pub model: RoadModel,
}

impl From<RoadMesh> for Mesh {
    fn from(value: RoadMesh) -> Self {
        assert!(value.edge_ring_count >= 2);

        let mut positions = Vec::new();
        let mut normals = Vec::new();
        let mut uvs = Vec::new();
        let mut indices = Vec::new();

        let ring_vertices = Road2dMesh::vertices();
        let ring_line_indices = Road2dMesh::line_indices();

        for ring_i in 0..value.edge_ring_count {
            // Vertices.
            let t = ring_i as f32 / (value.edge_ring_count - 1) as f32;
            let bezier_point = value.model.get_bezier_point(t);
            for vertex in &ring_vertices {
                positions.push(
                    *bezier_point
                        .local_to_world_point(vertex.point.extend(0.0))
                        .as_ref(),
                );
                normals.push(
                    *bezier_point
                        .local_to_world_vector(vertex.normal.extend(0.0))
                        .as_ref(),
                );
                uvs.push([0.0, 0.0]);
            }
        }

        for ring_i in 0..value.edge_ring_count - 1 {
            // Triangle indices.
            let root_i = ring_i * ring_vertices.len();
            let root_i_next = (ring_i + 1) * ring_vertices.len();
            for line_i in (0..ring_line_indices.len()).into_iter().step_by(2) {
                let line_index_a = ring_line_indices[line_i];
                let line_index_b = ring_line_indices[line_i + 1];
                let current_a = root_i + line_index_a;
                let current_b = root_i + line_index_b;
                let next_a = root_i_next + line_index_a;
                let next_b = root_i_next + line_index_b;
                indices.push(current_a as u32);
                indices.push(next_a as u32);
                indices.push(next_b as u32);
                indices.push(next_b as u32);
                indices.push(current_b as u32);
                indices.push(current_a as u32);
            }
        }

        let mut mesh = Mesh::new(PrimitiveTopology::TriangleList);
        mesh.set_indices(Some(Indices::U32(indices)));
        mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
        mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
        mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
        mesh
    }
}

pub struct Road2dMesh;

impl Road2dMesh {
    pub fn vertices() -> [RoadVertex; 16] {
        [
            RoadVertex {
                point: Vec2::new(3.0, 0.0),
                normal: Vec2::new(0.0, 1.0),
                u: 0.0,
            },
            RoadVertex {
                point: Vec2::new(3.0, 0.0),
                normal: Vec2::new(-0.70710678118, 0.70710678118),
                u: 0.0,
            },
            RoadVertex {
                point: Vec2::new(4.0, 1.0),
                normal: Vec2::new(-0.70710678118, 0.70710678118),
                u: 0.0,
            },
            RoadVertex {
                point: Vec2::new(4.0, 1.0),
                normal: Vec2::new(0.0, 1.0),
                u: 0.0,
            },
            RoadVertex {
                point: Vec2::new(5.0, 1.0),
                normal: Vec2::new(0.0, 1.0),
                u: 0.0,
            },
            RoadVertex {
                point: Vec2::new(5.0, 1.0),
                normal: Vec2::new(1.0, 0.0),
                u: 0.0,
            },
            RoadVertex {
                point: Vec2::new(5.0, -1.0),
                normal: Vec2::new(1.0, 0.0),
                u: 0.0,
            },
            RoadVertex {
                point: Vec2::new(5.0, -1.0),
                normal: Vec2::new(0.0, -1.0),
                u: 0.0,
            },
            RoadVertex {
                point: Vec2::new(-5.0, -1.0),
                normal: Vec2::new(0.0, -1.0),
                u: 0.0,
            },
            RoadVertex {
                point: Vec2::new(-5.0, -1.0),
                normal: Vec2::new(-1.0, 0.0),
                u: 0.0,
            },
            RoadVertex {
                point: Vec2::new(-5.0, 1.0),
                normal: Vec2::new(-1.0, 0.0),
                u: 0.0,
            },
            RoadVertex {
                point: Vec2::new(-5.0, 1.0),
                normal: Vec2::new(0.0, 1.0),
                u: 0.0,
            },
            RoadVertex {
                point: Vec2::new(-4.0, 1.0),
                normal: Vec2::new(0.0, 1.0),
                u: 0.0,
            },
            RoadVertex {
                point: Vec2::new(-4.0, 1.0),
                normal: Vec2::new(0.70710678118, 0.70710678118),
                u: 0.0,
            },
            RoadVertex {
                point: Vec2::new(-3.0, 0.0),
                normal: Vec2::new(-0.70710678118, 0.70710678118),
                u: 0.0,
            },
            RoadVertex {
                point: Vec2::new(-3.0, 0.0),
                normal: Vec2::new(0.0, 1.0),
                u: 0.0,
            },
        ]
    }

    #[rustfmt::skip]
    pub fn line_indices() -> [usize; 16] {
        [
            15, 0,
            1, 2,
            3, 4,
            5, 6,
            7, 8,
            9, 10,
            11, 12,
            13, 14,
        ]
    }
}

pub struct RoadVertex {
    pub point: Vec2,
    pub normal: Vec2,
    pub u: f32,
}
