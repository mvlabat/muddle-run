use crate::net::{MatchmakerChannels, MatchmakerState, ServerToConnect, TcpConnectionStatus};
use bevy::{
    ecs::system::{Local, Res, ResMut},
    log,
    utils::{HashMap, Instant},
};
use bevy_egui::{egui, EguiContext};
use mr_messages_lib::{MatchmakerMessage, Server};
use mr_shared_lib::net::{ConnectionState, ConnectionStatus};
use std::ops::{Add, Mul, Sub};
use tokio::sync::mpsc::error::TryRecvError;

#[derive(Default)]
pub struct MatchmakerUiState {
    observed_empty_servers_at: Option<Instant>,
    servers: HashMap<String, Server>,
    selected: Option<String>,
}

pub fn matchmaker_ui(
    mut matchmaker_ui_state: Local<MatchmakerUiState>,
    egui_context: ResMut<EguiContext>,
    matchmaker_state: Option<ResMut<MatchmakerState>>,
    matchmaker_channels: Option<ResMut<MatchmakerChannels>>,
    mut server_to_connect: ResMut<Option<ServerToConnect>>,
    connection_state: Res<ConnectionState>,
) {
    #[cfg(feature = "profiler")]
    puffin::profile_function!();

    let (matchmaker_state, mut matchmaker_channels) =
        match matchmaker_state.zip(matchmaker_channels) {
            Some(resources) => resources,
            None => return,
        };

    loop {
        match matchmaker_channels.message_rx.try_recv() {
            Ok(MatchmakerMessage::Init(init_list)) => {
                log::debug!("Initialize servers list: {:?}", init_list);
                matchmaker_ui_state.servers = init_list
                    .into_iter()
                    .map(|server| (server.name.clone(), server))
                    .collect();
            }
            Ok(MatchmakerMessage::ServerUpdated(server)) => {
                log::debug!("Server updated: {:?}", server);
                matchmaker_ui_state
                    .servers
                    .insert(server.name.clone(), server);
            }
            Ok(MatchmakerMessage::ServerRemoved(server_name)) => {
                log::debug!("Server removed: {:?}", server_name);
                matchmaker_ui_state.servers.remove(&server_name);
            }
            Err(TryRecvError::Empty) => break,
            Err(TryRecvError::Disconnected) => {
                panic!("Failed to read from a channel (matchmaker messages)")
            }
        }
    }

    if !matches!(matchmaker_state.status, TcpConnectionStatus::Connected) {
        matchmaker_ui_state.servers.clear();
    }

    if !matches!(connection_state.status(), ConnectionStatus::Uninitialized)
        || server_to_connect.is_some()
    {
        return;
    }

    if matchmaker_ui_state.servers.is_empty()
        && matches!(matchmaker_state.status, TcpConnectionStatus::Connected)
        && matchmaker_ui_state.observed_empty_servers_at.is_none()
    {
        matchmaker_ui_state.observed_empty_servers_at = Some(Instant::now());
    }

    let window_width = 400.0;
    let window_height = 600.0;

    let ctx = egui_context.ctx();
    egui::CentralPanel::default()
        .frame(egui::Frame::none().fill(egui::Color32::from_black_alpha(200)))
        .show(ctx, |ui| {
            ui.spacing_mut().window_padding = egui::Vec2::splat(ui.visuals().window_stroke().width);
            egui::Window::new("Server browser")
                .title_bar(false)
                .collapsible(false)
                .resizable(false)
                .frame(egui::Frame::window(ui.style()))
                .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
                .fixed_size(egui::Vec2::new(window_width, window_height))
                .show(ui.ctx(), |ui| {
                    let MatchmakerUiState {
                        ref observed_empty_servers_at,
                        ref servers,
                        ref mut selected,
                    } = &mut *matchmaker_ui_state;

                    match matchmaker_state.status {
                        TcpConnectionStatus::Connected => {
                            // Header.
                            if servers.is_empty() {
                                let progress = Instant::now()
                                    .duration_since(observed_empty_servers_at.unwrap())
                                    .as_secs_f32()
                                    / 120.0;
                                status_bar(
                                    ui,
                                    "Spinning up a game server (might take a couple of minutes)...",
                                    progress.min(0.95),
                                );
                            } else {
                                status_bar(ui, "Matchmaker server: connected", 1.0);
                            }

                            // Server list.
                            let mut sorted_servers = servers.values().collect::<Vec<_>>();
                            sorted_servers.sort_by(|a, b| a.name.cmp(&b.name));
                            egui::containers::ScrollArea::from_max_height(500.0).show(ui, |ui| {
                                server_list(ui, &sorted_servers, selected);
                            });
                        }
                        TcpConnectionStatus::Connecting | TcpConnectionStatus::Disconnected => {
                            status_bar(ui, "Connecting...", 0.0);
                        }
                    }

                    // Play button.
                    let button_size = egui::Vec2::new(100.0, 30.0);
                    let margin = 10.0;
                    let is_selected = selected
                        .as_ref()
                        .map_or(false, |selected| servers.contains_key(selected));
                    let (outer_rect, _) = ui.allocate_exact_size(
                        egui::Vec2::new(
                            ui.available_size_before_wrap().x,
                            button_size.y + margin * 2.0,
                        ),
                        egui::Sense::hover(),
                    );
                    let button_response = ui.put(
                        egui::Rect::from_min_size(
                            outer_rect.center() - button_size / 2.0,
                            button_size,
                        ),
                        egui::widgets::Button::new("Play").enabled(is_selected),
                    );
                    if button_response.clicked() {
                        *server_to_connect =
                            Some(ServerToConnect(servers[selected.as_ref().unwrap()].clone()));
                    }
                });
        });
}

