use crate::player_updates::DeferredUpdates;
use bevy::{ecs::ResMut, log, prelude::*};
use bevy_networking_turbulence::{NetworkEvent, NetworkResource, Packet};
use mr_shared_lib::{
    game::{commands::SpawnLevelObject, level::LevelState},
    net::{
        deserialize, serialize, ClientMessage, NewPlayer, PlayerInput, PlayerNetId, ServerMessage,
        StartGame,
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

pub fn process_network_events(
    mut _net: ResMut<NetworkResource>,
    mut state: ResMut<NetworkReader>,
    network_events: Res<Events<NetworkEvent>>,
    mut player_connections: ResMut<PlayerConnections>,
    mut deferred_player_updates: ResMut<DeferredUpdates<PlayerInput>>,
) {
    for event in state.network_events.iter(&network_events) {
        match event {
            NetworkEvent::Packet(handle, packet) => {
                let message: ClientMessage = match deserialize(packet) {
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

                let player_net_id = match player_connections.get_id(*handle) {
                    Some(id) => id,
                    None => {
                        log::error!("A player for handle {} is not registered", handle);
                        continue;
                    }
                };

                match message {
                    ClientMessage::PlayerInput(update) => {
                        deferred_player_updates.push(player_net_id, update)
                    }
                }
            }
            NetworkEvent::Connected(handle) => {
                log::info!("New connection: {}", handle);
                let _player_net_id = player_connections.register(*handle);
            }
            NetworkEvent::Disconnected(handle) => {
                log::info!("Disconnected: {}", handle);
                player_connections.remove_by_value(*handle);
            }
        }
    }
}

pub fn send_network_updates(
    time: Res<GameTime>,
    player_connections: Res<PlayerConnections>,
    level_state: Res<LevelState>,
    mut players: ResMut<HashMap<PlayerNetId, Player>>,
    mut net: ResMut<NetworkResource>,
) {
    for (&player_net_id, &connection_handle) in player_connections.iter() {
        // TODO: find a better place for initializing players.
        if !players.contains_key(&player_net_id) {
            let nickname = random_name();
            log::info!(
                "A new player ({}) has connected: {}",
                player_net_id.0,
                nickname
            );
            players.insert(
                player_net_id,
                Player {
                    nickname: nickname.clone(),
                    connected_at: time.game_frame,
                },
            );
            send_message(
                &mut net,
                connection_handle,
                &ServerMessage::StartGame(StartGame {
                    net_id: player_net_id,
                    nickname: nickname.clone(),
                    objects: level_state
                        .objects
                        .iter()
                        .map(|level_object| SpawnLevelObject {
                            object: level_object.clone(),
                        })
                        .collect(),
                }),
            );

            broadcast_message_to_others(
                &mut net,
                &player_connections,
                connection_handle,
                &ServerMessage::NewPlayer(NewPlayer {
                    net_id: player_net_id,
                    nickname,
                }),
            );
        }
    }
}

fn send_message(net: &mut NetworkResource, connection_handle: u32, message: &ServerMessage) {
    match serialize(message) {
        Ok(bytes) => net
            .send(connection_handle, Packet::from(bytes))
            .unwrap_or_else(|err| log::error!("Failed to send a message: {:?}", err)),
        Err(err) => log::error!("Failed to serialize a message: {:?}", err),
    };
}

fn broadcast_message_to_others(
    net: &mut NetworkResource,
    player_connections: &PlayerConnections,
    exluded_connection_handle: u32,
    message: &ServerMessage,
) {
    let packet = match serialize(message) {
        Ok(bytes) => Packet::from(bytes),
        Err(err) => {
            log::error!("Failed to serialize a message: {:?}", err);
            return;
        }
    };
    for (_player_net_id, &connection_handle) in player_connections.iter() {
        if connection_handle == exluded_connection_handle {
            continue;
        }

        if let Err(err) = net.send(connection_handle, packet.clone()) {
            log::error!("Failed to send a message: {:?}", err);
        }
    }
}
