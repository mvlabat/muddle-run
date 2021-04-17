use crate::player_updates::DeferredUpdates;
use bevy::{ecs::system::SystemParam, log, prelude::*};
use bevy_networking_turbulence::{NetworkEvent, NetworkResource};
use mr_shared_lib::{
    game::{
        commands::{DespawnPlayer, GameCommands, SpawnLevelObject, SpawnPlayer},
        components::{PlayerDirection, Position, Spawned},
        level::LevelState,
    },
    messages::{
        ConnectedPlayer, DeltaUpdate, DisconnectedPlayer, Message, PlayerInput, PlayerNetId,
        PlayerState, ReliableClientMessage, ReliableServerMessage, StartGame,
        UnreliableClientMessage, UnreliableServerMessage,
    },
    net::{ConnectionState, ConnectionStatus, SessionId},
    player::{random_name, Player},
    registry::{EntityRegistry, Registry},
    GameTime, COMPONENT_FRAMEBUFFER_LIMIT,
};
use std::{
    collections::HashMap,
    net::{IpAddr, SocketAddr},
    time::{Duration, Instant},
};

const SERVER_PORT: u16 = 3455;

pub fn startup(mut net: ResMut<NetworkResource>) {
    let socket_address: SocketAddr = SocketAddr::new(IpAddr::from([0, 0, 0, 0]), SERVER_PORT);
    log::info!("Starting the server");
    net.listen(socket_address);
}

pub type PlayerConnections = Registry<PlayerNetId, u32>;

#[derive(SystemParam)]
pub struct UpdateParams<'a> {
    deferred_player_updates: ResMut<'a, DeferredUpdates<PlayerInput>>,
    spawn_player_commands: ResMut<'a, GameCommands<SpawnPlayer>>,
    despawn_player_commands: ResMut<'a, GameCommands<DespawnPlayer>>,
}

#[derive(SystemParam)]
pub struct NetworkParams<'a> {
    net: ResMut<'a, NetworkResource>,
    connection_states: ResMut<'a, HashMap<u32, ConnectionState>>,
    player_connections: ResMut<'a, PlayerConnections>,
    new_player_connections: ResMut<'a, Vec<(PlayerNetId, u32)>>,
}

