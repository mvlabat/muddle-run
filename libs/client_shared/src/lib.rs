use crate::bevy_megaui::{MegaUiContext, MegaUiPlugin};
use bevy::{
    input::{keyboard::KeyboardInput, mouse::MouseButtonInput},
    prelude::*,
    render::camera::CameraProjection,
};
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
use megaui::{hash, Vector2};

mod bevy_megaui;
mod transform_node;

pub struct MuddlePlugin;

impl Plugin for MuddlePlugin {
    fn build(&self, builder: &mut AppBuilder) {
        builder
            .add_plugin(MegaUiPlugin)
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
            // Window and rendering.
            .add_resource(WindowDescriptor {
                title: "Muddle Run".to_owned(),
                width: 1280,
                height: 1024,
                ..Default::default()
            })
            // .add_resource(Msaa { samples: 4 })
            .add_plugin(MaterialsPlugin)
            .init_resource::<WindowInnerSize>()
            .init_resource::<MousePosition>()
            // Startup systems,
            .add_startup_system(basic_scene)
            // Track input events.
            .init_resource::<TrackInputState>()
            .add_system(track_input_events)
            // Game systems.
            .add_system(move_controllable_object)
            .add_system(detect_collisions)
            // Megaui.
            .add_system(test_ui);
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

#[derive(Default)]
struct TrackInputState {
    pub keys: EventReader<KeyboardInput>,
    pub cursor: EventReader<CursorMoved>,
    pub mouse_button: EventReader<MouseButtonInput>,
}

#[derive(Default)]
struct MousePosition(pub Vec2);

struct MouseRay(Ray);

// Components.
struct ControllableObjectTag;

struct MaterialsPlugin;

impl Plugin for MaterialsPlugin {
    fn build(&self, app: &mut AppBuilder) {
        let mut materials = app
            .resources_mut()
            .get_mut::<Assets<StandardMaterial>>()
            .expect("expected materials resource");

        let normal = materials.add(Color::rgb(0.87, 0.87, 0.87).into());
        let contacting = materials.add(Color::rgb(0.85, 0.1, 0.1).into());

        drop(materials);

        app.resources_mut()
            .insert(MaterialHandles { normal, contacting });
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

fn test_ui(_world: &mut World, resources: &mut Resources) {
    let mut megaui_context = resources.get_thread_local_mut::<MegaUiContext>().unwrap();
    megaui::widgets::Window::new(hash!(), Vector2::new(0.0, 0.0), Vector2::new(300.0, 300.0))
        .label("TEST")
        .ui(&mut megaui_context.ui, |_ui| {});
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
            mesh: meshes.add(Mesh::from(shape::Cube { size: 1.0 })),
            material: materials.add(Color::rgb(0.8, 0.7, 0.6).into()),
            ..Default::default()
        })
        .with(RigidBodyBuilder::new_static().translation(0.0, 1.0, 0.0))
        .with(ColliderBuilder::cuboid(1.0, 1.0, 1.0))
        // Controllable cube.
        .spawn(PbrBundle {
            mesh: meshes.add(Mesh::from(shape::Cube { size: 0.5 })),
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

fn track_input_events(
    mut state: ResMut<TrackInputState>,
    mut mouse_position: ResMut<MousePosition>,
    ev_keys: Res<Events<KeyboardInput>>,
    ev_cursor: Res<Events<CursorMoved>>,
    ev_mouse_button: Res<Events<MouseButtonInput>>,
) {
    // Keyboard input.
    for ev in state.keys.iter(&ev_keys) {
        if ev.state.is_pressed() {
            log::info!("Just pressed key: {:?}", ev.key_code);
        } else {
            log::info!("Just released key: {:?}", ev.key_code);
        }
    }

    // Absolute cursor position (in window coordinates).
    if let Some(ev) = state.cursor.latest(&ev_cursor) {
        mouse_position.0 = ev.position;
    }

    // Mouse buttons.
    for ev in state.mouse_button.iter(&ev_mouse_button) {
        if ev.state.is_pressed() {
            log::info!("Just pressed mouse button: {:?}", ev.button);
        } else {
            log::info!("Just released mouse button: {:?}", ev.button);
        }
    }
}

// Heavily inspired by https://github.com/bevyengine/bevy/pull/432/.
fn cursor_pos_to_ray(
    cursor_viewport: &Vec2,
    window: &Window,
    camera_transform: &Mat4,
    camera_perspective: &Mat4,
) -> Ray {
    // Calculate the cursor pos in NDC space [(-1,-1), (1,1)].
    let cursor_ndc = Vec4::from((
        (cursor_viewport.x / window.width() as f32) * 2.0 - 1.0,
        (cursor_viewport.y / window.height() as f32) * 2.0 - 1.0,
        -1.0, // let the cursor be on the far clipping plane
        1.0,
    ));

    let object_to_world = camera_transform;
    let object_to_ndc = camera_perspective;

    // Transform the cursor position into object/camera space. This also turns the cursor into
    // a vector that's pointing from the camera center onto the far plane.
    let mut ray_camera = object_to_ndc.inverse().mul_vec4(cursor_ndc);
    ray_camera.z = -1.0;
    ray_camera.w = 0.0; // treat the vector as a direction (0 = Direction, 1 = Position)

    // Transform the cursor into world space.
    let ray_world = object_to_world.mul_vec4(ray_camera);
    let ray_world = ray_world.truncate();

    let camera_pos = camera_transform.w_axis.truncate();
    let camera_pos = na::Point3::new(camera_pos.x, camera_pos.y, camera_pos.z);
    Ray::new(
        camera_pos,
        na::Vector3::from_row_slice(ray_world.normalize().as_ref()),
    )
}

fn move_controllable_object(
    windows: Res<Windows>,
    mouse_position: Res<MousePosition>,
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
    let mut rigid_body = rigid_body_set
        .get_mut(rigid_body_handle.handle())
        .expect("expected a rigid body");
    let collider = collider_set
        .get_mut(collider_handle.handle())
        .expect("expected a collider");

    let window = windows.iter().next().expect("expected a window");
    let (camera_transform, _camera, camera_projection) = cameras
        .get(main_camera_entity.0)
        .expect("expected a main camera");
    let mouse_ray = cursor_pos_to_ray(
        &mouse_position.0,
        window,
        &camera_transform.compute_matrix(),
        &camera_projection.get_projection_matrix(),
    );

    if let Some(intersection_point) = intersect_ray_plane(&mouse_ray, PLANE_SIZE) {
        let mut new_position = rigid_body.position;
        new_position.translation.x = intersection_point.x;
        new_position.translation.z = intersection_point.z;
        rigid_body.set_position(new_position);
        collider.set_position_debug(new_position);
    }
}

fn intersect_ray_plane(ray: &Ray, size: f32) -> Option<Vec3> {
    let plane_normal = Vec3::new(0.0, 1.0, 0.0);
    let ray_origin = Vec3::new(ray.origin.x, ray.origin.y, ray.origin.z);
    let ray_direction = Vec3::new(ray.dir.x, ray.dir.y, ray.dir.z);

    let denominator = plane_normal.dot(ray_direction);
    if denominator.abs() > f32::EPSILON {
        let t = (-ray_origin).dot(plane_normal) / denominator;
        if t >= f32::EPSILON {
            return Some(ray_origin + t * ray_direction).and_then(|intersection_point| {
                // Checks that the intersection point is within the plane. Assumes plane's origin
                // at zero coordinates.
                if intersection_point.y.abs() <= f32::EPSILON
                    && intersection_point.x.abs() <= size / 2.0
                    && intersection_point.z.abs() <= size / 2.0
                {
                    Some(intersection_point)
                } else {
                    None
                }
            });
        }
    }
    None
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
