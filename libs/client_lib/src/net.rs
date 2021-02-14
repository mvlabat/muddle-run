use crate::CurrentPlayerNetId;
use bevy::{ecs::SystemParam, log, prelude::*};
use bevy_networking_turbulence::{NetworkEvent, NetworkResource};
use mr_shared_lib::{
    framebuffer::FrameNumber,
    game::commands::{
        DespawnLevelObject, DespawnPlayer, GameCommands, SpawnLevelObject, SpawnPlayer,
    },
    messages::{
        ClientMessage, PlayerInput, PlayerNetId, ReliableServerMessage, UnreliableServerMessage,
    },
    player::Player,
};
use std::{
    collections::HashMap,
    net::{IpAddr, SocketAddr},
};

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

#[derive(SystemParam)]
pub struct UpdateParams<'a> {
    spawn_level_object_commands: ResMut<'a, GameCommands<SpawnLevelObject>>,
    despawn_level_object_commands: ResMut<'a, GameCommands<DespawnLevelObject>>,
    spawn_player_commands: ResMut<'a, GameCommands<SpawnPlayer>>,
    despawn_player_commands: ResMut<'a, GameCommands<DespawnPlayer>>,
}

pub fn process_network_events(
    mut net: ResMut<NetworkResource>,
    mut state: Local<NetworkReader>,
    network_events: Res<Events<NetworkEvent>>,
    mut current_player_net_id: ResMut<CurrentPlayerNetId>,
    mut players: ResMut<HashMap<PlayerNetId, Player>>,
    mut update_params: UpdateParams,
) {
    for event in state.network_events.iter(&network_events) {
        match event {
            NetworkEvent::Connected(handle) => {
                log::info!("Connected: {}", handle);
            }
            NetworkEvent::Disconnected(handle) => {
                log::info!("Disconnected: {}", handle);
            }
            _ => {}
        }
    }

    for (handle, connection) in net.connections.iter_mut() {
        let channels = connection.channels().unwrap();

        while let Some(message) = channels.recv::<UnreliableServerMessage>() {
            log::trace!(
                "UnreliableServerMessage received on [{}]: {:?}",
                handle,
                message
            );

            match message {
                UnreliableServerMessage::DeltaUpdate(_update) => {
                    // TODO: apply delta updates.
                }
            }
        }

        while let Some(message) = channels.recv::<ReliableServerMessage>() {
            log::trace!(
                "ReliableServerMessage received on [{}]: {:?}",
                handle,
                message
            );

            match message {
                ReliableServerMessage::StartGame(start_game) => {
                    if current_player_net_id.0 == Some(start_game.net_id) {
                        continue;
                    }
                    log::info!("Starting the game");
                    log::debug!("{:?}", start_game);
                    current_player_net_id.0 = Some(start_game.net_id);
                    players.insert(
                        start_game.net_id,
                        Player {
                            nickname: start_game.nickname,
                            connected_at: start_game.game_state.frame_number,
                        },
                    );
                    update_params.spawn_player_commands.push(SpawnPlayer {
                        net_id: start_game.net_id,
                    });
                    for player in start_game.players {
                        players.insert(
                            player.net_id,
                            Player {
                                nickname: player.nickname,
                                connected_at: player.connected_at,
                            },
                        );
                        update_params.spawn_player_commands.push(SpawnPlayer {
                            net_id: player.net_id,
                        });
                    }
                    for spawn_level_object in start_game.objects {
                        update_params
                            .spawn_level_object_commands
                            .push(spawn_level_object);
                    }
                }
                ReliableServerMessage::ConnectedPlayer(connected_player) => {
                    if !players.contains_key(&connected_player.net_id) {
                        log::info!(
                            "A new player ({}) connected: {}",
                            connected_player.net_id.0,
                            connected_player.nickname
                        );
                        players.insert(
                            connected_player.net_id,
                            Player {
                                nickname: connected_player.nickname,
                                connected_at: connected_player.connected_at,
                            },
                        );
                        update_params.spawn_player_commands.push(SpawnPlayer {
                            net_id: connected_player.net_id,
                        });
                    } else {
                        log::error!("Player ({}) is already spawned", connected_player.net_id.0);
                    }
                }
                ReliableServerMessage::DisconnectedPlayer(disconnected_player) => {
                    if let Some(player) = players.remove(&disconnected_player.net_id) {
                        log::info!(
                            "A player ({}) disconnected: {}",
                            disconnected_player.net_id.0,
                            player.nickname
                        );
                        update_params.despawn_player_commands.push(DespawnPlayer {
                            net_id: disconnected_player.net_id,
                        });
                    } else {
                        log::error!(
                            "Unknown player with net id {}",
                            disconnected_player.net_id.0
                        );
                    }
                }
                ReliableServerMessage::SpawnLevelObject(spawn_level_object) => {
                    update_params
                        .spawn_level_object_commands
                        .push(spawn_level_object);
                }
                ReliableServerMessage::DespawnLevelObject(despawn_level_object) => {
                    update_params
                        .despawn_level_object_commands
                        .push(despawn_level_object);
                }
            }
        }

        while channels.recv::<ClientMessage>().is_some() {
            log::error!("Unexpected ClientMessage received on [{}]", handle);
        }
    }
}

pub fn send_network_updates(mut net: ResMut<NetworkResource>) {
    let (connection_handle, address) = match net.connections.iter_mut().next() {
        Some((&handle, connection)) => (handle, connection.remote_address()),
        None => return,
    };
    let result = net.send_message(
        connection_handle,
        ClientMessage::PlayerInput(PlayerInput {
            frame_number: FrameNumber::new(0),
            direction: Vec2::new(0.0, 0.0),
        }),
    );
    if let Err(err) = result {
        log::error!("Failed to send a message to {:?}: {:?}", address, err);
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
