use crate::net::{initiate_connection, process_network_events, send_network_updates};
use bevy::{
    diagnostic::FrameTimeDiagnosticsPlugin, log, prelude::*, render::camera::CameraProjection,
};
use bevy_egui::EguiPlugin;
use bevy_networking_turbulence::NetworkingPlugin;
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
use mr_shared_lib::{
    game::{level_objects::PlaneDesc, spawn::EmptySpawner},
    MuddleSharedPlugin,
};
use crate::spawners::{PlaneSpawner, SpawnerPbrDeps, PlayerSpawner};

mod helpers;
mod input;
mod net;
mod spawners;
mod ui;

pub struct MuddleClientPlugin;

impl Plugin for MuddleClientPlugin {
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
            // Networking.
            .add_plugin(NetworkingPlugin)
            .add_system(initiate_connection.system())
            .add_system(process_network_events.system())
            // Track input events.
            .init_resource::<input::TrackInputState>()
            .add_system(input::track_input_events.system())
            // Game.
            .add_plugin(MuddleSharedPlugin::<
                SpawnerPbrDeps,
                PlayerSpawner,
                SpawnerPbrDeps,
                PlaneSpawner,
            >::default())
            // Write network updates.
            .add_system(send_network_updates.system())
            // Egui.
            .add_system(ui::debug_ui::debug_ui.system());

        let resources = builder.resources_mut();
        resources.get_or_insert_with(ui::debug_ui::DebugUiState::default);
    }
}

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

fn basic_scene(commands: &mut Commands) {
    // Add entities to the scene.
    commands
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
