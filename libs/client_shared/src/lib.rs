use bevy::{log, prelude::*, render::camera::CameraProjection, diagnostic::FrameTimeDiagnosticsPlugin};
use bevy_egui::EguiPlugin;
use bevy_rapier3d::{
    physics::{
        ColliderHandleComponent, EventQueue, InteractionPairFilters, RapierPhysicsPlugin,
        RigidBodyHandleComponent,
    },
    rapier::{
        dynamics::{RigidBodyBuilder, RigidBodySet},
        geometry::{
            ColliderBuilder, ColliderSet, ContactEvent, ContactPairFilter, PairFilterContext,
            ProximityPairFilter, Ray, SolverFlags,
        },
        na,
    },
};

mod helpers;
mod input;
mod ui;

pub struct MuddlePlugin;

impl Plugin for MuddlePlugin {
    fn build(&self, builder: &mut AppBuilder) {
        builder
            .add_plugin(FrameTimeDiagnosticsPlugin)
            .add_plugin(EguiPlugin)
            // Physics.
            .add_resource(MouseRay(Ray::new(
                na::Point3::new(0.0, 0.0, 0.0),
                na::Vector3::new(0.0, 0.0, 0.0),
            )))
            .add_plugin(RapierPhysicsPlugin)
            .add_resource(
                InteractionPairFilters::new()
                    .contact_filter(PairFilter)
                    .proximity_filter(PairFilter),
            )
            .add_plugin(MaterialsPlugin)
            .init_resource::<WindowInnerSize>()
            .init_resource::<input::MousePosition>()
            // Startup systems,
            .add_startup_system(basic_scene.system())
            // Track input events.
            .init_resource::<input::TrackInputState>()
            .add_system(input::track_input_events.system())
            // Game systems.
            .add_system(move_controllable_object.system())
            .add_system(detect_collisions.system())
            // Egui.
            .add_system(ui::debug_ui::debug_ui.system());

        let resources = builder.resources_mut();
        resources.get_or_insert_with(ui::debug_ui::DebugUiState::default);
    }
}

// Constants.
const PLANE_SIZE: f32 = 10.0;

// Resources.
#[derive(Default)]
pub struct WindowInnerSize {
    pub width: usize,
    pub height: usize,
}

struct MaterialHandles {
    pub normal: Handle<StandardMaterial>,
    pub contacting: Handle<StandardMaterial>,
}

struct MainCameraEntity(pub Entity);

struct MouseRay(Ray);

// Components.
struct ControllableObjectTag;

struct MaterialsPlugin;

impl Plugin for MaterialsPlugin {
    fn build(&self, builder: &mut AppBuilder) {
        let mut materials = builder
            .resources_mut()
            .get_mut::<Assets<StandardMaterial>>()
            .expect("expected materials resource");

        let normal = materials.add(Color::rgb(0.87, 0.87, 0.87).into());
        let contacting = materials.add(Color::rgb(0.85, 0.1, 0.1).into());

        drop(materials);

        let resources = builder.resources_mut();
        resources.insert(MaterialHandles { normal, contacting });
    }
}

struct PairFilter;

impl ContactPairFilter for PairFilter {
    fn filter_contact_pair(&self, _context: &PairFilterContext) -> Option<SolverFlags> {
        Some(SolverFlags::COMPUTE_IMPULSES)
    }
}

impl ProximityPairFilter for PairFilter {
    fn filter_proximity_pair(&self, _context: &PairFilterContext) -> bool {
        true
    }
}

