use crate::game::{level::CollisionLogic, level_objects::*};
#[cfg(feature = "client")]
use crate::{
    client::{assets::MuddleAssets, components::DebugUiVisibility, *},
    game::components::PredictedPosition,
    GHOST_SIZE_MULTIPLIER, PLAYER_RADIUS,
};
use bevy::{
    ecs::system::{EntityCommands, SystemParam},
    math::Vec2,
};
#[cfg(feature = "client")]
use bevy::{
    prelude::*,
    render::{mesh::Indices, render_resource::PrimitiveTopology},
};

pub fn object_height(collision_logic: CollisionLogic) -> f32 {
    match collision_logic {
        CollisionLogic::None => 0.0,
        CollisionLogic::Finish => 0.001,
        CollisionLogic::Death => 0.002,
    }
}

pub trait ClientFactory<'w, 's> {
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

impl<'w, 's> ClientFactory<'w, 's> for PlayerClientFactory {
    type Dependencies = PbrClientParams<'w, 's>;
    type Input = Vec2;

    #[cfg(feature = "client")]
    fn insert_components(
        commands: &mut EntityCommands,
        deps: &mut Self::Dependencies,
        position: Self::Input,
    ) {
        commands.insert(PbrBundle {
            mesh: deps.meshes.add(Mesh::from(XyCircle {
                radius: PLAYER_RADIUS,
            })),
            material: deps.assets.materials.player.clone(),
            transform: Transform::from_translation(position.extend(0.01)),
            ..Default::default()
        });
        commands.insert(PredictedPosition { value: position });
        commands.insert(bevy_mod_picking::PickableBundle::default());
    }

    #[cfg(feature = "client")]
    fn remove_components(commands: &mut EntityCommands, deps: &mut Self::Dependencies) {
        commands.remove::<PbrBundle>();
        commands.remove::<bevy_mod_picking::PickableBundle>();
        commands.remove::<PredictedPosition>();
        let mesh = deps.mesh_query.get(commands.id()).unwrap().clone();
        deps.meshes.remove(mesh);
    }
}

pub struct PlayerSensorClientFactory;

impl<'w, 's> ClientFactory<'w, 's> for PlayerSensorClientFactory {
    type Dependencies = PbrClientParams<'w, 's>;
    type Input = ();

    #[cfg(feature = "client")]
    fn insert_components(
        commands: &mut EntityCommands,
        deps: &mut Self::Dependencies,
        (): Self::Input,
    ) {
        commands
            .insert(PbrBundle {
                mesh: deps.assets.meshes.player_sensor.clone(),
                material: deps.assets.materials.player_sensor_normal.clone(),
                visibility: Visibility {
                    is_visible: deps.visibility_settings.debug,
                },
                ..Default::default()
            })
            .insert(DebugUiVisibility);
    }

    #[cfg(feature = "client")]
    fn remove_components(commands: &mut EntityCommands, _deps: &mut Self::Dependencies) {
        commands.remove::<PbrBundle>().remove::<DebugUiVisibility>();
    }
}

pub struct PlaneClientFactory;

#[derive(Clone)]
pub struct LevelObjectInput<T: Clone> {
    pub desc: T,
    pub collision_logic: CollisionLogic,
    pub is_ghost: bool,
}

impl<'w, 's> ClientFactory<'w, 's> for PlaneClientFactory {
    type Dependencies = PbrClientParams<'w, 's>;
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
                                            isometry * convex.points()[i - 1],
                                            isometry * convex.points()[i],
                                            isometry * convex.points()[convex.points().len() - 1],
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
                mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
                mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
                mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
                mesh
            }
        };

        commands.insert(PbrBundle {
            visibility: Visibility {
                is_visible: if input.is_ghost {
                    deps.visibility_settings.ghosts
                } else {
                    true
                },
            },
            mesh: deps.meshes.add(mesh),
            material: {
                let materials = if input.is_ghost {
                    &deps.assets.materials.ghost
                } else {
                    &deps.assets.materials.normal
                };
                match input.collision_logic {
                    CollisionLogic::Finish => materials.plane_finish.clone(),
                    CollisionLogic::Death => materials.plane_death.clone(),
                    CollisionLogic::None => materials.plane.clone(),
                }
            },
            transform: Transform::from_translation(
                input
                    .desc
                    .position
                    .extend(object_height(input.collision_logic)),
            ),
            ..Default::default()
        });
        commands.insert(bevy_mod_picking::PickableBundle::default());
    }

    #[cfg(feature = "client")]
    fn remove_components(commands: &mut EntityCommands, deps: &mut Self::Dependencies) {
        commands.remove::<PbrBundle>();
        commands.remove::<bevy_mod_picking::PickableBundle>();
        let mesh = deps.mesh_query.get(commands.id()).unwrap().clone();
        deps.meshes.remove(mesh);
    }
}

