use crate::{
    components::{CameraPivotDirection, CameraPivotTag},
    MainCameraEntity, MainCameraPivotEntity,
};
use bevy::{
    core_pipeline::core_3d::Camera3dBundle,
    ecs::{
        entity::Entity,
        system::{Commands, Local},
    },
    hierarchy::BuildChildren,
    log,
    math::{Vec2, Vec3},
    pbr::{PbrBundle, PointLight, PointLightBundle},
    transform::components::{GlobalTransform, Transform},
};
use iyes_loopless::state::NextState;
use mr_shared_lib::{client::assets::MuddleAssets, AppState};

/// This system is needed for the web version. As assets loading is blocking
/// there, we need to trigger loading shaders before we join a game.
pub fn load_shaders_system(
    mut commands: Commands,
    assets: MuddleAssets,
    mut frames_skipped: Local<u8>,
    mut entities_to_clean_up: Local<Vec<Entity>>,
) {
    // This lets Egui to fully initialise and finish loading its shaders. <3
    *frames_skipped += 1;
    if *frames_skipped < 3 {
        return;
    }

    if !entities_to_clean_up.is_empty() {
        for entity in std::mem::take(&mut *entities_to_clean_up) {
            commands.entity(entity).despawn();
        }
        log::info!("Changing the app state to {:?}", AppState::MainMenu);
        commands.insert_resource(NextState(AppState::MainMenu));
        return;
    }

    log::info!("Starting to load the shaders");
    entities_to_clean_up.push(
        commands
            .spawn(PbrBundle {
                mesh: assets.meshes.control_point.clone(),
                material: assets.materials.player.clone(),
                ..Default::default()
            })
            .id(),
    );
}

pub fn basic_scene_system(mut commands: Commands) {
    // Add entities to the scene.
    commands.spawn(PointLightBundle {
        point_light: PointLight {
            range: 256.0,
            intensity: 1280000.0,
            ..Default::default()
        },
        transform: Transform::from_translation(Vec3::new(-64.0, -92.0, 144.0)),
        ..Default::default()
    });
    // Camera.
    let main_camera_entity = commands
        .spawn(Camera3dBundle {
            transform: Transform::from_translation(Vec3::new(-3.0, -14.0, 14.0))
                .looking_at(Vec3::default(), Vec3::Z),
            ..Default::default()
        })
        .insert(bevy_mod_picking::PickingCameraBundle::default())
        .id();
    let main_camera_pivot_entity = commands
        .spawn_empty()
        .insert(CameraPivotTag)
        .insert(CameraPivotDirection(Vec2::ZERO))
        .insert(Transform::IDENTITY)
        .insert(GlobalTransform::IDENTITY)
        .add_child(main_camera_entity)
        .id();
    commands.insert_resource(MainCameraPivotEntity(main_camera_pivot_entity));
    commands.insert_resource(MainCameraEntity(main_camera_entity));
}
