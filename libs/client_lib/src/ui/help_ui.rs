use crate::helpers::PlayerParams;
use bevy::ecs::system::{Res, ResMut};
use bevy_egui::{egui, EguiContext};
use mr_shared_lib::{simulations_per_second, GameTime};

pub fn help_ui(
    time: Res<GameTime>,
    egui_context: ResMut<EguiContext>,
    player_params: PlayerParams,
) {
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
            let current_player = player_params.current_player();

            ui.centered_and_justified(|ui| {
                if let Some((respawned_at, _)) =
                    current_player.and_then(|player| player.respawning_at)
                {
                    let respawning_in_secs = (respawned_at
                        .value()
                        .saturating_sub(time.frame_number.value())
                        as f32
                        / simulations_per_second() as f32)
                        .ceil() as u16;
                    ui.label(format!("Respawning in {}...", respawning_in_secs));
                } else {
                    ui.label("Press ESC to toggle Builder mode");
                }
            });
        });
}
