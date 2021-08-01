use bevy::ecs::system::ResMut;
use bevy_egui::{egui, EguiContext};

pub fn help_ui(egui_context: ResMut<EguiContext>) {
    puffin::profile_function!();
    let window_width = 280.0;
    let window_height = 30.0;

    egui::Window::new("Help")
        .title_bar(false)
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_BOTTOM, egui::Vec2::new(0.0, -40.0))
        .fixed_size(egui::Vec2::new(window_width, window_height))
        .show(egui_context.ctx(), |ui| {
            ui.centered_and_justified(|ui| {
                ui.label("Press ESC to toggle Builder mode");
            });
        });
}
