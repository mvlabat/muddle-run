use crate::game::level_objects::*;
#[cfg(feature = "client")]
use crate::{
    client::{materials::MuddleMaterials, *},
    game::components::{PlayerFrameSimulated, PredictedPosition},
    GHOST_SIZE_MULTIPLIER, PLAYER_SIZE,
};
#[cfg(feature = "client")]
use bevy::prelude::*;
use bevy::{
    ecs::system::{EntityCommands, SystemParam},
    math::Vec2,
};

pub trait ClientFactory<'a> {
    type Dependencies;
    type Input;

    fn insert_components(
        _commands: &mut EntityCommands,
        _deps: &mut Self::Dependencies,
        _input: &Self::Input,
    ) {
    }

    fn remove_components(_commands: &mut EntityCommands, _deps: &mut Self::Dependencies) {}
}

pub struct PlayerClientFactory;

impl<'a> ClientFactory<'a> for PlayerClientFactory {
    type Dependencies = PbrClientParams<'a>;
    type Input = (Vec2, bool);

    #[cfg(feature = "client")]
    fn insert_components(
        commands: &mut EntityCommands,
        deps: &mut Self::Dependencies,
        (position, is_player_frame_simulated): &Self::Input,
    ) {
        commands.insert_bundle(PbrBundle {
            mesh: deps.meshes.add(Mesh::from(shape::Cube {
                size: PLAYER_SIZE * 2.0,
            })),
            material: deps.materials.player.clone(),
            ..Default::default()
        });
        commands.insert(PredictedPosition { value: *position });
        if *is_player_frame_simulated {
            commands.insert(PlayerFrameSimulated);
        }
    }

    #[cfg(feature = "client")]
    fn remove_components(commands: &mut EntityCommands, deps: &mut Self::Dependencies) {
        commands.remove_bundle::<PbrBundle>();
        let mesh = deps.mesh_query.get(commands.id()).unwrap().clone();
        deps.meshes.remove(mesh);
    }
}

pub struct PlaneClientFactory;

pub struct LevelObjectInput<T> {
    pub desc: T,
    pub is_ghost: bool,
}

impl<'a> ClientFactory<'a> for PlaneClientFactory {
    type Dependencies = PbrClientParams<'a>;
    type Input = LevelObjectInput<PlaneDesc>;

    #[cfg(feature = "client")]
    fn insert_components(
        commands: &mut EntityCommands,
        deps: &mut Self::Dependencies,
        input: &Self::Input,
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
            mesh: deps.meshes.add(Mesh::from(XyPlane {
                size: input.desc.size * 2.0 * ghost_size_multiplier,
            })),
            material: if input.is_ghost {
                deps.materials.ghost.plane.clone()
            } else {
                deps.materials.normal.plane.clone()
            },
            ..Default::default()
        });
        commands.insert(PlayerFrameSimulated);
    }

    #[cfg(feature = "client")]
    fn remove_components(commands: &mut EntityCommands, deps: &mut Self::Dependencies) {
        commands.remove_bundle::<PbrBundle>();
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
        input: &Self::Input,
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
            ..Default::default()
        });
        commands.insert(PlayerFrameSimulated);
    }

    #[cfg(feature = "client")]
    fn remove_components(commands: &mut EntityCommands, deps: &mut Self::Dependencies) {
        commands.remove_bundle::<PbrBundle>();
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
        input: &Self::Input,
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
        commands.insert(PlayerFrameSimulated);
    }

    #[cfg(feature = "client")]
    fn remove_components(commands: &mut EntityCommands, deps: &mut Self::Dependencies) {
        commands.remove_bundle::<PbrBundle>();
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
