use crate::game::level_objects::*;
#[cfg(feature = "client")]
use crate::{
    client::XyPlane,
    game::components::{PlayerFrameSimulated, PredictedPosition},
    PLAYER_SIZE,
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
        commands.remove::<PbrBundle>();
    }
}

pub struct PlaneClientFactory;

impl<'a> ClientFactory<'a> for PlaneClientFactory {
    type Dependencies = PbrClientParams<'a>;
    type Input = PlaneDesc;

    #[cfg(feature = "client")]
    fn insert_components(
        commands: &mut EntityCommands,
        deps: &mut Self::Dependencies,
        plane_desc: &Self::Input,
    ) {
        commands.insert_bundle(PbrBundle {
            mesh: deps.meshes.add(Mesh::from(XyPlane {
                size: plane_desc.size * 2.0,
            })),
            material: deps.materials.add(Color::rgb(0.3, 0.5, 0.3).into()),
            ..Default::default()
        });
        commands.insert(PlayerFrameSimulated);
    }

    #[cfg(feature = "client")]
    fn remove_components(commands: &mut EntityCommands) {
        commands.remove::<PbrBundle>();
    }
}

pub struct CubeClientFactory;

impl<'a> ClientFactory<'a> for CubeClientFactory {
    type Dependencies = PbrClientParams<'a>;
    type Input = CubeDesc;

    #[cfg(feature = "client")]
    fn insert_components(
        commands: &mut EntityCommands,
        deps: &mut Self::Dependencies,
        cube_desc: &Self::Input,
    ) {
        commands.insert_bundle(PbrBundle {
            mesh: deps.meshes.add(Mesh::from(shape::Cube {
                size: cube_desc.size * 2.0,
            })),
            material: deps.materials.add(Color::rgb(0.4, 0.4, 0.4).into()),
            ..Default::default()
        });
        commands.insert(PlayerFrameSimulated);
    }

    #[cfg(feature = "client")]
    fn remove_components(commands: &mut EntityCommands) {
        commands.remove::<PbrBundle>();
    }
}

#[cfg(feature = "client")]
#[derive(SystemParam)]
pub struct PbrClientParams<'a> {
    meshes: ResMut<'a, Assets<Mesh>>,
    materials: ResMut<'a, Assets<StandardMaterial>>,
}

#[cfg(not(feature = "client"))]
#[derive(SystemParam)]
pub struct PbrClientParams<'a> {
    #[system_param(ignore)]
    _lifetime: std::marker::PhantomData<&'a ()>,
}
