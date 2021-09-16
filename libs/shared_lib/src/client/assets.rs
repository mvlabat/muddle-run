use bevy::{
    asset::{Assets, Handle},
    ecs::system::{Commands, Res, ResMut, SystemParam},
    prelude::StandardMaterial,
    render::{
        color::Color,
        mesh::{shape::Icosphere, Mesh},
    },
};

#[derive(SystemParam)]
pub struct MuddleAssets<'a> {
    pub materials: Res<'a, MuddleMaterials>,
    pub meshes: Res<'a, MuddleMeshes>,
}

pub struct MuddleMaterials {
    pub player: Handle<StandardMaterial>,
    pub normal: ObjectMaterials,
    pub ghost: ObjectMaterials,
    pub control_point_normal: Handle<StandardMaterial>,
    pub control_point_hovered: Handle<StandardMaterial>,
}

pub struct MuddleMeshes {
    pub control_point: Handle<Mesh>,
}

pub fn init_muddle_assets(
    mut commands: Commands,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut meshes: ResMut<Assets<Mesh>>,
) {
    let a = 0.5;
    commands.insert_resource(MuddleMaterials {
        player: materials.add(Color::rgb(0.8, 0.7, 0.6).into()),
        normal: ObjectMaterials {
            plane: materials.add(Color::rgb(0.3, 0.5, 0.3).into()),
            cube: materials.add(Color::rgb(0.4, 0.4, 0.4).into()),
            route_point: {
                let mut material: StandardMaterial = Color::rgb(0.4, 0.4, 0.7).into();
                material.reflectance = 0.0;
                material.metallic = 0.0;
                materials.add(material)
            },
        },
        ghost: ObjectMaterials {
            plane: materials.add(Color::rgba(0.3, 0.5, 0.3, a).into()),
            cube: materials.add(Color::rgba(0.4, 0.4, 0.4, a).into()),
            route_point: {
                let mut material: StandardMaterial = Color::rgba(0.4, 0.4, 0.7, a).into();
                material.reflectance = 0.0;
                material.metallic = 0.0;
                materials.add(material)
            },
        },
        control_point_normal: materials.add(Color::rgb(1.0, 0.992, 0.816).into()),
        control_point_hovered: materials.add(Color::rgb(0.5, 0.492, 0.816).into()),
    });
    commands.insert_resource(MuddleMeshes {
        control_point: meshes.add(Mesh::from(Icosphere {
            radius: 0.15,
            subdivisions: 32,
        })),
    });
}

pub struct ObjectMaterials {
    pub plane: Handle<StandardMaterial>,
    pub cube: Handle<StandardMaterial>,
    pub route_point: Handle<StandardMaterial>,
}
