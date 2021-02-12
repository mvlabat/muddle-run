use crate::net::{initiate_connection, process_network_events, send_network_updates};
use bevy::{diagnostic::FrameTimeDiagnosticsPlugin, prelude::*};
use bevy_egui::EguiPlugin;
use bevy_networking_turbulence::NetworkingPlugin;
use mr_shared_lib::{net::PlayerNetId, MuddleSharedPlugin};

mod helpers;
mod input;
mod net;
mod ui;

pub struct MuddleClientPlugin;

impl Plugin for MuddleClientPlugin {
    fn build(&self, builder: &mut AppBuilder) {
        builder
            .add_plugin(FrameTimeDiagnosticsPlugin)
            .add_plugin(EguiPlugin)
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
            .add_plugin(MuddleSharedPlugin::default())
            // Write network updates.
            .add_system(send_network_updates.system())
            // Egui.
            .add_system(ui::debug_ui::debug_ui.system());

        let resources = builder.resources_mut();
        resources.get_or_insert_with(ui::debug_ui::DebugUiState::default);
        resources.get_or_insert_with(CurrentPlayerNetId::default);
    }
}

// Resources.
#[derive(Default)]
pub struct WindowInnerSize {
    pub width: usize,
    pub height: usize,
}

#[derive(Default)]
pub struct CurrentPlayerNetId(pub Option<PlayerNetId>);

struct MainCameraEntity(pub Entity);

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