pub fn process_network_events(
    time: Res<GameTime>,
    mut prev_time: Local<GameTime>,
    mut players: ResMut<HashMap<PlayerNetId, Player>>,
    mut network_events: EventReader<NetworkEvent>,
    mut network_params: NetworkParams,
    mut update_params: UpdateParams,
) {
    log::trace!("Processing network updates (frame: {})", time.frame_number);

    // Processing connection events.
    for event in network_events.iter() {
        match event {
            NetworkEvent::Connected(handle) => {
                log::info!("New connection: {}", handle);
                if network_params.player_connections.get_id(*handle).is_none() {
                    network_params.player_connections.register(*handle);
                }
                let connection_state = network_params.connection_states.entry(*handle).or_default();

                if matches!(
                    connection_state.status(),
                    ConnectionStatus::Connected | ConnectionStatus::Disconnecting
                ) {
                    log::warn!("Received a Connected event from a connection that is already connected (or being disconnected). That probably means that the clean-up wasn't properly finished");
                }
                match connection_state.status() {
                    ConnectionStatus::Disconnecting | ConnectionStatus::Disconnected => {
                        connection_state.set_status(ConnectionStatus::Uninitialized);
                        connection_state.session_id += SessionId::new(1);
                    }
                    _ => {}
                };
            }
            NetworkEvent::Disconnected(handle) => {
                log::info!("Disconnected: {}", handle);
                let connection_state = network_params
                    .connection_states
                    .get_mut(&handle)
                    .expect("Expected a connection when receiving a Disconnect event");
                if matches!(
                    connection_state.status(),
                    ConnectionStatus::Disconnecting | ConnectionStatus::Disconnected
                ) {
                    log::info!("Received a Disconnected event for a player that's already disconnected, skipped");
                    continue;
                }
                connection_state.set_status(ConnectionStatus::Disconnecting);
            }
            _ => {}
        }
    }

    // Reading message channels.
    for (handle, connection) in network_params.net.connections.iter_mut() {
        let channels = connection.channels().unwrap();

        while let Some(client_message) = channels.recv::<Message<ReliableClientMessage>>() {
            log::trace!(
                "ReliableClientMessage received on [{}]: {:?}",
                handle,
                client_message
            );

            match client_message.message {
                // NOTE: before adding new messages, make sure to ignore them if connection status
                // is not `Connected`.
                ReliableClientMessage::Handshake => {
                    log::info!("Player handshake: {}", handle);
                    let player_net_id = network_params
                        .player_connections
                        .get_id(*handle)
                        // At the moment of writing this we never removed player connections.
                        .expect("Expected a registered player id for a connection");

                    let connection_state =
                        network_params.connection_states.entry(*handle).or_default();

                    if matches!(
                        connection_state.status(),
                        ConnectionStatus::Connected | ConnectionStatus::Disconnecting
                    ) {
                        log::warn!("Received a Connected event from a connection that is already connected (or being disconnected). That probably means that the clean-up wasn't properly finished");
                    }
                    match connection_state.status() {
                        ConnectionStatus::Disconnecting
                        | ConnectionStatus::Disconnected
                        | ConnectionStatus::Connected
                        | ConnectionStatus::Handshaking => {
                            connection_state.session_id += SessionId::new(1);
                        }
                        ConnectionStatus::Uninitialized => {}
                    };

                    connection_state.set_status(ConnectionStatus::Handshaking);

                    network_params
                        .new_player_connections
                        .push((player_net_id, *handle));

                    let nickname = random_name();
                    players.insert(player_net_id, Player { nickname });
                    update_params.spawn_player_commands.push(SpawnPlayer {
                        net_id: player_net_id,
                        start_position: Vec2::ZERO,
                        is_player_frame_simulated: false,
                    });
                    // Add an initial update to have something to extrapolate from.
                    update_params.deferred_player_updates.push(
                        player_net_id,
                        PlayerInput {
                            frame_number: time.frame_number,
                            direction: Vec2::ZERO,
                        },
                    );
                }
            }
        }

        while let Some(client_message) = channels.recv::<Message<UnreliableClientMessage>>() {
            log::trace!(
                "UnreliableClientMessage received on [{}]: {:?}",
                handle,
                client_message
            );

            let (player_net_id, connection_state) = match (
                network_params.player_connections.get_id(*handle),
                network_params.connection_states.get_mut(handle),
            ) {
                (Some(id), Some(connection_state)) => (id, connection_state),
                _ => {
                    log::error!("A player for handle {} is not registered", handle);
                    break;
                }
            };

            if !matches!(connection_state.status(), ConnectionStatus::Connected) {
                log::warn!(
                    "Ignoring a message for a player ({}): expected connection status is {:?}, but it's {:?}",
                    player_net_id.0,
                    ConnectionStatus::Connected,
                    connection_state.status()
                );
            }

            if client_message.session_id != connection_state.session_id {
                log::warn!(
                    "Ignoring a message for a player ({}): sent session id {} doesn't match {}",
                    player_net_id.0,
                    client_message.session_id,
                    connection_state.session_id
                );
                continue;
            }
            let client_message = client_message.message;

            match client_message {
                UnreliableClientMessage::PlayerUpdate(update) => {
                    if let Err(err) = connection_state.acknowledge_incoming(update.frame_number) {
                        // TODO: disconnect players.
                        log::error!("Failed to acknowledge an incoming packet (update frame: {}, current frame: {}): {:?}", update.frame_number, time.frame_number, err);
                        break;
                    }
                    if let (Some(frame_number), ack_bit_set) = update.acknowledgments {
                        if let Err(err) = connection_state
                            .apply_outgoing_acknowledgements(frame_number, ack_bit_set)
                        {
                            // TODO: disconnect players.
                            log::error!(
                                "Failed to apply outgoing packet acknowledgments (update frame: {}, current frame: {}): {:?}",
                                update.frame_number,
                                time.frame_number,
                                err
                            );
                            break;
                        }
                    }
                    for input in update.inputs {
                        update_params
                            .deferred_player_updates
                            .push(player_net_id, input);
                    }
                }
            }
        }

        while channels.recv::<Message<ReliableServerMessage>>().is_some() {
            log::error!("Unexpected ReliableServerMessage received on [{}]", handle);
        }
        while channels
            .recv::<Message<UnreliableServerMessage>>()
            .is_some()
        {
            log::error!(
                "Unexpected UnreliableServerMessage received on [{}]",
                handle
            );
        }
    }

    if *prev_time != *time {
        disconnect_players(&time, &mut network_params, &mut update_params);
    }
    *prev_time = time.clone();
}

