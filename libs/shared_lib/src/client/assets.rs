use crate::PLAYER_SENSOR_RADIUS;
use bevy::{
    asset::{Assets, Handle},
    ecs::system::{Commands, Res, ResMut, Resource, SystemParam},
    pbr::AlphaMode,
    prelude::StandardMaterial,
    render::{
        color::Color,
        mesh::{shape::Icosphere, Mesh},
    },
};
use std::marker::PhantomData;

#[derive(SystemParam)]
pub struct MuddleAssets<'w, 's> {
    pub materials: Res<'w, MuddleMaterials>,
    pub meshes: Res<'w, MuddleMeshes>,
    #[system_param(ignore)]
    marker: PhantomData<&'s ()>,
}

#[derive(Resource)]
pub struct MuddleMaterials {
    pub player: Handle<StandardMaterial>,
    pub player_sensor_death: Handle<StandardMaterial>,
    pub player_sensor_normal: Handle<StandardMaterial>,
    pub normal: ObjectMaterials,
    pub ghost: ObjectMaterials,
    pub control_point_normal: Handle<StandardMaterial>,
    pub control_point_hovered: Handle<StandardMaterial>,
}

#[derive(Resource)]
pub struct MuddleMeshes {
    pub player_sensor: Handle<Mesh>,
    pub control_point: Handle<Mesh>,
}

pub fn init_muddle_assets_system(
    mut commands: Commands,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut meshes: ResMut<Assets<Mesh>>,
) {
    let a = 0.5;
    commands.insert_resource(MuddleMaterials {
        player: materials.add(Color::rgb(0.8, 0.7, 0.6).into()),
        player_sensor_death: {
            let mut material: StandardMaterial = Color::rgb(1.0, 0.2, 0.25).into();
            material.reflectance = 0.0;
            material.metallic = 0.0;
            materials.add(material)
        },
        player_sensor_normal: {
            let mut material: StandardMaterial = Color::rgb(0.4, 0.4, 0.7).into();
            material.reflectance = 0.0;
            material.metallic = 0.0;
            materials.add(material)
        },
        normal: ObjectMaterials {
            plane: materials.add(Color::rgb(0.3, 0.5, 0.3).into()),
            plane_death: materials.add(Color::rgb(0.55, 0.15, 0.2).into()),
            plane_finish: materials.add(Color::rgb(0.2, 0.25, 0.75).into()),
            cube: materials.add(Color::rgb(0.4, 0.4, 0.4).into()),
            cube_death: materials.add(Color::rgb(0.8, 0.35, 0.35).into()),
            route_point: {
                let mut material: StandardMaterial = Color::rgb(0.4, 0.4, 0.7).into();
                material.reflectance = 0.0;
                material.metallic = 0.0;
                materials.add(material)
            },
        },
        ghost: ObjectMaterials {
            plane: materials.add(with_blend_alpha_mode(Color::rgba(0.3, 0.5, 0.3, a).into())),
            plane_death: materials.add(with_blend_alpha_mode(
                Color::rgba(0.55, 0.15, 0.2, a).into(),
            )),
            plane_finish: materials.add(with_blend_alpha_mode(
                Color::rgba(0.2, 0.25, 0.75, a).into(),
            )),
            cube: materials.add(with_blend_alpha_mode(Color::rgba(0.4, 0.4, 0.4, a).into())),
            cube_death: materials.add(with_blend_alpha_mode(
                Color::rgba(0.8, 0.35, 0.35, a).into(),
            )),
            route_point: {
                let mut material: StandardMaterial =
                    with_blend_alpha_mode(Color::rgba(0.4, 0.4, 0.7, a).into());
                material.reflectance = 0.0;
                material.metallic = 0.0;
                materials.add(material)
            },
        },
        control_point_normal: materials
            .add(with_blend_alpha_mode(Color::rgb(1.0, 0.992, 0.816).into())),
        control_point_hovered: materials
            .add(with_blend_alpha_mode(Color::rgb(0.5, 0.492, 0.816).into())),
    });
    commands.insert_resource(MuddleMeshes {
        player_sensor: meshes.add(Mesh::from(Icosphere {
            radius: PLAYER_SENSOR_RADIUS,
            subdivisions: 16,
        })),
        control_point: meshes.add(Mesh::from(Icosphere {
            radius: 0.15,
            subdivisions: 32,
        })),
    });
}

fn with_blend_alpha_mode(mut material: StandardMaterial) -> StandardMaterial {
    material.alpha_mode = AlphaMode::Blend;
    material
}

pub struct ObjectMaterials {
    pub plane: Handle<StandardMaterial>,
    pub plane_death: Handle<StandardMaterial>,
    pub plane_finish: Handle<StandardMaterial>,
    pub cube: Handle<StandardMaterial>,
    pub cube_death: Handle<StandardMaterial>,
    pub route_point: Handle<StandardMaterial>,
}
