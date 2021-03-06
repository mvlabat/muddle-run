use bevy::{
    asset::{Assets, Handle},
    ecs::system::{Commands, ResMut},
    prelude::StandardMaterial,
    render::color::Color,
};

pub struct MuddleMaterials {
    pub player: Handle<StandardMaterial>,
    pub normal: ObjectMaterials,
    pub ghost: ObjectMaterials,
}

pub fn init_object_materials(
    mut commands: Commands,
    mut materials: ResMut<Assets<StandardMaterial>>,
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
    })
}

pub struct ObjectMaterials {
    pub plane: Handle<StandardMaterial>,
    pub cube: Handle<StandardMaterial>,
    pub route_point: Handle<StandardMaterial>,
}