fn disconnect_players(
    time: &GameTime,
    network_params: &mut NetworkParams,
    update_params: &mut UpdateParams,
) {
    // Disconnecting players that have been failing to deliver updates for some time.
    for (connection_handle, connection_state) in network_params.connection_states.iter_mut() {
        if let ConnectionStatus::Uninitialized | ConnectionStatus::Disconnected =
            connection_state.status()
        {
            continue;
        }

        // We might have marked a client as `Disconnecting` when processing connection events.
        if !matches!(connection_state.status(), ConnectionStatus::Disconnecting) {
            let (last_incoming_frame, _) = connection_state.incoming_acknowledgments();
            if let Some(last_incoming_frame) = last_incoming_frame {
                // If the difference between last incoming frame and the current one is more
                // than 5 secs, we disconnect the client. Both lagging behind and being far ahead
                // isn't right.
                if (time.frame_number.value() as i32 - last_incoming_frame.value() as i32).abs()
                    > (COMPONENT_FRAMEBUFFER_LIMIT / 2) as i32
                {
                    connection_state.set_status(ConnectionStatus::Disconnecting);
                }
            } else if Instant::now().duration_since(connection_state.status_updated_at())
                > Duration::from_secs(5)
            {
                connection_state.set_status(ConnectionStatus::Disconnecting);
            }
        }

        // We expect that this status lives only during this frame so despawning will be queued
        // only once. The status MUST be changed to `Disconnected` when broadcasting the updates.
        if let ConnectionStatus::Disconnecting = connection_state.status() {
            if let Some(player_net_id) =
                network_params.player_connections.get_id(*connection_handle)
            {
                log::debug!(
                    "Adding a DespawnPlayer command (frame: {}, player: {})",
                    time.frame_number,
                    player_net_id.0
                );
                update_params.despawn_player_commands.push(DespawnPlayer {
                    net_id: player_net_id,
                    frame_number: time.frame_number,
                });
            } else {
                log::error!("A disconnected player wasn't in the connections list");
            }
        }
    }
}

pub fn send_network_updates(
    mut network_params: NetworkParams,
    time: Res<GameTime>,
    level_state: Res<LevelState>,
    players: Res<HashMap<PlayerNetId, Player>>,
    player_entities: Query<(Entity, &Position, &PlayerDirection, &Spawned)>,
    players_registry: Res<EntityRegistry<PlayerNetId>>,
) {
    log::trace!("Sending network updates (frame: {})", time.frame_number);

    broadcast_start_game_messages(
        &mut network_params,
        &time,
        &level_state,
        &players,
        &player_entities,
        &players_registry,
    );

    broadcast_disconnected_players(&mut network_params);

    for (&_connection_player_net_id, &connection_handle) in network_params.player_connections.iter()
    {
        let connection_state = network_params
            .connection_states
            .get_mut(&connection_handle)
            .expect("Expected a connection state for a connected player");

        if !matches!(connection_state.status(), ConnectionStatus::Connected) {
            continue;
        }

        broadcast_delta_update_messages(
            &mut network_params.net,
            &time,
            &players,
            &player_entities,
            &players_registry,
            connection_handle,
            connection_state,
        );

        broadcast_new_player_messages(
            &mut network_params.net,
            &network_params.new_player_connections,
            &players,
            connection_handle,
            connection_state,
        )
    }
}

