use crate::{
    net::ServerToConnect,
    ui::widgets::list_menu::{button_panel, PanelButton},
};
use bevy::{
    ecs::{
        schedule::State,
        system::{Res, ResMut},
    },
    log,
    prelude::NextState,
};
use bevy_egui::{egui, EguiContexts};
use mr_shared_lib::{
    messages::DisconnectReason,
    net::{ConnectionState, ConnectionStatus},
    AppState, GameSessionState,
};

pub fn app_loading_ui(mut egui_contexts: EguiContexts) {
    let window_width = 400.0;
    let window_height = 100.0;

    egui::CentralPanel::default()
        .frame(egui::Frame::none().fill(egui::Color32::from_rgb(47, 47, 47)))
        .show(egui_contexts.ctx_mut(), |ui| {
            ui.style_mut().spacing.window_margin = egui::style::Margin::same(0.0);
            egui::Window::new("app loading ui")
                .frame(egui::Frame::window(ui.style()))
                .title_bar(false)
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
                .fixed_size(egui::Vec2::new(window_width, window_height))
                .show(ui.ctx(), |ui| {
                    let rounding = ui.style().visuals.window_rounding;
                    let fill = egui::Color32::from_rgb(23, 98, 3);
                    let stroke = egui::Stroke::new(1.0, egui::Color32::from_rgb(60, 60, 60));

                    ui.painter().add(egui::Shape::Rect(egui::epaint::RectShape {
                        rect: ui.max_rect(),
                        rounding,
                        fill,
                        stroke,
                    }));
                    ui.centered_and_justified(|ui| {
                        ui.style_mut().override_text_style = Some(egui::TextStyle::Heading);
                        ui.label("Loading shaders...");
                    });
                });
        });
}

pub fn connection_status_overlay_system(
    mut next_app_state: ResMut<NextState<AppState>>,
    mut next_game_session_state: ResMut<NextState<GameSessionState>>,
    game_session_state: Res<State<GameSessionState>>,
    mut egui_contexts: EguiContexts,
    mut connection_state: ResMut<ConnectionState>,
    mut server_to_connect: ResMut<ServerToConnect>,
) {
    #[cfg(feature = "profiler")]
    puffin::profile_function!();
    if matches!(
        connection_state.status(),
        ConnectionStatus::Uninitialized | ConnectionStatus::Connected
    ) && server_to_connect.is_none()
        && game_session_state.0 != GameSessionState::Paused
    {
        // We don't display the overlay if we haven't even started connecting or the
        // connection is ok. We do want to show it when we have a server to
        // connect or we didn't have any updates from the server for a while (to let a
        // player disconnect).
        return;
    }

    let window_width = 400.0;
    let window_height = 100.0;

    let ctx = egui_contexts.ctx_mut();
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
                        let text = match (&game_session_state.0, connection_state.status()) {
                            (GameSessionState::Paused, _) => "No updates from the server...",
                            (
                                _,
                                ConnectionStatus::Uninitialized | ConnectionStatus::Initialized,
                            ) => "Connecting...",
                            (
                                _,
                                ConnectionStatus::Connecting
                                | ConnectionStatus::Handshaking
                                | ConnectionStatus::Connected,
                            ) => "Handshaking...",
                            (
                                _,
                                ConnectionStatus::Disconnecting(_) | ConnectionStatus::Disconnected,
                            ) => "Disconnected",
                        };

                        ui.style_mut().override_text_style = Some(egui::TextStyle::Heading);
                        ui.label(text);
                    });

                    let button_label = if game_session_state.0 == GameSessionState::Paused {
                        "Disconnect"
                    } else {
                        "Cancel"
                    };
                    let [response] = button_panel(
                        ui,
                        100.0,
                        [PanelButton::new(egui::Button::new(button_label))],
                    );
                    if response.clicked() {
                        **server_to_connect = None;
                        connection_state
                            .set_status(ConnectionStatus::Disconnecting(DisconnectReason::Aborted));
                        log::info!("Changing the app state to {:?}", AppState::MainMenu);
                        next_app_state.set(AppState::MainMenu);
                        log::info!(
                            "Changing the game session state to {:?}",
                            GameSessionState::Loading
                        );
                        next_game_session_state.set(GameSessionState::Loading);
                    }
                });
        });
}