fn server_list(ui: &mut egui::Ui, servers: &[&Server], selected: &mut Option<String>) {
    for server in servers {
        let is_selected = selected
            .as_ref()
            .map_or(false, |selected| &server.name == selected);

        let padding = 10.0;
        let spacing = 5.0;
        let (outer_rect, response) = ui.allocate_exact_size(
            egui::Vec2::new(ui.available_size_before_wrap_finite().x, 60.0),
            egui::Sense::click(),
        );

        let inner_rect = outer_rect.shrink(padding);

        let fill = if is_selected {
            Some(ui.style().visuals.extreme_bg_color)
        } else if response.hovered() {
            Some(ui.style().visuals.faint_bg_color)
        } else {
            None
        };

        let server_name_galley = ui
            .fonts()
            .layout_no_wrap(egui::TextStyle::Heading, server.name.clone());
        let server_name_cursor = inner_rect.min;

        let players_galley = ui.fonts().layout_no_wrap(
            egui::TextStyle::Body,
            format!(
                "Players: {}/{}",
                server.player_count, server.player_capacity
            ),
        );
        let players_cursor =
            inner_rect.min + egui::Vec2::new(0.0, server_name_galley.size.y + spacing);

        if let Some(fill) = fill {
            ui.painter().rect_filled(outer_rect, 0.0, fill);
        }
        ui.painter().line_segment(
            [
                egui::Pos2::new(outer_rect.min.x, outer_rect.max.y),
                egui::Pos2::new(outer_rect.max.x, outer_rect.max.y),
            ],
            ui.style().visuals.window_stroke(),
        );
        ui.painter().galley(
            server_name_cursor,
            server_name_galley,
            ui.visuals().text_color(),
        );
        ui.painter()
            .galley(players_cursor, players_galley, ui.visuals().text_color());

        if response.clicked() {
            *selected = Some(server.name.clone());
        }
        response.on_hover_cursor(egui::CursorIcon::PointingHand);
    }
}

fn status_bar(ui: &mut egui::Ui, label: impl ToString, progress: f32) {
    let desired_width = ui.available_size_before_wrap_finite().x;
    let height = ui.spacing().interact_size.y;
    let (outer_rect, _response) =
        ui.allocate_exact_size(egui::Vec2::new(desired_width, height), egui::Sense::hover());

    if progress > 0.0 {
        let corner_radius = ui.style().visuals.window_corner_radius;
        let size = outer_rect
            .size()
            .mul(egui::Vec2::new(progress, 1.0))
            .max(egui::Vec2::new(corner_radius * 2.0, outer_rect.height()));
        let fill = egui::Color32::from_rgb(23, 98, 3);
        let stroke = egui::Stroke::new(0.0, egui::Color32::TRANSPARENT);

        ui.painter().add(egui::Shape::Rect {
            rect: egui::Rect::from_min_size(outer_rect.min, size),
            corner_radius,
            fill,
            stroke,
        });
        ui.painter().add(egui::Shape::Rect {
            rect: egui::Rect::from_min_size(
                outer_rect.min.add(egui::Vec2::new(0.0, size.y / 2.0)),
                size.mul(egui::Vec2::new(1.0, 0.5)),
            ),
            corner_radius: 0.0,
            fill,
            stroke,
        });
        if size.x < outer_rect.size().sub(egui::Vec2::new(corner_radius, 0.0)).x {
            ui.painter().add(egui::Shape::Rect {
                rect: egui::Rect::from_min_size(
                    outer_rect.min.add(egui::Vec2::new(corner_radius, 0.0)),
                    size.sub(egui::Vec2::new(corner_radius, 0.0)),
                ),
                corner_radius: 0.0,
                fill,
                stroke,
            });
        }
    }

    ui.painter().text(
        outer_rect.center(),
        egui::Align2::CENTER_CENTER,
        label,
        ui.style().body_text_style,
        ui.visuals().text_color(),
    );
}