fn broadcast_disconnected_players(network_params: &mut NetworkParams) {
    let mut disconnected_players = Vec::new();
    for (&connection_player_net_id, &connection_handle) in network_params.player_connections.iter()
    {
        let connection_state = network_params
            .connection_states
            .get_mut(&connection_handle)
            .expect("Expected a connection state for a connected player");

        if !matches!(connection_state.status(), ConnectionStatus::Disconnecting) {
            continue;
        }

        disconnected_players.push(connection_player_net_id);

        if let Err(err) = network_params.net.send_message(
            connection_handle,
            Message {
                session_id: connection_state.session_id,
                message: ReliableServerMessage::Disconnect,
            },
        ) {
            log::error!("Failed to send a message: {:?}", err);
        }
        log::debug!(
            "Marking Player's ({}) connection as Disconnected",
            connection_player_net_id.0
        );
        connection_state.set_status(ConnectionStatus::Disconnected);
    }

    for (_, &connection_handle) in network_params.player_connections.iter() {
        let connection_state = network_params
            .connection_states
            .get_mut(&connection_handle)
            .expect("Expected a connection state for a connected player");

        if !matches!(connection_state.status(), ConnectionStatus::Connected) {
            continue;
        }

        for disconnected_player in &disconnected_players {
            if let Err(err) = network_params.net.send_message(
                connection_handle,
                Message {
                    session_id: connection_state.session_id,
                    message: ReliableServerMessage::DisconnectedPlayer(DisconnectedPlayer {
                        net_id: *disconnected_player,
                    }),
                },
            ) {
                log::error!("Failed to send a message: {:?}", err);
            }
        }
    }
}

fn broadcast_delta_update_messages(
    net: &mut NetworkResource,
    time: &GameTime,
    players: &HashMap<PlayerNetId, Player>,
    player_entities: &Query<(Entity, &Position, &PlayerDirection, &Spawned)>,
    players_registry: &EntityRegistry<PlayerNetId>,
    connection_handle: u32,
    connection_state: &mut ConnectionState,
) {
    // Checks that a player that we broadcast the message to is connected.
    if !matches!(connection_state.status(), ConnectionStatus::Connected) {
        return;
    }

    let message = UnreliableServerMessage::DeltaUpdate(DeltaUpdate {
        frame_number: time.frame_number,
        acknowledgments: connection_state.incoming_acknowledgments(),
        players: players
            .iter()
            .filter_map(|(&player_net_id, _player)| {
                players_registry
                    .get_entity(player_net_id)
                    .and_then(|entity| {
                        create_player_state(
                            player_net_id,
                            &time,
                            connection_state,
                            entity,
                            &player_entities,
                        )
                    })
            })
            .collect(),
        confirmed_actions: vec![],
    });

    if let Err(err) = net.send_message(
        connection_handle,
        Message {
            session_id: connection_state.session_id,
            message,
        },
    ) {
        log::error!("Failed to send a message: {:?}", err);
    }

    connection_state.add_outgoing_packet(time.frame_number, Instant::now());
}

fn broadcast_new_player_messages(
    net: &mut NetworkResource,
    new_player_connections: &[(PlayerNetId, u32)],
    players: &HashMap<PlayerNetId, Player>,
    connection_handle: u32,
    connection_state: &mut ConnectionState,
) {
    // Broadcasting updates about new connected players.
    for (connected_player_net_id, _connection_handle) in new_player_connections.iter() {
        let player = players
            .get(&connected_player_net_id)
            .expect("Expected a registered Player");
        let message = ReliableServerMessage::ConnectedPlayer(ConnectedPlayer {
            net_id: *connected_player_net_id,
            nickname: player.nickname.clone(),
        });

        if let Err(err) = net.send_message(
            connection_handle,
            Message {
                session_id: connection_state.session_id,
                message,
            },
        ) {
            log::error!("Failed to send a message: {:?}", err);
        }
    }
}

