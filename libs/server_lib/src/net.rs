use crate::player_updates::DeferredUpdates;
use bevy::{ecs::SystemParam, log, prelude::*};
use bevy_networking_turbulence::{NetworkEvent, NetworkResource, Packet};
use mr_shared_lib::{
    game::{
        commands::{DespawnPlayer, GameCommands, SpawnLevelObject, SpawnPlayer},
        level::LevelState,
    },
    messages::{
        ClientMessage, ConnectedPlayer, DeltaUpdate, PlayerInput, PlayerNetId,
        ReliableServerMessage, StartGame, UnreliableServerMessage,
    },
    player::{random_name, Player},
    registry::Registry,
    GameTime,
};
use std::{
    collections::HashMap,
    net::{IpAddr, SocketAddr},
};

const SERVER_PORT: u16 = 3455;

pub fn startup(mut net: ResMut<NetworkResource>) {
    let socket_address: SocketAddr = SocketAddr::new(IpAddr::from([0, 0, 0, 0]), SERVER_PORT);
    log::info!("Starting the server");
    net.listen(socket_address);
}

#[derive(Default)]
pub struct NetworkReader {
    network_events: EventReader<NetworkEvent>,
}

pub type PlayerConnections = Registry<PlayerNetId, u32>;

#[derive(SystemParam)]
pub struct UpdateParams<'a> {
    deferred_player_updates: ResMut<'a, DeferredUpdates<PlayerInput>>,
    spawn_player_commands: ResMut<'a, GameCommands<SpawnPlayer>>,
    despawn_player_commands: ResMut<'a, GameCommands<DespawnPlayer>>,
}

pub fn process_network_events(
    mut net: ResMut<NetworkResource>,
    time: Res<GameTime>,
    mut state: ResMut<NetworkReader>,
    network_events: Res<Events<NetworkEvent>>,
    mut player_connections: ResMut<PlayerConnections>,
    mut players: ResMut<HashMap<PlayerNetId, Player>>,
    mut update_params: UpdateParams,
) {
    for event in state.network_events.iter(&network_events) {
        match event {
            NetworkEvent::Connected(handle) => {
                log::info!("New connection: {}", handle);
                let player_net_id = player_connections.register(*handle);
                let nickname = random_name();
                players.insert(
                    player_net_id,
                    Player {
                        nickname,
                        connected_at: time.game_frame,
                    },
                );
                update_params.spawn_player_commands.push(SpawnPlayer {
                    net_id: player_net_id,
                });
            }
            NetworkEvent::Disconnected(handle) => {
                log::info!("Disconnected: {}", handle);
                if let Some(player_net_id) = player_connections.remove_by_value(*handle) {
                    update_params.despawn_player_commands.push(DespawnPlayer {
                        net_id: player_net_id,
                    });
                    players.remove(&player_net_id);
                    update_params.despawn_player_commands.push(DespawnPlayer {
                        net_id: player_net_id,
                    });
                } else {
                    log::error!("A disconnected player wasn't in the connections list");
                }
            }
            _ => {}
        }
    }

    for (handle, connection) in net.connections.iter_mut() {
        let channels = connection.channels().unwrap();
        while let Some(client_message) = channels.recv::<ClientMessage>() {
            log::trace!(
                "ClientMessage received on [{}]: {:?}",
                handle,
                client_message
            );

            let player_net_id = match player_connections.get_id(*handle) {
                Some(id) => id,
                None => {
                    log::error!("A player for handle {} is not registered", handle);
                    break;
                }
            };

            match client_message {
                ClientMessage::PlayerInput(update) => update_params
                    .deferred_player_updates
                    .push(player_net_id, update),
            }
        }

        while let Some(_) = channels.recv::<ReliableServerMessage>() {
            log::error!("Unexpected ReliableServerMessage received on [{}]", handle);
        }
        while let Some(_) = channels.recv::<UnreliableServerMessage>() {
            log::error!(
                "Unexpected UnreliableServerMessage received on [{}]",
                handle
            );
        }
    }
}

pub fn send_network_updates(
    mut net: ResMut<NetworkResource>,
    time: Res<GameTime>,
    player_connections: Res<PlayerConnections>,
    level_state: Res<LevelState>,
    players: Res<HashMap<PlayerNetId, Player>>,
) {
    for (&connection_player_net_id, &connection_handle) in player_connections.iter() {
        for (&player_net_id, player) in players.iter() {
            if player.connected_at != time.game_frame {
                continue;
            }

            if connection_player_net_id == player_net_id {
                // TODO: prepare the update in another system.
                let result = net.send_message(
                    connection_handle,
                    ReliableServerMessage::StartGame(StartGame {
                        net_id: connection_player_net_id,
                        nickname: player.nickname.clone(),
                        objects: level_state
                            .objects
                            .iter()
                            .map(|level_object| SpawnLevelObject {
                                object: level_object.clone(),
                                frame_number: time.game_frame,
                            })
                            .collect(),
                        players: Vec::new(),
                        game_state: DeltaUpdate {
                            players: Vec::new(),
                            confirmed_actions: Vec::new(),
                            frame_number: time.game_frame,
                        },
                    }),
                );
                if let Err(err) = result {
                    log::error!("Failed to send a message: {:?}", err);
                }
            } else {
                broadcast_message_to_others(
                    &mut net,
                    &player_connections,
                    connection_handle,
                    &ReliableServerMessage::ConnectedPlayer(ConnectedPlayer {
                        net_id: connection_player_net_id,
                        connected_at: time.game_frame,
                        nickname: player.nickname.clone(),
                    }),
                );
            }
        }
    }
}

fn broadcast_message_to_others(
    net: &mut NetworkResource,
    player_connections: &PlayerConnections,
    exluded_connection_handle: u32,
    message: &ReliableServerMessage,
) {
    for (_player_net_id, &connection_handle) in player_connections.iter() {
        if connection_handle == exluded_connection_handle {
            continue;
        }

        if let Err(err) = net.send_message(connection_handle, message.clone()) {
            log::error!("Failed to send a message: {:?}", err);
        }
    }
}
