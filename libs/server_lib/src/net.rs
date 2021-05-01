use crate::player_updates::DeferredUpdates;
use bevy::{ecs::system::SystemParam, log, prelude::*, utils::HashSet};
use bevy_networking_turbulence::{NetworkEvent, NetworkResource};
use chrono::Utc;
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
    net::{ConnectionState, ConnectionStatus, SessionId, CONNECTION_TIMEOUT_MILLIS},
    player::{random_name, Player},
    registry::{EntityRegistry, Registry},
    GameTime, COMPONENT_FRAMEBUFFER_LIMIT,
};
use std::{
    collections::{hash_map::Entry, HashMap},
    net::{IpAddr, Ipv4Addr, SocketAddr},
};

pub fn startup(mut net: ResMut<NetworkResource>) {
    log::info!("Starting the server");
    let (listen, public) = listen_addr()
        .zip(public_id_addr())
        .expect("Expected MUDDLE_LISTEN_PORT and MUDDLE_PUBLIC_IP_ADDR env variables");
    net.listen(listen, public);
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
    mut despawned_players_for_handles: Local<HashSet<u32>>,
    time: Res<GameTime>,
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
                let connection_state = network_params.connection_states.entry(*handle).or_default();

                if matches!(
                    connection_state.status(),
                    ConnectionStatus::Connected | ConnectionStatus::Disconnecting
                ) {
                    log::warn!("Received a Connected event from a connection that is already connected (or being disconnected). That probably means that the clean-up wasn't properly finished");
                }
                match connection_state.status() {
                    ConnectionStatus::Disconnecting | ConnectionStatus::Disconnected => {
                        // It's unlikely that this branch will ever be called with the current state of bevy_networking_turbulence.
                        // TODO: track https://github.com/smokku/bevy_networking_turbulence/issues/6.
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
            NetworkEvent::Error(handle, err) => {
                log::error!("Network error ({}): {:?}", handle, err);
            }
            _ => {}
        }
    }

    let mut initialize_messages_to_send = Vec::new();
    let mut handshake_messages_to_send = Vec::new();

    // Reading message channels.
    for (handle, connection) in network_params.net.connections.iter_mut() {
        let channels = connection.channels().unwrap();

        'channel: while let Some(client_message) =
            channels.recv::<Message<UnreliableClientMessage>>()
        {
            log::trace!(
                "UnreliableClientMessage received on [{}]: {:?}",
                handle,
                client_message
            );

            if let UnreliableClientMessage::Connect(message_id) = &client_message.message {
                log::info!("New client ({}) Connect message: {}", handle, message_id);
                let connection_state_entry = match network_params.connection_states.entry(*handle) {
                    Entry::Occupied(connection_state_entry) => {
                        let connection_state = connection_state_entry.get();
                        let current_handshake_id = if matches!(
                            connection_state.status(),
                            ConnectionStatus::Uninitialized | ConnectionStatus::Disconnected
                        ) {
                            None
                        } else {
                            Some(connection_state.handshake_id)
                        };
                        if current_handshake_id.map_or(false, |id| id >= *message_id) {
                            log::warn!(
                                "Ignoring Connect message with outdated handshake id: {}, current: {:?}",
                                message_id,
                                current_handshake_id
                            );
                            continue;
                        }
                        Entry::Occupied(connection_state_entry)
                    }
                    Entry::Vacant(entry) => Entry::Vacant(entry),
                };
                let connection_state = connection_state_entry.or_default();

                match connection_state.status() {
                    ConnectionStatus::Uninitialized | ConnectionStatus::Connecting => {}
                    ConnectionStatus::Disconnected => {
                        connection_state.session_id += SessionId::new(1);
                    }
                    ConnectionStatus::Connected
                    | ConnectionStatus::Handshaking
                    | ConnectionStatus::Disconnecting => {
                        log::warn!("Skipping Connect message for a connected client");
                        continue;
                    }
                }

                connection_state.set_status(ConnectionStatus::Connecting);
                connection_state.handshake_id = *message_id;
                connection_state.last_message_received_at = Utc::now();
                handshake_messages_to_send.push((
                    *handle,
                    Message {
                        session_id: SessionId::new(0),
                        message: UnreliableServerMessage::Handshake(*message_id),
                    },
                ));

                continue;
            };

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
            connection_state.last_message_received_at = Utc::now();

            if !matches!(connection_state.status(), ConnectionStatus::Connected) {
                log::warn!(
                    "Ignoring a message for a player ({}): expected connection status is {:?}, but it's {:?}",
                    player_net_id.0,
                    ConnectionStatus::Connected,
                    connection_state.status()
                );
                continue;
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
                        connection_state.set_status(ConnectionStatus::Disconnecting);
                        log::error!(
                            "Failed to acknowledge an incoming packet (player: {}, update frame: {}, current frame: {}), disconnecting: {:?}",
                            player_net_id.0,
                                    update.frame_number,
                            time.frame_number,
                            err
                        );
                        break 'channel;
                    }
                    if let (Some(frame_number), ack_bit_set) = update.acknowledgments {
                        if let Err(err) = connection_state
                            .apply_outgoing_acknowledgements(frame_number, ack_bit_set)
                        {
                            connection_state.set_status(ConnectionStatus::Disconnecting);
                            log::error!(
                                "Failed to apply outgoing packet acknowledgments (player: {}, update frame: {}, current frame: {}), disconnecting: {:?}",
                                player_net_id.0,
                                update.frame_number,
                                time.frame_number,
                                err
                            );
                            break 'channel;
                        }
                    }
                    for input in update.inputs {
                        if input.frame_number.diff_abs(time.frame_number).value()
                            > COMPONENT_FRAMEBUFFER_LIMIT / 2
                        {
                            log::warn!(
                                "Player {} is out of sync (input frame {}, current frame: {}), disconnecting",
                                player_net_id.0,
                                input.frame_number,
                                time.frame_number
                            );
                            connection_state.set_status(ConnectionStatus::Disconnecting);
                            break 'channel;
                        }
                        update_params
                            .deferred_player_updates
                            .push(player_net_id, input);
                    }
                }
                UnreliableClientMessage::Connect(_) => {}
            }
        }

        while let Some(client_message) = channels.recv::<Message<ReliableClientMessage>>() {
            log::trace!(
                "ReliableClientMessage received on [{}]: {:?}",
                handle,
                client_message
            );

            match client_message.message {
                ReliableClientMessage::Initialize => {
                    log::info!("Client ({}) Initialize message", handle);
                    initialize_messages_to_send.push((
                        *handle,
                        Message {
                            session_id: SessionId::new(0),
                            message: ReliableServerMessage::Initialize,
                        },
                    ));
                }
                // NOTE: before adding new messages, make sure to ignore them if connection status
                // is not `Connected`.
                ReliableClientMessage::Handshake(handshake_id) => {
                    log::info!("Client ({}) handshake: {}", handle, handshake_id);
                    let connection_state = network_params
                        .connection_states
                        .get_mut(handle)
                        .expect("Expected a connection state for an existing connection");

                    if connection_state.handshake_id != handshake_id
                        || !matches!(connection_state.status(), ConnectionStatus::Connecting)
                    {
                        log::warn!(
                            "Ignoring a client's ({}) Handshake message. Connection status: {:?}, expected handshake id: {}, received handshake id: {}",
                            handle,
                            connection_state.status(),
                            connection_state.handshake_id,
                            handshake_id
                        );
                        break;
                    }

                    let player_net_id = network_params.player_connections.register(*handle);
                    connection_state.set_status(ConnectionStatus::Handshaking);
                    connection_state.last_message_received_at = Utc::now();

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

    for (handle, message) in initialize_messages_to_send {
        if let Err(err) = network_params.net.send_message(handle, message) {
            log::error!("Failed to send Initialize message: {:?}", err);
        }
    }
    for (handle, message) in handshake_messages_to_send {
        if let Err(err) = network_params.net.send_message(handle, message) {
            log::error!("Failed to send Handshake message: {:?}", err);
        }
    }

    disconnect_players(
        &mut despawned_players_for_handles,
        &time,
        &mut network_params,
        &mut update_params,
    );
}

fn disconnect_players(
    despawned_players_for_handles: &mut HashSet<u32>,
    time: &GameTime,
    network_params: &mut NetworkParams,
    update_params: &mut UpdateParams,
) {
    // Disconnecting players that have been failing to deliver updates for some time.
    for (handle, connection_state) in network_params.connection_states.iter_mut() {
        // We might have marked a client as `Disconnecting` when processing connection events.
        if let ConnectionStatus::Disconnected | ConnectionStatus::Disconnecting =
            connection_state.status()
        {
            continue;
        }

        let (last_incoming_frame, _) = connection_state.incoming_acknowledgments();
        if let Some(last_incoming_frame) = last_incoming_frame {
            // If the difference between last incoming frame and the current one is more
            // than 5 secs, we disconnect the client. Both lagging behind and being far ahead
            // isn't right.
            if time.frame_number.diff_abs(last_incoming_frame).value()
                > COMPONENT_FRAMEBUFFER_LIMIT / 2
            {
                log::warn!("Disconnecting {}: lagging or falling behind", handle);
                connection_state.set_status(ConnectionStatus::Disconnecting);
            }
        } else if Utc::now()
            .signed_duration_since(connection_state.status_updated_at())
            .to_std()
            .unwrap()
            > std::time::Duration::from_secs(5)
        {
            // Disconnect players that haven't sent any updates at all (they are likely
            // in the `Connecting` or `Handshaking` status) if they are staying in this state
            // for 5 seconds.
            log::warn!("Disconnecting {}: handshake timeout", handle);
            connection_state.set_status(ConnectionStatus::Disconnecting);
        }

        // Disconnecting players that haven't sent any message for `CONNECTION_TIMEOUT_MILLIS`.
        if Utc::now()
            .signed_duration_since(connection_state.last_message_received_at)
            .to_std()
            .unwrap()
            > std::time::Duration::from_secs(CONNECTION_TIMEOUT_MILLIS)
        {
            log::warn!("Disconnecting {}: idle", handle);
            connection_state.set_status(ConnectionStatus::Disconnecting);
        }
    }

    // FixedTimestep may run this several times in a row. We want to make sure that we despawn
    // a player only once.
    despawned_players_for_handles
        .drain_filter(|handle| !network_params.connection_states.contains_key(handle));

    for (connection_handle, connection_state) in network_params.connection_states.iter() {
        // We expect that this status lives only during this frame so despawning will be queued
        // only once. The status MUST be changed to `Disconnected` when broadcasting the updates.
        if let ConnectionStatus::Disconnecting = connection_state.status() {
            if !despawned_players_for_handles.insert(*connection_handle) {
                continue;
            }

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
                log::warn!("A disconnected player wasn't in the connections list");
            }
        } else {
            despawned_players_for_handles.remove(connection_handle);
        }
    }

    // Cleaning up connections with `Disconnected` status.
    let disconnected_handles: Vec<u32> = network_params
        .connection_states
        .iter()
        .filter_map(|(handle, connection_state)| {
            matches!(connection_state.status(), ConnectionStatus::Disconnected).then_some(*handle)
        })
        .collect();
    for handle in disconnected_handles {
        log::info!("Removing connection {}", handle);
        network_params.connection_states.remove(&handle);
        network_params.net.connections.remove(&handle);
        network_params.player_connections.remove_by_value(handle);
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
    for (&connection_handle, connection_state) in network_params.connection_states.iter_mut() {
        if !matches!(connection_state.status(), ConnectionStatus::Disconnecting) {
            continue;
        }

        if let Some(connection_player_net_id) =
            network_params.player_connections.get_id(connection_handle)
        {
            disconnected_players.push(connection_player_net_id);
        }

        if let Err(err) = network_params.net.send_message(
            connection_handle,
            Message {
                session_id: connection_state.session_id,
                message: ReliableServerMessage::Disconnect,
            },
        ) {
            log::error!("Failed to send a message: {:?}", err);
        }
        log::debug!("Marking connection {} as Disconnected", connection_handle);
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

    connection_state.add_outgoing_packet(time.frame_number, Utc::now());
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
            handshake_id: connection_state.handshake_id,
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

fn listen_addr() -> Option<SocketAddr> {
    let server_port = std::env::var("MUDDLE_LISTEN_PORT")
        .ok()
        .or_else(|| std::option_env!("MUDDLE_LISTEN_PORT").map(str::to_owned))
        .map(|port| port.parse::<u16>().expect("invalid port"))?;

    let env_ip_addr = std::env::var("MUDDLE_LISTEN_IP_ADDR")
        .ok()
        .or_else(|| std::option_env!("MUDDLE_LISTEN_IP_ADDR").map(str::to_owned));
    if let Some(env_addr) = env_ip_addr {
        return Some(SocketAddr::new(
            env_addr.parse::<IpAddr>().expect("invalid socket address"),
            server_port,
        ));
    }

    Some(SocketAddr::new(
        IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0)),
        server_port,
    ))
}

fn public_id_addr() -> Option<IpAddr> {
    let env_ip_addr = std::env::var("MUDDLE_PUBLIC_IP_ADDR")
        .ok()
        .or_else(|| std::option_env!("MUDDLE_PUBLIC_IP_ADDR").map(str::to_owned));
    if let Some(env_addr) = env_ip_addr {
        return Some(env_addr.parse::<IpAddr>().expect("invalid socket address"));
    }

    if let Some(addr) = bevy_networking_turbulence::find_my_ip_address() {
        return Some(addr);
    }

    None
}
