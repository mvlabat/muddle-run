use crate::net::{MatchmakerChannels, MatchmakerState, ServerToConnect, TcpConnectionStatus};
use bevy::{
    ecs::system::{Local, Res, ResMut},
    log,
    utils::HashMap,
};
use bevy_egui::{egui, EguiContext};
use mr_messages_lib::{MatchmakerMessage, Server};
use mr_shared_lib::net::{ConnectionState, ConnectionStatus};
use tokio::sync::mpsc::error::TryRecvError;

pub fn matchmaker_ui(
    mut servers: Local<HashMap<String, Server>>,
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
                *servers = init_list
                    .into_iter()
                    .map(|server| (server.name.clone(), server))
                    .collect();
            }
            Ok(MatchmakerMessage::ServerUpdated(server)) => {
                log::debug!("Server updated: {:?}", server);
                servers.insert(server.name.clone(), server);
            }
            Ok(MatchmakerMessage::ServerRemoved(server_name)) => {
                log::debug!("Server removed: {:?}", server_name);
                servers.remove(&server_name);
            }
            Err(TryRecvError::Empty) => break,
            Err(TryRecvError::Disconnected) => {
                panic!("Failed to read from a channel (matchmaker messages)")
            }
        }
    }

    if !matches!(matchmaker_state.status, TcpConnectionStatus::Connected) {
        servers.clear();
    }

    if !matches!(connection_state.status(), ConnectionStatus::Uninitialized)
        || server_to_connect.is_some()
    {
        return;
    }

    let window_width = 400.0;
    let window_height = 600.0;

    let ctx = egui_context.ctx();
    egui::CentralPanel::default()
        .frame(egui::Frame::none().fill(egui::Color32::from_black_alpha(200)))
        .show(ctx, |ui| {
            egui::Window::new("connection status")
                .title_bar(false)
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
                .fixed_size(egui::Vec2::new(window_width, window_height))
                .show(ui.ctx(), |ui| match matchmaker_state.status {
                    TcpConnectionStatus::Connected => {
                        egui::ScrollArea::auto_sized().show(ui, |ui| {
                            for (i, server) in servers.values().enumerate() {
                                if i > 0 {
                                    ui.separator();
                                }
                                ui.label(&server.name);
                                ui.label(server.addr.to_string());
                                if ui.button("Connect").clicked() {
                                    *server_to_connect = Some(ServerToConnect(server.clone()));
                                }
                            }
                        });

                        ui.label("Matchmaker server: connected");
                    }
                    TcpConnectionStatus::Connecting => {
                        ui.centered_and_justified(|ui| {
                            ui.style_mut().body_text_style = egui::TextStyle::Heading;
                            ui.label("Connecting...");
                        });
                    }
                    TcpConnectionStatus::Disconnected => {
                        ui.centered_and_justified(|ui| {
                            ui.style_mut().body_text_style = egui::TextStyle::Heading;
                            ui.label("Disconnected");
                        });
                    }
                });
        });
}
