use crate::helpers::PlayerParams;
use bevy::{
    ecs::system::{Local, Res, ResMut},
    input::{keyboard::KeyCode, Input},
};
use bevy_egui::{egui, EguiContext};
use mr_shared_lib::{
    messages::RespawnPlayerReason, player::PlayerRole, simulations_per_second, GameTime,
};

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

pub struct LeaderboardState {
    show: bool,
}

impl Default for LeaderboardState {
    fn default() -> Self {
        Self { show: true }
    }
}

pub fn leaderboard_ui(
    mut state: Local<LeaderboardState>,
    keyboard_input: Res<Input<KeyCode>>,
    egui_context: ResMut<EguiContext>,
    player_params: PlayerParams,
) {
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
        .show(egui_context.ctx(), |ui| {
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
                        ui.label(player_status_icon);
                        if player_params.current_player_net_id.0 == Some(*net_id) {
                            ui.add(egui::Label::new(&player.nickname).strong());
                        } else {
                            ui.label(&player.nickname);
                        }
                        ui.label(format!("{}", player.finishes));
                        ui.label(format!("{}", player.deaths));
                        ui.label("");
                        ui.end_row();
                    }
                });
        });
}
