use crate::game::level_objects::*;
#[cfg(feature = "client")]
use crate::{
    client::{assets::MuddleMaterials, *},
    game::components::PredictedPosition,
    GHOST_SIZE_MULTIPLIER, PLAYER_SIZE,
};
use bevy::{
    ecs::system::{EntityCommands, SystemParam},
    math::Vec2,
};
#[cfg(feature = "client")]
use bevy::{
    prelude::*,
    render::{mesh::Indices, pipeline::PrimitiveTopology},
};

pub trait ClientFactory<'a> {
    type Dependencies;
    type Input;

    fn insert_components(
        _commands: &mut EntityCommands,
        _deps: &mut Self::Dependencies,
        _input: Self::Input,
    ) {
    }

    fn remove_components(_commands: &mut EntityCommands, _deps: &mut Self::Dependencies) {}
}

pub struct PlayerClientFactory;

impl<'a> ClientFactory<'a> for PlayerClientFactory {
    type Dependencies = PbrClientParams<'a>;
    type Input = Vec2;

    #[cfg(feature = "client")]
    fn insert_components(
        commands: &mut EntityCommands,
        deps: &mut Self::Dependencies,
        position: Self::Input,
    ) {
        commands.insert_bundle(PbrBundle {
            mesh: deps.meshes.add(Mesh::from(shape::Cube {
                size: PLAYER_SIZE * 2.0,
            })),
            material: deps.materials.player.clone(),
            transform: Transform::from_translation(position.extend(PLAYER_SIZE)),
            ..Default::default()
        });
        commands.insert(PredictedPosition { value: position });
        commands.insert_bundle(bevy_mod_picking::PickableBundle::default());
    }

    #[cfg(feature = "client")]
    fn remove_components(commands: &mut EntityCommands, deps: &mut Self::Dependencies) {
        commands.remove_bundle::<PbrBundle>();
        commands.remove_bundle::<bevy_mod_picking::PickableBundle>();
        let mesh = deps.mesh_query.get(commands.id()).unwrap().clone();
        deps.meshes.remove(mesh);
    }
}

pub struct PlaneClientFactory;

#[derive(Clone)]
pub struct LevelObjectInput<T: Clone> {
    pub desc: T,
    pub is_ghost: bool,
}

impl<'a> ClientFactory<'a> for PlaneClientFactory {
    type Dependencies = PbrClientParams<'a>;
    type Input = (
        LevelObjectInput<PlaneDesc>,
        Option<bevy_rapier2d::rapier::geometry::SharedShape>,
    );

    #[cfg(feature = "client")]
    fn insert_components(
        commands: &mut EntityCommands,
        deps: &mut Self::Dependencies,
        (input, shape): Self::Input,
    ) {
        let ghost_size_multiplier = if input.is_ghost {
            GHOST_SIZE_MULTIPLIER
        } else {
            1.0
        };

        let mesh = match &input.desc.form_desc {
            PlaneFormDesc::Circle { radius } => Mesh::from(XyCircle {
                radius: radius * ghost_size_multiplier,
            }),
            PlaneFormDesc::Rectangle { size } => Mesh::from(XyPlane {
                size: *size * ghost_size_multiplier,
            }),
            PlaneFormDesc::Concave { points: _ } => {
                let shape = shape
                    .as_ref()
                    .expect("Expected a collider shape for a concave plane");

                let mut index = 0;
                let mut indices: Vec<u32> = Vec::new();
                let mut positions: Vec<[f32; 3]> = Vec::new();
                let mut normals: Vec<[f32; 3]> = Vec::new();
                let mut uvs: Vec<[f32; 2]> = Vec::new();
                match shape.as_typed_shape() {
                    bevy_rapier2d::rapier::geometry::TypedShape::Compound(compound) => {
                        for (isometry, shape) in compound.shapes() {
                            match shape.as_typed_shape() {
                                bevy_rapier2d::rapier::geometry::TypedShape::ConvexPolygon(
                                    convex,
                                ) => {
                                    for i in 1..convex.points().len() - 1 {
                                        let i = convex.points().len() - i - 1;
                                        let points = vec![
                                            isometry * convex.points()[convex.points().len() - 1],
                                            isometry * convex.points()[i],
                                            isometry * convex.points()[i - 1],
                                        ];
                                        for point in points {
                                            let position = [
                                                point.x * ghost_size_multiplier,
                                                point.y * ghost_size_multiplier,
                                                0.0,
                                            ];
                                            #[allow(clippy::float_cmp)]
                                            if let Some(existing_index) =
                                                positions.iter().position(|p| *p == position)
                                            {
                                                indices.push(existing_index as u32);
                                            } else {
                                                indices.push(index);
                                                positions.push(position);
                                                normals.push([0.0, 0.0, 1.0]);
                                                uvs.push([0.0, 0.0]);
                                                index += 1;
                                            }
                                        }
                                    }
                                }
                                _ => panic!(
                                    "Unexpected shape type (ConvexPolygon is expected): {:?}",
                                    shape.shape_type()
                                ),
                            };
                        }
                    }
                    _ => panic!(
                        "Unexpected shape type (Compound is expected): {:?}",
                        shape.shape_type()
                    ),
                }

                let mut mesh = Mesh::new(PrimitiveTopology::TriangleList);
                mesh.set_indices(Some(Indices::U32(indices)));
                mesh.set_attribute(Mesh::ATTRIBUTE_POSITION, positions);
                mesh.set_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
                mesh.set_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
                mesh
            }
        };

        commands.insert_bundle(PbrBundle {
            visible: Visible {
                is_visible: if input.is_ghost {
                    deps.visibility_settings.ghosts
                } else {
                    true
                },
                is_transparent: input.is_ghost,
            },
            mesh: deps.meshes.add(mesh),
            material: if input.is_ghost {
                deps.materials.ghost.plane.clone()
            } else {
                deps.materials.normal.plane.clone()
            },
            transform: Transform::from_translation(input.desc.position.extend(0.0)),
            ..Default::default()
        });
        commands.insert_bundle(bevy_mod_picking::PickableBundle::default());
    }