fn basic_scene(
    commands: &mut Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    material_handles: Res<MaterialHandles>,
) {
    // Add entities to the scene.
    commands
        // Plane.
        .spawn(PbrBundle {
            mesh: meshes.add(Mesh::from(shape::Plane { size: PLANE_SIZE })),
            material: materials.add(Color::rgb(0.3, 0.5, 0.3).into()),
            ..Default::default()
        })
        // Cube.
        .spawn(PbrBundle {
            mesh: meshes.add(Mesh::from(shape::Cube { size: 2.0 })),
            material: materials.add(Color::rgb(0.8, 0.7, 0.6).into()),
            ..Default::default()
        })
        .with(RigidBodyBuilder::new_static().translation(0.0, 1.0, 0.0))
        .with(ColliderBuilder::cuboid(1.0, 1.0, 1.0))
        // Controllable cube.
        .spawn(PbrBundle {
            mesh: meshes.add(Mesh::from(shape::Cube { size: 1.0 })),
            material: material_handles.normal.clone(),
            ..Default::default()
        })
        .with(ControllableObjectTag)
        .with(RigidBodyBuilder::new_kinematic().translation(0.0, 0.5, 0.0))
        .with(ColliderBuilder::cuboid(0.5, 0.5, 0.5))
        // Light.
        .spawn(LightBundle {
            transform: Transform::from_translation(Vec3::new(4.0, 8.0, 4.0)),
            ..Default::default()
        })
        // Camera.
        .spawn(Camera3dBundle {
            transform: Transform::from_translation(Vec3::new(-3.0, 5.0, 8.0))
                .looking_at(Vec3::default(), Vec3::unit_y()),
            ..Default::default()
        });
    let main_camera_entity = commands.current_entity().unwrap();
    commands.insert_resource(MainCameraEntity(main_camera_entity));
}

fn move_controllable_object(
    windows: Res<Windows>,
    mouse_position: Res<input::MousePosition>,
    main_camera_entity: Res<MainCameraEntity>,
    mut rigid_body_set: ResMut<RigidBodySet>,
    mut collider_set: ResMut<ColliderSet>,
    mut controllable_objects: Query<
        (&RigidBodyHandleComponent, &ColliderHandleComponent),
        With<ControllableObjectTag>,
    >,
    cameras: Query<(
        &Transform,
        &bevy::render::camera::Camera,
        &bevy::render::camera::PerspectiveProjection,
    )>,
) {
    let (rigid_body_handle, collider_handle) = controllable_objects
        .iter_mut()
        .next()
        .expect("expected a controllable object");
    let rigid_body = rigid_body_set
        .get_mut(rigid_body_handle.handle())
        .expect("expected a rigid body");
    let collider = collider_set
        .get_mut(collider_handle.handle())
        .expect("expected a collider");

    let window = windows.iter().next().expect("expected a window");
    let (camera_transform, _camera, camera_projection) = cameras
        .get(main_camera_entity.0)
        .expect("expected a main camera");
    let mouse_ray = helpers::cursor_pos_to_ray(
        &mouse_position.0,
        window,
        &camera_transform.compute_matrix(),
        &camera_projection.get_projection_matrix(),
    );

    if let Some(intersection_point) = helpers::intersect_ray_plane(&mouse_ray, PLANE_SIZE) {
        let mut new_position = *rigid_body.position();
        new_position.translation.x = intersection_point.x;
        new_position.translation.z = intersection_point.z;
        rigid_body.set_position(new_position, true);
        collider.set_position_debug(new_position);
    }
}

fn detect_collisions(
    events: Res<EventQueue>,
    material_handles: Res<MaterialHandles>,
    mut controllable_objects: Query<
        (&mut Handle<StandardMaterial>, &ColliderHandleComponent),
        With<ControllableObjectTag>,
    >,
) {
    while let Ok(contact_event) = events.contact_events.pop() {
        let (contacting, lhs_handle, rhs_handle) = match contact_event {
            ContactEvent::Started(lhs_handle, rhs_handle) => (true, lhs_handle, rhs_handle),
            ContactEvent::Stopped(lhs_handle, rhs_handle) => (false, lhs_handle, rhs_handle),
        };
        let controllable_object = controllable_objects
            .iter_mut()
            .find(|(_, collider_handle)| {
                collider_handle.handle() == lhs_handle || collider_handle.handle() == rhs_handle
            });

        if let Some((mut material, _)) = controllable_object {
            if contacting {
                log::info!("Applying contacting material");
                *material = material_handles.contacting.clone();
            } else {
                log::info!("Applying normal material");
                *material = material_handles.normal.clone();
            }
        }
    }
}
