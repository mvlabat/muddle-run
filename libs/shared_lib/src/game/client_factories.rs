use crate::game::level_objects::PlaneDesc;
#[cfg(feature = "render")]
use crate::{game::components::PlayerFrameSimulated, PLAYER_SIZE};
use bevy::{ecs::SystemParam, prelude::*};

pub trait ClientFactory<'a> {
    type Dependencies;
    type Input;

    fn create(
        commands: &mut Commands,
        _deps: &mut Self::Dependencies,
        _input: &Self::Input,
    ) -> Entity {
        commands.spawn(());
        commands.current_entity().unwrap()
    }

    fn remove_renderables(_commands: &mut Commands, _entity: Entity) {}
}

pub struct PlayerClientFactory;

impl<'a> ClientFactory<'a> for PlayerClientFactory {
    type Dependencies = PbrClientParams<'a>;
    type Input = bool;

    #[cfg(feature = "render")]
    fn create(
        commands: &mut Commands,
        deps: &mut Self::Dependencies,
        is_player_frame_simulated: &Self::Input,
    ) -> Entity {
        commands.spawn(PbrBundle {
            mesh: deps
                .meshes
                .add(Mesh::from(shape::Cube { size: PLAYER_SIZE })),
            material: deps.materials.add(Color::rgb(0.8, 0.7, 0.6).into()),
            ..Default::default()
        });
        if *is_player_frame_simulated {
            commands.with(PlayerFrameSimulated);
        }
        commands.current_entity().unwrap()
    }

    #[cfg(feature = "render")]
    fn remove_renderables(commands: &mut Commands, entity: Entity) {
        commands.remove::<PbrBundle>(entity);
    }
}

pub struct PlaneClientFactory;

impl<'a> ClientFactory<'a> for PlaneClientFactory {
    type Dependencies = PbrClientParams<'a>;
    type Input = (PlaneDesc, bool);

    #[cfg(feature = "render")]
    fn create(
        commands: &mut Commands,
        deps: &mut Self::Dependencies,
        (plane_desc, is_player_frame_simulated): &Self::Input,
    ) -> Entity {
        commands.spawn(PbrBundle {
            mesh: deps.meshes.add(Mesh::from(shape::Plane {
                size: plane_desc.size,
            })),
            material: deps.materials.add(Color::rgb(0.3, 0.5, 0.3).into()),
            ..Default::default()
        });
        if *is_player_frame_simulated {
            commands.with(PlayerFrameSimulated);
        }
        commands.current_entity().unwrap()
    }
}

#[cfg(feature = "render")]
#[derive(SystemParam)]
pub struct PbrClientParams<'a> {
    meshes: ResMut<'a, Assets<Mesh>>,
    materials: ResMut<'a, Assets<StandardMaterial>>,
}

#[cfg(not(feature = "render"))]
#[derive(SystemParam)]
pub struct PbrClientParams<'a> {
    #[system_param(ignore)]
    _lifetime: std::marker::PhantomData<&'a ()>,
}