fn broadcast_start_game_messages(
    network_params: &mut NetworkParams,
    time: &GameTime,
    level_state: &LevelState,
    players: &HashMap<PlayerNetId, Player>,
    player_entities: &Query<(Entity, &Position, &PlayerDirection, &Spawned)>,
    players_registry: &EntityRegistry<PlayerNetId>,
) {
    // Broadcasting updates about new connected players.
    for (connected_player_net_id, connected_player_connection_handle) in
        network_params.new_player_connections.drain(..)
    {
        let connection_state = network_params
            .connection_states
            .get_mut(&connected_player_connection_handle)
            .expect("Expected a ConnectionState for a new player");
        let connected_player = players
            .get(&connected_player_net_id)
            .expect("Expected a new Player to exist");

        assert!(matches!(
            connection_state.status(),
            ConnectionStatus::Handshaking
        ));

        // TODO: prepare the update in another system.
        let mut players_state: Vec<PlayerState> = players
            .iter()
            .filter_map(|(&iter_player_net_id, _player)| {
                players_registry
                    .get_entity(iter_player_net_id)
                    .and_then(|entity| {
                        if connected_player_net_id == iter_player_net_id {
                            Some(PlayerState {
                                net_id: connected_player_net_id,
                                position: Vec2::ZERO,
                                inputs: Vec::new(),
                            })
                        } else {
                            create_player_state(
                                iter_player_net_id,
                                &time,
                                connection_state,
                                entity,
                                &player_entities,
                            )
                        }
                    })
            })
            .collect();
        players_state.push(PlayerState {
            net_id: connected_player_net_id,
            position: Vec2::ZERO,
            inputs: Vec::new(),
        });

        let message = ReliableServerMessage::StartGame(StartGame {
            net_id: connected_player_net_id,
            nickname: connected_player.nickname.clone(),
            objects: level_state
                .objects
                .iter()
                .map(|level_object| SpawnLevelObject {
                    object: level_object.clone(),
                    frame_number: time.frame_number,
                })
                .collect(),
            players: players
                .iter()
                .map(|(&net_id, player)| ConnectedPlayer {
                    net_id,
                    nickname: player.nickname.clone(),
                })
                .collect(),
            game_state: DeltaUpdate {
                frame_number: time.frame_number,
                acknowledgments: connection_state.incoming_acknowledgments(),
                players: players_state,
                confirmed_actions: Vec::new(),
            },
        });

        let result = network_params.net.send_message(
            connected_player_connection_handle,
            Message {
                session_id: connection_state.session_id,
                message,
            },
        );
        if let Err(err) = result {
            log::error!("Failed to send a message: {:?}", err);
        } else {
            connection_state.set_status(ConnectionStatus::Connected);
        }
    }
}

/// Returns `None` if the entity is not spawned for the current frame.
fn create_player_state(
    net_id: PlayerNetId,
    time: &GameTime,
    connection_state: &ConnectionState,
    entity: Entity,
    player_entities: &Query<(Entity, &Position, &PlayerDirection, &Spawned)>,
) -> Option<PlayerState> {
    let (_, position, player_direction, spawned) = player_entities.get(entity).unwrap();
    if !spawned.is_spawned(time.frame_number) {
        return None;
    }

    let updates_start_frame = if connection_state.packet_loss() > 0.0 {
        // TODO: avoid doing the same searches when gathering updates for every player?
        connection_state
            .first_unacknowledged_outgoing_packet()
            .unwrap_or(time.frame_number)
    } else {
        time.frame_number
    };

    // TODO: deduplicate updates (the same code is written for client).
    let mut inputs: Vec<PlayerInput> = Vec::new();
    for (frame_number, &direction) in player_direction
        .buffer
        // TODO: avoid iterating from the beginning?
        .iter_with_interpolation()
        .skip_while(|(frame_number, _)| *frame_number < updates_start_frame)
    {
        if Some(direction) != inputs.last().map(|i| i.direction) {
            inputs.push(PlayerInput {
                frame_number,
                direction,
            });
        }
    }

    let start_position_frame = inputs.first().map_or_else(
        || std::cmp::max(updates_start_frame, position.buffer.start_frame()),
        |input| input.frame_number,
    );

    Some(PlayerState {
        net_id,
        position: *position
            .buffer
            .get(start_position_frame)
            .unwrap_or_else(|| {
                panic!(
                    "Player ({}) position for frame {} doesn't exist (current frame: {})",
                    net_id.0,
                    start_position_frame.value(),
                    time.frame_number.value()
                )
            }),
        inputs,
    })
}
