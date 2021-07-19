use bevy::{
    ecs::system::{Res, ResMut},
    window::Windows,
};
use bevy_egui::{egui, EguiContext};

pub fn help_ui(egui_context: ResMut<EguiContext>, windows: Res<Windows>) {
    puffin::profile_function!();
    let primary_window = windows.get_primary().unwrap();
    let window_width = 280.0;
    let window_height = 30.0;

    egui::Window::new("Help")
        .title_bar(false)
        .collapsible(false)
        .resizable(false)
        .fixed_pos(egui::Pos2::new(
            (primary_window.physical_width() as f32 - window_width) / 2.0,
            primary_window.physical_height() as f32 - window_height - 40.0,
        ))
        .fixed_size(egui::Vec2::new(window_width, window_height))
        .show(egui_context.ctx(), |ui| {
            ui.centered_and_justified(|ui| {
                ui.label("Press ESC to toggle Builder mode");
            });
        });
}
