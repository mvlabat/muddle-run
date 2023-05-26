use crate::road::{
    mesh::RoadMesh,
    systems::{edit_road, sync_mesh},
    ControlPointIndex, Road, RoadModel, SelectedRoad,
};
use bevy::{
    pbr::{NotShadowCaster, NotShadowReceiver},
    prelude::*,
    render::view::RenderLayers,
};

mod road;

fn main() {
    App::new()
        .add_plugins(DefaultPlugins)
        .add_plugins(bevy_mod_picking::DefaultPickingPlugins)
        .add_plugin(bevy_egui::EguiPlugin)
        .add_plugin(bevy_inspector_egui::quick::WorldInspectorPlugin::new())
        .add_plugin(bevy_transform_gizmo::TransformGizmoPlugin::default())
        .add_system(edit_road)
        .add_system(sync_mesh)
        .add_startup_system(setup)
        .run();
}

/// set up a simple 3D scene
fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let control_point_mesh = meshes.add(
        shape::Icosphere {
            radius: 0.5,
            subdivisions: 32,
        }
        .try_into()
        .unwrap(),
    );
    let control_point_material = materials.add(Color::rgb(0.8, 0.7, 0.6).into());

    // control points
    let control_points = vec![
        Vec3::new(-6.0, 2.5, -15.0),
        Vec3::new(0.0, 0.5, -7.0),
        Vec3::new(0.0, 0.5, 2.0),
        Vec3::new(-0.5, 0.7, 7.0),
    ];
    let road_entity = commands
        .spawn((
            PbrBundle {
                mesh: meshes.add(Mesh::from(RoadMesh {
                    edge_ring_count: 8,
                    model: RoadModel {
                        control_points: control_points.clone(),
                    },
                })),
                material: materials.add(Color::rgb(0.45, 0.75, 0.60).into()),
                ..default()
            },
            Road { edge_ring_count: 8 },
        ))
        .with_children(|builder| {
            for (i, control_point) in control_points.iter().enumerate() {
                builder.spawn((
                    PbrBundle {
                        mesh: control_point_mesh.clone(),
                        material: control_point_material.clone(),
                        transform: Transform::from_translation(*control_point),
                        ..default()
                    },
                    ControlPointIndex(i),
                    bevy_mod_picking::PickableBundle::default(),
                    bevy_transform_gizmo::GizmoTransformable,
                    NotShadowCaster,
                    NotShadowReceiver,
                    RenderLayers::layer(12),
                ));
            }
        })
        .id();

    commands.insert_resource(SelectedRoad(road_entity));

    // light
    commands.spawn(PointLightBundle {
        point_light: PointLight {
            intensity: 4000.0,
            range: 75.0,
            shadows_enabled: true,
            radius: 20.0,
            ..default()
        },
        transform: Transform::from_xyz(4.0, 13.0, 4.0),
        ..default()
    });
    // camera
    commands
        .spawn(Camera3dBundle {
            transform: Transform::from_xyz(-6.0, 25.5, 6.0).looking_at(Vec3::ZERO, Vec3::Y),
            ..default()
        })
        .insert(bevy_mod_picking::PickingCameraBundle::default())
        .insert(bevy_transform_gizmo::GizmoPickSource::default());
}
