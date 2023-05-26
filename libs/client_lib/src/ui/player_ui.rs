use crate::helpers::PlayerParams;
use bevy::{
    ecs::system::{Local, Res},
    input::{keyboard::KeyCode, Input},
};
use bevy_egui::{egui, EguiContexts};
use mr_shared_lib::{
    messages::RespawnPlayerReason, player::PlayerRole, GameTime, SIMULATIONS_PER_SECOND,
};

pub fn help_ui_system(
    time: Res<GameTime>,
    mut egui_contexts: EguiContexts,
    player_params: PlayerParams,
) {
    #[cfg(feature = "profiler")]
    puffin::profile_function!();
    let window_width = 280.0;
    let window_height = 30.0;

    egui::Window::new("Help")
        .title_bar(false)
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_BOTTOM, egui::Vec2::new(0.0, -40.0))
        .fixed_size(egui::Vec2::new(window_width, window_height))
        .show(egui_contexts.ctx_mut(), |ui| {
            let current_player = player_params.current_player();

            ui.centered_and_justified(|ui| {
                if let Some((respawned_at, _)) =
                    current_player.and_then(|player| player.respawning_at)
                {
                    let respawning_in_secs = (respawned_at
                        .value()
                        .saturating_sub(time.frame_number.value())
                        as f32
                        / SIMULATIONS_PER_SECOND)
                        .ceil() as u16;
                    ui.label(format!("Respawning in {respawning_in_secs}..."));
                } else {
                    ui.label("Press ESC to toggle Builder mode");
                }
            });
        });
}

pub struct LeaderboardState {
    show: bool,
}

impl Default for LeaderboardState {
    fn default() -> Self {
        Self { show: true }
    }
}

pub fn leaderboard_ui_system(
    mut state: Local<LeaderboardState>,
    keyboard_input: Res<Input<KeyCode>>,
    mut egui_contexts: EguiContexts,
    player_params: PlayerParams,
) {
    #[cfg(feature = "profiler")]
    puffin::profile_function!();
    if keyboard_input.just_pressed(KeyCode::F3) {
        state.show = !state.show;
    }

    if !state.show {
        return;
    }

    egui::Window::new("Leaderboard [F3]")
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::RIGHT_TOP, egui::Vec2::new(-35.0, 35.0))
        .show(egui_contexts.ctx_mut(), |ui| {
            egui::Grid::new("stats board")
                .min_col_width(13.0)
                .show(ui, |ui| {
                    let mut players = player_params.players.iter().collect::<Vec<_>>();
                    players.sort_by(|(a_id, a), (b_id, b)| {
                        b.finishes
                            .cmp(&a.finishes)
                            .then(a.deaths.cmp(&b.deaths))
                            .then(a_id.0.cmp(&b_id.0))
                    });
                    ui.label("");
                    ui.label("Nickname");
                    ui.label("Finishes");
                    ui.label("Deaths");
                    ui.label("");
                    ui.end_row();
                    for (net_id, player) in players.into_iter() {
                        let player_status_icon =
                            match (player.is_connected, player.role, player.respawning_at) {
                                (false, _, _) => "🔌",
                                (_, PlayerRole::Builder, _) => "🔨",
                                (_, _, Some((_, RespawnPlayerReason::Finish))) => "★",
                                (_, _, Some((_, RespawnPlayerReason::Death))) => "💀",
                                _ => "",
                            };

                        let columns = [
                            egui::RichText::new(player_status_icon),
                            egui::RichText::new(&player.nickname),
                            egui::RichText::new(format!("{}", player.finishes)),
                            egui::RichText::new(format!("{}", player.deaths)),
                        ];

                        for column in columns {
                            let label = if player_params.current_player_net_id.0 == Some(*net_id) {
                                column.strong()
                            } else {
                                column
                            };
                            ui.add(egui::Label::new(label));
                        }

                        ui.label("");
                        ui.end_row();
                    }
                });
        });
}
