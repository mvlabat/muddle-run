use crate::CurrentPlayerNetId;
use bevy::{ecs::SystemParam, log, prelude::*};
use bevy_networking_turbulence::{NetworkEvent, NetworkResource};
use mr_shared_lib::{
    framebuffer::FrameNumber,
    game::commands::{
        DespawnLevelObject, DespawnPlayer, GameCommands, SpawnLevelObject, SpawnPlayer,
    },
    messages::{
        ClientMessage, ConnectedPlayer, DeltaUpdate, DisconnectedPlayer, PlayerInput, PlayerNetId,
        ReliableServerMessage, StartGame, UnreliableServerMessage,
    },
    player::Player,
    GameTime,
};
use std::{
    collections::{hash_map::Entry, HashMap},
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
                    process_start_game_message(
                        start_game,
                        &mut current_player_net_id,
                        &mut players,
                        &mut update_params,
                    );
                }
                ReliableServerMessage::ConnectedPlayer(connected_player) => {
                    process_connected_player_message(
                        connected_player,
                        &mut players,
                        &mut update_params,
                    );
                }
                ReliableServerMessage::DisconnectedPlayer(disconnected_player) => {
                    process_disconnected_player_message(
                        disconnected_player,
                        &mut players,
                        &mut update_params,
                    );
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

pub fn send_network_updates(time: Res<GameTime>, mut net: ResMut<NetworkResource>) {
    log::trace!("Broadcast updates for frame {}", time.game_frame);
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

fn process_start_game_message(
    start_game: StartGame,
    current_player_net_id: &mut CurrentPlayerNetId,
    players: &mut HashMap<PlayerNetId, Player>,
    update_params: &mut UpdateParams,
) {
    if let Some(start_position) = player_start_position(start_game.net_id, &start_game.game_state) {
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
            start_position,
        });
    } else {
        log::error!("Player's position isn't found in the game state");
    }

    for player in start_game.players {
        if let Some(start_position) = player_start_position(player.net_id, &start_game.game_state) {
            players.insert(
                player.net_id,
                Player {
                    nickname: player.nickname,
                    connected_at: player.connected_at,
                },
            );
            update_params.spawn_player_commands.push(SpawnPlayer {
                net_id: player.net_id,
                start_position,
            });
        } else {
            log::error!(
                "Player ({}) position isn't found in the game state",
                player.net_id.0
            );
        }
    }
    for spawn_level_object in start_game.objects {
        update_params
            .spawn_level_object_commands
            .push(spawn_level_object);
    }
}

fn process_connected_player_message(
    connected_player: ConnectedPlayer,
    players: &mut HashMap<PlayerNetId, Player>,
    update_params: &mut UpdateParams,
) {
    let player_entry = players.entry(connected_player.net_id);
    if let Entry::Occupied(_) = player_entry {
        log::error!("Player ({}) is already spawned", connected_player.net_id.0);
    }
    player_entry.or_insert_with(|| {
        log::info!(
            "A new player ({}) connected: {}",
            connected_player.net_id.0,
            connected_player.nickname
        );
        // TODO: spawn a player only when getting a delta update with it.
        update_params.spawn_player_commands.push(SpawnPlayer {
            net_id: connected_player.net_id,
            start_position: Vec2::zero(),
        });
        Player {
            nickname: connected_player.nickname,
            connected_at: connected_player.connected_at,
        }
    });
}

fn process_disconnected_player_message(
    disconnected_player: DisconnectedPlayer,
    players: &mut HashMap<PlayerNetId, Player>,
    update_params: &mut UpdateParams,
) {
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

fn player_start_position(player_net_id: PlayerNetId, delta_update: &DeltaUpdate) -> Option<Vec2> {
    delta_update
        .players
        .iter()
        .find(|player_state| player_state.net_id == player_net_id)
        .map(|player_state| player_state.position)
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