pub struct CubeClientFactory;

impl<'w, 's> ClientFactory<'w, 's> for CubeClientFactory {
    type Dependencies = PbrClientParams<'w, 's>;
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
        commands.insert(PbrBundle {
            visibility: Visibility {
                is_visible: if input.is_ghost {
                    deps.visibility_settings.ghosts
                } else {
                    true
                },
            },
            mesh: deps.meshes.add(Mesh::from(shape::Cube {
                size: input.desc.size * 2.0 * ghost_size_multiplier,
            })),
            material: {
                let materials = if input.is_ghost {
                    &deps.assets.materials.ghost
                } else {
                    &deps.assets.materials.normal
                };
                match input.collision_logic {
                    CollisionLogic::Death => materials.cube_death.clone(),
                    CollisionLogic::None => materials.cube.clone(),
                    // TODO: actually, reachable as we don't validate user's input yet: https://github.com/mvlabat/muddle-run/issues/36
                    CollisionLogic::Finish => unreachable!(),
                }
            },
            transform: Transform::from_translation(
                input
                    .desc
                    .position
                    .extend(input.desc.size + object_height(input.collision_logic)),
            ),
            ..Default::default()
        });
        commands.insert(bevy_mod_picking::PickableBundle::default());
    }

    #[cfg(feature = "client")]
    fn remove_components(commands: &mut EntityCommands, deps: &mut Self::Dependencies) {
        commands.remove::<PbrBundle>();
        commands.remove::<bevy_mod_picking::PickableBundle>();
        let mesh = deps.mesh_query.get(commands.id()).unwrap().clone();
        deps.meshes.remove(mesh);
    }
}

pub const ROUTE_POINT_HEIGHT: f32 = 0.8;
pub const ROUTE_POINT_BASE_EDGE_HALF_LEN: f32 = 0.25;

pub struct RoutePointClientFactory;

impl<'w, 's> ClientFactory<'w, 's> for RoutePointClientFactory {
    type Dependencies = PbrClientParams<'w, 's>;
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
        commands.insert(PbrBundle {
            visibility: Visibility {
                is_visible: if input.is_ghost {
                    deps.visibility_settings.ghosts
                } else {
                    deps.visibility_settings.route_points
                },
            },
            mesh: deps.meshes.add(Mesh::from(Pyramid {
                height: ROUTE_POINT_HEIGHT * ghost_size_multiplier,
                base_edge_half_len: ROUTE_POINT_BASE_EDGE_HALF_LEN * ghost_size_multiplier,
            })),
            material: if input.is_ghost {
                deps.assets.materials.ghost.route_point.clone()
            } else {
                deps.assets.materials.normal.route_point.clone()
            },
            transform: Transform::from_translation(input.desc.position.extend(0.0)),
            ..Default::default()
        });
        commands.insert(bevy_mod_picking::PickableBundle::default());
    }

    #[cfg(feature = "client")]
    fn remove_components(commands: &mut EntityCommands, deps: &mut Self::Dependencies) {
        commands.remove::<PbrBundle>();
        commands.remove::<bevy_mod_picking::PickableBundle>();
        let mesh = deps.mesh_query.get(commands.id()).unwrap().clone();
        deps.meshes.remove(mesh);
    }
}

#[cfg(feature = "client")]
#[derive(Resource, Default)]
pub struct VisibilitySettings {
    pub debug: bool,
    pub route_points: bool,
    pub ghosts: bool,
}

#[cfg(feature = "client")]
#[derive(SystemParam)]
pub struct PbrClientParams<'w, 's> {
    meshes: ResMut<'w, Assets<Mesh>>,
    assets: MuddleAssets<'w, 's>,
    visibility_settings: Res<'w, VisibilitySettings>,
    mesh_query: Query<'w, 's, &'static Handle<Mesh>>,
}

#[cfg(not(feature = "client"))]
#[derive(SystemParam)]
pub struct PbrClientParams<'w, 's> {
    #[system_param(ignore)]
    _w_lt: std::marker::PhantomData<&'w ()>,
    #[system_param(ignore)]
    _s_lt: std::marker::PhantomData<&'s ()>,
}
