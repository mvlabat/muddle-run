use std::net::{IpAddr, SocketAddr};

use bevy::{log, prelude::*};
use bevy_networking_turbulence::{NetworkEvent, NetworkResource};

use crate::CurrentPlayerNetId;
use mr_shared_lib::{
    framebuffer::FrameNumber,
    game::commands::{GameCommands, SpawnLevelObject, SpawnPlayer},
    net::{deserialize, serialize, ClientMessage, PlayerInput, PlayerNetId, ServerMessage},
    player::Player,
};
use std::collections::HashMap;

const DEFAULT_SERVER_PORT: u16 = 3455;

pub fn initiate_connection(mut net: ResMut<NetworkResource>) {
    if net.connections.is_empty() {
        let server_socket_addr = server_addr().expect("cannot find current ip address");

        log::info!("Starting the client");
        net.connect(server_socket_addr);
    }
}

#[derive(Default)]
pub struct NetworkReader {
    network_events: EventReader<NetworkEvent>,
}

pub fn process_network_events(
    _net: Res<NetworkResource>,
    mut state: Local<NetworkReader>,
    network_events: Res<Events<NetworkEvent>>,
    mut current_player_net_id: ResMut<CurrentPlayerNetId>,
    mut players: ResMut<HashMap<PlayerNetId, Player>>,
    mut spawn_player_commands: ResMut<GameCommands<SpawnPlayer>>,
    mut spawn_level_object_commands: ResMut<GameCommands<SpawnLevelObject>>,
) {
    for event in state.network_events.iter(&network_events) {
        match event {
            NetworkEvent::Packet(handle, packet) => {
                let message: ServerMessage = match deserialize(packet) {
                    Ok(message) => message,
                    Err(err) => {
                        log::warn!(
                            "Failed to deserialize message (from [{}]): {:?}",
                            handle,
                            err
                        );
                        continue;
                    }
                };

                log::trace!("Got packet on [{}]: {:?}", handle, message);

                match message {
                    ServerMessage::StartGame(start_game) => {
                        log::info!("Starting the game");
                        log::debug!("{:?}", start_game);
                        current_player_net_id.0 = Some(start_game.net_id);
                        players.insert(
                            start_game.net_id,
                            Player {
                                nickname: start_game.nickname,
                                connected_at: Default::default(),
                            },
                        );
                        for spawn_level_object in start_game.objects {
                            spawn_level_object_commands.push(spawn_level_object);
                        }
                    }
                    ServerMessage::NewPlayer(new_player) => {
                        players.insert(
                            new_player.net_id,
                            Player {
                                nickname: new_player.nickname,
                                connected_at: Default::default(),
                            },
                        );
                    }
                    ServerMessage::DeltaUpdate(_update) => {
                        // TODO: apply delta updates.
                    }
                }
            }
            NetworkEvent::Connected(handle) => {
                log::info!("Connected: {}", handle);
            }
            NetworkEvent::Disconnected(handle) => {
                log::info!("Disconnected: {}", handle);
            }
        }
    }
}

pub fn send_network_updates(mut net: ResMut<NetworkResource>) {
    let (_handle, connection) = match net.connections.iter_mut().next() {
        Some(connection) => connection,
        None => return,
    };
    if let Err(err) = connection.send(
        serialize(&ClientMessage::PlayerInput(PlayerInput {
            frame_number: FrameNumber::new(0),
            direction: Vec2::new(0.0, 0.0),
        }))
        .unwrap()
        .into(),
    ) {
        log::error!(
            "Failed to send message to {:?}: {:?}",
            connection.remote_address(),
            err
        );
    }
}

fn server_addr() -> Option<SocketAddr> {
    let server_port = std::env::var("MUDDLE_SERVER_PORT")
        .ok()
        .or_else(|| std::option_env!("MUDDLE_SERVER_PORT").map(str::to_owned))
        .map(|port| port.parse::<u16>().expect("invalid port"))
        .unwrap_or(DEFAULT_SERVER_PORT);

    let env_ip_addr = std::env::var("MUDDLE_SERVER_PORT")
        .ok()
        .or_else(|| std::option_env!("MUDDLE_SERVER_IP_ADDR").map(str::to_owned));
    if let Some(env_addr) = env_ip_addr {
        return Some(SocketAddr::new(
            env_addr.parse::<IpAddr>().expect("invalid socket address"),
            server_port,
        ));
    }

    #[cfg(not(target_arch = "wasm32"))]
    if let Some(addr) = bevy_networking_turbulence::find_my_ip_address() {
        return Some(SocketAddr::new(addr, server_port));
    }

    None
}