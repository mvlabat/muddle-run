use crate::net::ServerToConnect;
use bevy::ecs::system::{Res, ResMut};
use bevy_egui::{egui, EguiContext};
use mr_shared_lib::net::{ConnectionState, ConnectionStatus};

pub fn connection_status_overlay_system(
    mut egui_context: ResMut<EguiContext>,
    connection_state: Res<ConnectionState>,
    server_to_connect: Res<ServerToConnect>,
) {
    #[cfg(feature = "profiler")]
    puffin::profile_function!();
    if matches!(
        connection_state.status(),
        ConnectionStatus::Uninitialized | ConnectionStatus::Connected
    ) && server_to_connect.is_none()
    {
        return;
    }

    let window_width = 200.0;
    let window_height = 100.0;

    let ctx = egui_context.ctx_mut();
    egui::CentralPanel::default()
        .frame(egui::Frame::none().fill(egui::Color32::from_black_alpha(200)))
        .show(ctx, |ui| {
            egui::Window::new("connection status")
                .title_bar(false)
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
                .fixed_size(egui::Vec2::new(window_width, window_height))
                .show(ui.ctx(), |ui| {
                    ui.centered_and_justified(|ui| {
                        ui.style_mut().override_text_style = Some(egui::TextStyle::Heading);
                        ui.label(format!("{:?}", connection_state.status()));
                    });
                });
        });
}