    #[cfg(feature = "client")]
    fn remove_components(commands: &mut EntityCommands, deps: &mut Self::Dependencies) {
        commands.remove_bundle::<PbrBundle>();
        commands.remove_bundle::<bevy_mod_picking::PickableBundle>();
        let mesh = deps.mesh_query.get(commands.id()).unwrap().clone();
        deps.meshes.remove(mesh);
    }
}

pub struct CubeClientFactory;

impl<'a> ClientFactory<'a> for CubeClientFactory {
    type Dependencies = PbrClientParams<'a>;
    type Input = LevelObjectInput<CubeDesc>;

    #[cfg(feature = "client")]
    fn insert_components(
        commands: &mut EntityCommands,
        deps: &mut Self::Dependencies,
        input: Self::Input,
    ) {
        let ghost_size_multiplier = if input.is_ghost {
            GHOST_SIZE_MULTIPLIER
        } else {
            1.0
        };
        commands.insert_bundle(PbrBundle {
            visible: Visible {
                is_visible: if input.is_ghost {
                    deps.visibility_settings.ghosts
                } else {
                    true
                },
                is_transparent: input.is_ghost,
            },
            mesh: deps.meshes.add(Mesh::from(shape::Cube {
                size: input.desc.size * 2.0 * ghost_size_multiplier,
            })),
            material: if input.is_ghost {
                deps.materials.ghost.cube.clone()
            } else {
                deps.materials.normal.cube.clone()
            },
            transform: Transform::from_translation(input.desc.position.extend(input.desc.size)),
            ..Default::default()
        });
        commands.insert_bundle(bevy_mod_picking::PickableBundle::default());
    }

    #[cfg(feature = "client")]
    fn remove_components(commands: &mut EntityCommands, deps: &mut Self::Dependencies) {
        commands.remove_bundle::<PbrBundle>();
        commands.remove_bundle::<bevy_mod_picking::PickableBundle>();
        let mesh = deps.mesh_query.get(commands.id()).unwrap().clone();
        deps.meshes.remove(mesh);
    }
}

pub const ROUTE_POINT_HEIGHT: f32 = 0.8;
pub const ROUTE_POINT_BASE_EDGE_HALF_LEN: f32 = 0.25;

pub struct RoutePointClientFactory;

impl<'a> ClientFactory<'a> for RoutePointClientFactory {
    type Dependencies = PbrClientParams<'a>;
    type Input = LevelObjectInput<RoutePointDesc>;

    #[cfg(feature = "client")]
    fn insert_components(
        commands: &mut EntityCommands,
        deps: &mut Self::Dependencies,
        input: Self::Input,
    ) {
        let ghost_size_multiplier = if input.is_ghost {
            GHOST_SIZE_MULTIPLIER
        } else {
            1.0
        };
        commands.insert_bundle(PbrBundle {
            visible: Visible {
                is_visible: if input.is_ghost {
                    deps.visibility_settings.ghosts
                } else {
                    deps.visibility_settings.route_points
                },
                is_transparent: input.is_ghost,
            },
            mesh: deps.meshes.add(Mesh::from(Pyramid {
                height: ROUTE_POINT_HEIGHT * ghost_size_multiplier,
                base_edge_half_len: ROUTE_POINT_BASE_EDGE_HALF_LEN * ghost_size_multiplier,
            })),
            material: if input.is_ghost {
                deps.materials.ghost.route_point.clone()
            } else {
                deps.materials.normal.route_point.clone()
            },
            ..Default::default()
        });
        commands.insert_bundle(bevy_mod_picking::PickableBundle::default());
    }

    #[cfg(feature = "client")]
    fn remove_components(commands: &mut EntityCommands, deps: &mut Self::Dependencies) {
        commands.remove_bundle::<PbrBundle>();
        commands.remove_bundle::<bevy_mod_picking::PickableBundle>();
        let mesh = deps.mesh_query.get(commands.id()).unwrap().clone();
        deps.meshes.remove(mesh);
    }
}

#[cfg(feature = "client")]
#[derive(Default)]
pub struct VisibilitySettings {
    pub route_points: bool,
    pub ghosts: bool,
}

#[cfg(feature = "client")]
#[derive(SystemParam)]
pub struct PbrClientParams<'a> {
    meshes: ResMut<'a, Assets<Mesh>>,
    materials: Res<'a, MuddleMaterials>,
    visibility_settings: Res<'a, VisibilitySettings>,
    mesh_query: Query<'a, &'static Handle<Mesh>>,
}

#[cfg(not(feature = "client"))]
#[derive(SystemParam)]
pub struct PbrClientParams<'a> {
    #[system_param(ignore)]
    _lifetime: std::marker::PhantomData<&'a ()>,
}
