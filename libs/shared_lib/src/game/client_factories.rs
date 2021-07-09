use crate::game::level_objects::*;
#[cfg(feature = "client")]
use crate::{
    client::*,
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

    fn remove_components(_commands: &mut EntityCommands) {}
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
            material: deps.materials.add(Color::rgb(0.8, 0.7, 0.6).into()),
            ..Default::default()
        });
        commands.insert(PredictedPosition { value: *position });
        if *is_player_frame_simulated {
            commands.insert(PlayerFrameSimulated);
        }
    }

    #[cfg(feature = "client")]
    fn remove_components(commands: &mut EntityCommands) {
        commands.remove_bundle::<PbrBundle>();
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
        let a = if input.is_ghost { 0.5 } else { 1.0 };
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
                is_transparent: false,
            },
            mesh: deps.meshes.add(Mesh::from(XyPlane {
                size: input.desc.size * 2.0 * ghost_size_multiplier,
            })),
            material: deps.materials.add(Color::rgba(0.3, 0.5, 0.3, a).into()),
            ..Default::default()
        });
        commands.insert(PlayerFrameSimulated);
    }

    #[cfg(feature = "client")]
    fn remove_components(commands: &mut EntityCommands) {
        commands.remove_bundle::<PbrBundle>();
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
        let a = if input.is_ghost { 0.5 } else { 1.0 };
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
                is_transparent: false,
            },
            mesh: deps.meshes.add(Mesh::from(shape::Cube {
                size: input.desc.size * 2.0 * ghost_size_multiplier,
            })),
            material: deps.materials.add(Color::rgba(0.4, 0.4, 0.4, a).into()),
            ..Default::default()
        });
        commands.insert(PlayerFrameSimulated);
    }

    #[cfg(feature = "client")]
    fn remove_components(commands: &mut EntityCommands) {
        commands.remove_bundle::<PbrBundle>();
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
        let a = if input.is_ghost { 0.5 } else { 1.0 };
        let ghost_size_multiplier = if input.is_ghost {
            GHOST_SIZE_MULTIPLIER
        } else {
            1.0
        };
        let mut material: StandardMaterial = Color::rgba(0.4, 0.4, 0.7, a).into();
        material.reflectance = 0.0;
        material.metallic = 0.0;
        commands.insert_bundle(PbrBundle {
            visible: Visible {
                is_visible: if input.is_ghost {
                    deps.visibility_settings.ghosts
                } else {
                    deps.visibility_settings.route_points
                },
                is_transparent: false,
            },
            mesh: deps.meshes.add(Mesh::from(Pyramid {
                height: ROUTE_POINT_HEIGHT * ghost_size_multiplier,
                base_edge_half_len: ROUTE_POINT_BASE_EDGE_HALF_LEN * ghost_size_multiplier,
            })),
            material: deps.materials.add(material),
            ..Default::default()
        });
        commands.insert(PlayerFrameSimulated);
    }

    #[cfg(feature = "client")]
    fn remove_components(commands: &mut EntityCommands) {
        commands.remove_bundle::<PbrBundle>();
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
    materials: ResMut<'a, Assets<StandardMaterial>>,
    visibility_settings: Res<'a, VisibilitySettings>,
}

#[cfg(not(feature = "client"))]
#[derive(SystemParam)]
pub struct PbrClientParams<'a> {
    #[system_param(ignore)]
    _lifetime: std::marker::PhantomData<&'a ()>,
}
