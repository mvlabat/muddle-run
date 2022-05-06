use crate::{
    Agones, LastPlayerDisconnectedAt, MuddleServerConfig, PersistenceMessage, PersistenceRequest,
    TOKIO,
};
use bevy::{
    ecs::system::SystemParam,
    log,
    prelude::*,
    utils::{Entry, HashMap, HashSet, Instant},
};
use bevy_networking_turbulence::{ConnectionHandle, NetworkEvent, NetworkResource};
use mr_messages_lib::{GetLevelResponse, PLAYER_CAPACITY};
use mr_shared_lib::{
    game::{
        commands::{self, DeferredPlayerQueues, DeferredQueue},
        components::{PlayerDirection, Position, Spawned},
        level::{LevelObject, LevelState},
    },
    messages::{
        DeferredMessagesQueue, DeltaUpdate, DisconnectReason, DisconnectedPlayer, EntityNetId,
        Message, PlayerInputs, PlayerNetId, PlayerState, ReliableClientMessage,
        ReliableServerMessage, RespawnPlayer, RunnerInput, SpawnLevelObject,
        SpawnLevelObjectRequest, StartGame, SwitchRole, UnreliableClientMessage,
        UnreliableServerMessage,
    },
    net::{ConnectionState, ConnectionStatus, MessageId, SessionId, CONNECTION_TIMEOUT_MILLIS},
    player::{random_name, Player, PlayerEvent, PlayerRole},
    registry::{EntityRegistry, Registry},
    server::level_spawn_location_service::LevelSpawnLocationService,
    GameTime, SimulationTime, COMPONENT_FRAMEBUFFER_LIMIT,
};
use rymder::{futures_util::stream::StreamExt, GameServer};
use std::{
    marker::PhantomData,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    time::Duration,
};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};

pub fn watch_agones_updates(
    mut agones_sdk: rymder::Sdk,
) -> tokio::sync::oneshot::Receiver<GameServer> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    TOKIO.spawn(async move {
        log::info!("Marking the GameServer as Ready...");
        if let Err(err) = agones_sdk.mark_ready().await {
            log::error!(
                "Failed to mark the Game Server as Ready, exiting: {:?}",
                err
            );
            std::process::exit(1);
        }
        let mut stream = match agones_sdk.watch_gameserver().await {
            Ok(stream) => stream,
            Err(err) => {
                log::error!("Failed to start watching GameServer updates: {:?}", err);
                return;
            }
        };
        let mut tx = Some(tx);
        while let Some(Ok(game_server)) = stream.next().await {
            log::debug!("GameServer update: {:#?}", game_server);
            if let Some(status) = game_server.status.as_ref() {
                if status.state == rymder::gameserver::State::Allocated {
                    if let Some(tx) = tx.take() {
                        if let Err(err) = tx.send(game_server) {
                            log::error!("Failed to send Agones allocation message: {:?}", err);
                        }
                    }
                }
            }
        }
    });
    rx
}

pub fn startup(
    config: Res<MuddleServerConfig>,
    mut net: ResMut<NetworkResource>,
    agones: Option<Res<Agones>>,
) {
    log::info!("Starting the server");
    let agones_status = agones.as_ref().and_then(|agones| {
        let mut sdk = agones.sdk.clone();
        TOKIO.spawn(async move {
            log::info!(
                "Setting GameServer player capacity to {}...",
                PLAYER_CAPACITY
            );
            if let Err(err) = sdk.set_player_capacity(PLAYER_CAPACITY as u64).await {
                log::error!(
                    "Failed to set Game Server player capacity, exiting: {:?}",
                    err
                );
                std::process::exit(1);
            }
        });

        agones.game_server.status.as_ref()
    });
    let (listen, public) = listen_addr(&config, agones_status)
        .zip(public_id_addr(&config, agones_status))
        .expect("Expected MUDDLE_LISTEN_PORT and MUDDLE_PUBLIC_IP_ADDR env variables");
    net.listen(
        listen,
        Some(listen),
        Some(SocketAddr::new(public, listen.port())),
    );
}

pub type PlayerConnections = Registry<PlayerNetId, u32>;

#[derive(SystemParam)]
pub struct UpdateParams<'w, 's> {
    deferred_player_updates: ResMut<'w, DeferredPlayerQueues<RunnerInput>>,
    switch_role_requests: ResMut<'w, DeferredPlayerQueues<PlayerRole>>,
    spawn_level_object_requests: ResMut<'w, DeferredPlayerQueues<SpawnLevelObjectRequest>>,
    update_level_object_requests: ResMut<'w, DeferredPlayerQueues<LevelObject>>,
    despawn_level_object_requests: ResMut<'w, DeferredPlayerQueues<EntityNetId>>,
    spawn_player_commands: ResMut<'w, DeferredQueue<commands::SpawnPlayer>>,
    despawn_player_commands: ResMut<'w, DeferredQueue<commands::DespawnPlayer>>,
    #[system_param(ignore)]
    marker: PhantomData<&'s ()>,
}

#[derive(SystemParam)]
pub struct NetworkParams<'w, 's> {
    net: ResMut<'w, NetworkResource>,
    connection_states: ResMut<'w, HashMap<u32, ConnectionState>>,
    player_connections: ResMut<'w, PlayerConnections>,
    new_player_connections: ResMut<'w, Vec<(PlayerNetId, u32)>>,
    last_player_disconnected_at: ResMut<'w, LastPlayerDisconnectedAt>,
    players_tracking_channel: Option<ResMut<'w, tokio::sync::mpsc::UnboundedSender<PlayerEvent>>>,
    pending_requests: Local<'s, HashMap<MessageId, ConnectionHandle>>,
    persistence_req_tx: Option<Res<'w, UnboundedSender<PersistenceRequest>>>,
    persistence_msg_rx: Option<ResMut<'w, UnboundedReceiver<PersistenceMessage>>>,
}

pub fn process_network_events(
    mut despawned_players_for_handles: Local<HashSet<u32>>,
    time: Res<GameTime>,
    mut players: ResMut<HashMap<PlayerNetId, Player>>,
    mut network_events: EventReader<NetworkEvent>,
    mut network_params: NetworkParams,
    mut update_params: UpdateParams,
    level_spawn_location_service: LevelSpawnLocationService,
) {
    #[cfg(feature = "profiler")]
    puffin::profile_function!();
    log::trace!("Processing network updates (frame: {})", time.frame_number);

    // Processing connection events.
    for event in network_events.iter() {
        match event {
            NetworkEvent::Connected(handle) => {
                log::info!("New connection: {}", handle);
                let connection_state = network_params.connection_states.entry(*handle).or_default();

                if matches!(
                    connection_state.status(),
                    ConnectionStatus::Connected | ConnectionStatus::Disconnecting(_)
                ) {
                    log::warn!("Received a Connected event from a connection that is already connected (or being disconnected). That probably means that the clean-up wasn't properly finished");
                }
                match connection_state.status() {
                    ConnectionStatus::Disconnecting(_) | ConnectionStatus::Disconnected => {
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
                    .get_mut(handle)
                    .expect("Expected a connection when receiving a Disconnect event");
                if matches!(
                    connection_state.status(),
                    ConnectionStatus::Disconnecting(_) | ConnectionStatus::Disconnected
                ) {
                    log::info!("Received a Disconnected event for a player that's already disconnected, skipped");
                    continue;
                }
                connection_state
                    .set_status(ConnectionStatus::Disconnecting(DisconnectReason::Closed));
            }
            NetworkEvent::Error(handle, err) => {
                log::error!("Network error ({}): {:?}", handle, err);
            }
            _ => {}
        }
    }

    let mut initialize_messages_to_send = Vec::new();
    let mut handshake_messages_to_send = Vec::new();
    let mut disconnect_messages_to_send = Vec::new();

    if let Some(msg_rx) = network_params.persistence_msg_rx.as_mut() {
        while let Ok(persistence_message) = msg_rx.try_recv() {
            match persistence_message {
                PersistenceMessage::UserInfoResponse { id, user } => {
                    let handle = network_params
                        .pending_requests
                        .get(&id)
                        .expect("Expected a pending persistence request on a response message");
                    let connection_state = network_params
                        .connection_states
                        .get_mut(handle)
                        .expect("Expected a connection state for an existing connection");

                    let Some(user) = user else {
                        disconnect_messages_to_send.push((
                            *handle,
                            Message {
                                session_id: SessionId::new(0),
                                message: ReliableServerMessage::Disconnect(DisconnectReason::InvalidJwt),
                            },
                        ));
                        continue;
                    };

                    let uuid = uuid::Uuid::new_v4().to_string();
                    let player = Player {
                        uuid,
                        ..Player::new_with_nickname(
                            PlayerRole::Runner,
                            user.display_name.unwrap_or_else(random_name),
                        )
                    };
                    log::debug!("Registering a player: {}", player.nickname);
                    let deps = RegisterPlayerDeps {
                        players: &mut players,
                        player_connections: &mut network_params.player_connections,
                        new_player_connections: &mut network_params.new_player_connections,
                        players_tracking_channel: network_params
                            .players_tracking_channel
                            .as_deref_mut(),
                    };
                    register_player(
                        &time,
                        deps,
                        player,
                        &mut update_params,
                        &level_spawn_location_service,
                        *handle,
                    );
                    connection_state.set_status(ConnectionStatus::Handshaking);
                }
                PersistenceMessage::SaveLevelResponse(_) => {
                    log::warn!("TODO: cover `PersistenceMessage::SaveLevelResponse`");
                }
            }
        }
    }

    // Reading message channels.
    for (handle, connection) in network_params.net.connections.iter_mut() {
        let channels = connection.channels().unwrap();

        while let Some(client_message) = channels.recv::<Message<UnreliableClientMessage>>() {
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
                    | ConnectionStatus::Disconnecting(_) => {
                        log::warn!("Skipping Connect message for a connected client");
                        continue;
                    }
                    ConnectionStatus::Initialized => unreachable!(),
                }

                connection_state.set_status(ConnectionStatus::Connecting);
                connection_state.handshake_id = *message_id;
                connection_state.last_valid_message_received_at = Instant::now();
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
                    log::trace!(
                        "Incoming update message (frame: {}): {:?}",
                        time.frame_number,
                        update
                    );
                    if let Err(err) = connection_state.acknowledge_incoming(update.frame_number) {
                        log::debug!(
                            "Failed to acknowledge an incoming packet (player: {}, update frame: {}, current frame: {}): {:?}",
                            player_net_id.0,
                                    update.frame_number,
                            time.frame_number,
                            err
                        );
                        continue;
                    }
                    if let (Some(frame_number), ack_bit_set) = update.acknowledgments {
                        if let Err(err) = connection_state
                            .apply_outgoing_acknowledgements(frame_number, ack_bit_set)
                        {
                            log::debug!(
                                "Failed to apply outgoing packet acknowledgments (player: {}, update frame: {}, current frame: {}): {:?}",
                                player_net_id.0,
                                update.frame_number,
                                time.frame_number,
                                err
                            );
                            continue;
                        }
                    }

                    // Builders don't send any useful inputs that we need to track with unreliable
                    // messages atm.
                    if let PlayerInputs::Runner { inputs } = update.inputs {
                        for input in inputs {
                            if input.frame_number.diff_abs(time.frame_number).value()
                                <= COMPONENT_FRAMEBUFFER_LIMIT / 2
                            {
                                update_params
                                    .deferred_player_updates
                                    .push(player_net_id, input);
                            } else {
                                log::warn!(
                                    "Player {} is out of sync (input frame {}, current frame: {}), skipping the update",
                                    player_net_id.0,
                                    input.frame_number,
                                    time.frame_number
                                );
                                continue;
                            }
                        }
                    }
                }
                UnreliableClientMessage::Connect(_) => {}
            }
            connection_state.last_valid_message_received_at = Instant::now();
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
                ReliableClientMessage::Handshake {
                    message_id: handshake_id,
                    id_token,
                } => {
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

                    if let Some(id_token) = id_token {
                        let Some(req_tx) = network_params.persistence_req_tx.as_ref() else {
                            disconnect_messages_to_send.push((
                                *handle,
                                Message {
                                    session_id: SessionId::new(0),
                                    message: ReliableServerMessage::Disconnect(DisconnectReason::InvalidJwt),
                                },
                            ));
                            break;
                        };

                        req_tx
                            .send(PersistenceRequest::GetUser {
                                id: handshake_id,
                                id_token,
                            })
                            .expect("Failed to send a persistence request");
                        network_params
                            .pending_requests
                            .insert(handshake_id, *handle);
                        break;
                    }

                    let nickname = random_name();
                    let uuid = uuid::Uuid::new_v4().to_string();
                    let player = Player {
                        uuid,
                        ..Player::new_with_nickname(PlayerRole::Runner, nickname)
                    };
                    log::debug!("Registering an anonymous player: {}", player.nickname);
                    let deps = RegisterPlayerDeps {
                        players: &mut players,
                        player_connections: &mut network_params.player_connections,
                        new_player_connections: &mut network_params.new_player_connections,
                        players_tracking_channel: network_params
                            .players_tracking_channel
                            .as_deref_mut(),
                    };
                    register_player(
                        &time,
                        deps,
                        player,
                        &mut update_params,
                        &level_spawn_location_service,
                        *handle,
                    );
                    connection_state.set_status(ConnectionStatus::Handshaking);
                }
                ReliableClientMessage::SwitchRole(role) => {
                    log::info!("Client ({}) requests to switch role to {:?}", handle, role);
                    let connection_state = network_params
                        .connection_states
                        .get_mut(handle)
                        .expect("Expected a connection state for an existing connection");
                    if !matches!(connection_state.status(), ConnectionStatus::Connected) {
                        continue;
                    }
                    let player_net_id = network_params
                        .player_connections
                        .get_id(*handle)
                        .expect("Expected a registered player net id for an existing connection");
                    update_params.switch_role_requests.push(player_net_id, role);
                }
                ReliableClientMessage::SpawnLevelObject(spawn_level_object_request) => {
                    log::info!(
                        "Client ({}) requests to spawn a new object: {:?}",
                        handle,
                        spawn_level_object_request
                    );
                    let connection_state = network_params
                        .connection_states
                        .get_mut(handle)
                        .expect("Expected a connection state for an existing connection");
                    if !matches!(connection_state.status(), ConnectionStatus::Connected) {
                        continue;
                    }
                    let player_net_id = network_params
                        .player_connections
                        .get_id(*handle)
                        .expect("Expected a registered player net id for an existing connection");
                    update_params
                        .spawn_level_object_requests
                        .push(player_net_id, spawn_level_object_request);
                }
                ReliableClientMessage::UpdateLevelObject(update_level_object) => {
                    log::trace!(
                        "Client ({}) requests to update an object: {:?}",
                        handle,
                        update_level_object
                    );
                    let connection_state = network_params
                        .connection_states
                        .get_mut(handle)
                        .expect("Expected a connection state for an existing connection");
                    if !matches!(connection_state.status(), ConnectionStatus::Connected) {
                        continue;
                    }
                    let player_net_id = network_params
                        .player_connections
                        .get_id(*handle)
                        .expect("Expected a registered player net id for an existing connection");
                    update_params
                        .update_level_object_requests
                        .push(player_net_id, update_level_object);
                }
                ReliableClientMessage::DespawnLevelObject(despawned_level_object_net_id) => {
                    log::trace!(
                        "Client ({}) requests to despawn an object: {:?}",
                        handle,
                        despawned_level_object_net_id
                    );
                    let connection_state = network_params
                        .connection_states
                        .get_mut(handle)
                        .expect("Expected a connection state for an existing connection");
                    if !matches!(connection_state.status(), ConnectionStatus::Connected) {
                        continue;
                    }
                    let player_net_id = network_params
                        .player_connections
                        .get_id(*handle)
                        .expect("Expected a registered player net id for an existing connection");
                    update_params
                        .despawn_level_object_requests
                        .push(player_net_id, despawned_level_object_net_id);
                }
            }

            if let Some(connection_state) = network_params.connection_states.get_mut(handle) {
                connection_state.last_valid_message_received_at = Instant::now();
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
    for (handle, message) in disconnect_messages_to_send {
        if let Err(err) = network_params.net.send_message(handle, message) {
            log::error!("Failed to send Disconnect message: {:?}", err);
        }
    }

    disconnect_players(
        &mut despawned_players_for_handles,
        &time,
        &mut network_params,
        &mut update_params,
        &mut players,
    );
}

struct RegisterPlayerDeps<'a> {
    players: &'a mut HashMap<PlayerNetId, Player>,
    player_connections: &'a mut PlayerConnections,
    new_player_connections: &'a mut Vec<(PlayerNetId, u32)>,
    players_tracking_channel: Option<&'a mut tokio::sync::mpsc::UnboundedSender<PlayerEvent>>,
}

fn register_player(
    time: &GameTime,
    mut register_player_deps: RegisterPlayerDeps,
    player: Player,
    update_params: &mut UpdateParams,
    level_spawn_location_service: &LevelSpawnLocationService,
    handle: ConnectionHandle,
) {
    let player_net_id = register_player_deps.player_connections.register(handle);

    log::trace!(
        "Add new player ({:?}) connection to broadcast: {}",
        player_net_id,
        handle
    );
    register_player_deps
        .new_player_connections
        .push((player_net_id, handle));

    if let Some(players_tracking_channel) = register_player_deps.players_tracking_channel.as_mut() {
        if let Err(err) = players_tracking_channel.send(PlayerEvent::Connected(player.uuid.clone()))
        {
            log::error!("Failed to send PlayerEvent: {:?}", err);
        }
    }
    register_player_deps.players.insert(player_net_id, player);
    update_params
        .spawn_player_commands
        .push(commands::SpawnPlayer {
            net_id: player_net_id,
            start_position: level_spawn_location_service.spawn_position(time.frame_number),
            is_player_frame_simulated: false,
        });
    // Add an initial update to have something to extrapolate from.
    update_params.deferred_player_updates.push(
        player_net_id,
        RunnerInput {
            frame_number: time.frame_number,
            direction: Vec2::ZERO,
        },
    );
}

fn disconnect_players(
    despawned_players_for_handles: &mut HashSet<u32>,
    time: &GameTime,
    network_params: &mut NetworkParams,
    update_params: &mut UpdateParams,
    players: &mut HashMap<PlayerNetId, Player>,
) {
    // Disconnecting players that have been failing to deliver updates for some time.
    for (handle, connection_state) in network_params.connection_states.iter_mut() {
        // We might have marked a client as `Disconnecting` when processing connection events.
        if let ConnectionStatus::Disconnected | ConnectionStatus::Disconnecting(_) =
            connection_state.status()
        {
            continue;
        }

        let (last_incoming_frame, _) = connection_state.incoming_acknowledgments();
        if let Some(last_incoming_frame) = last_incoming_frame {
            // If the difference between last incoming frame and the current one is more
            // than 10 secs, we disconnect the client. Neither lagging behind, nor being far ahead
            // is right.
            if time.frame_number.diff_abs(last_incoming_frame).value() > COMPONENT_FRAMEBUFFER_LIMIT
            {
                log::warn!("Disconnecting {}: lagging or falling behind", handle);
                connection_state
                    .set_status(ConnectionStatus::Disconnecting(DisconnectReason::Timeout));
            }
        } else if Instant::now().duration_since(connection_state.status_updated_at())
            > Duration::from_secs(CONNECTION_TIMEOUT_MILLIS)
        {
            // Disconnect players that haven't sent any updates at all (they are likely
            // in the `Connecting` or `Handshaking` status) if they are staying in this state
            // for 10 seconds.
            log::warn!("Disconnecting {}: handshake timeout", handle);
            connection_state.set_status(ConnectionStatus::Disconnecting(DisconnectReason::Timeout));
        }

        // Disconnecting players that haven't sent any message for `CONNECTION_TIMEOUT_MILLIS`.
        if Instant::now().duration_since(connection_state.last_valid_message_received_at)
            > Duration::from_secs(CONNECTION_TIMEOUT_MILLIS)
        {
            log::warn!("Disconnecting {}: idle", handle);
            connection_state.set_status(ConnectionStatus::Disconnecting(DisconnectReason::Timeout));
        }
    }

    // FixedTimestep may run this several times in a row. We want to make sure that we despawn
    // a player only once.
    despawned_players_for_handles
        .drain_filter(|handle| !network_params.connection_states.contains_key(handle));

    for (connection_handle, connection_state) in network_params.connection_states.iter() {
        // We expect that this status lives only during this frame so despawning will be queued
        // only once. The status MUST be changed to `Disconnected` when broadcasting the updates.
        if let ConnectionStatus::Disconnecting(_) = connection_state.status() {
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
                update_params
                    .despawn_player_commands
                    .push(commands::DespawnPlayer {
                        net_id: player_net_id,
                        frame_number: time.frame_number,
                    });
                players
                    .get_mut(&player_net_id)
                    .expect("Expected a registered player with an existing player_net_id")
                    .is_connected = false;
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
        network_params.net.disconnect(handle);
        network_params.player_connections.remove_by_value(handle);
    }
}

#[derive(SystemParam)]
pub struct DeferredMessageQueues<'w, 's> {
    switch_role_messages: ResMut<'w, DeferredMessagesQueue<SwitchRole>>,
    respawn_player_messages: ResMut<'w, DeferredMessagesQueue<RespawnPlayer>>,
    spawn_level_object_messages: ResMut<'w, DeferredMessagesQueue<SpawnLevelObject>>,
    update_level_object_messages: ResMut<'w, DeferredMessagesQueue<commands::UpdateLevelObject>>,
    despawn_level_object_messages: ResMut<'w, DeferredMessagesQueue<commands::DespawnLevelObject>>,
    #[system_param(ignore)]
    marker: PhantomData<&'s ()>,
}

#[derive(SystemParam)]
pub struct LevelParams<'w, 's> {
    level_info: Option<Res<'w, GetLevelResponse>>,
    level_state: Res<'w, LevelState>,
    #[system_param(ignore)]
    marker: PhantomData<&'s ()>,
}

pub fn send_network_updates(
    mut network_params: NetworkParams,
    time: Res<SimulationTime>,
    level_params: LevelParams,
    players: Res<HashMap<PlayerNetId, Player>>,
    player_entities: Query<(Entity, &Position, &PlayerDirection, &Spawned)>,
    players_registry: Res<EntityRegistry<PlayerNetId>>,
    mut deferred_message_queues: DeferredMessageQueues,
) {
    #[cfg(feature = "profiler")]
    puffin::profile_function!();
    // We run this system after we've concluded the simulation. As we don't have updates for the
    // next frame yet, we decrement the frame number.
    let time = time.prev_frame();
    log::trace!("Sending network updates (frame: {})", time.server_frame);

    broadcast_start_game_messages(
        &mut network_params,
        &time,
        level_params.level_info.as_deref(),
        &level_params.level_state,
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

        send_new_player_messages(
            &mut network_params.net,
            &network_params.new_player_connections,
            &players,
            connection_handle,
            connection_state,
        )
    }

    for switch_role_message in deferred_message_queues
        .switch_role_messages
        .drain()
        .into_iter()
    {
        broadcast_reliable_game_message(
            &mut network_params.net,
            &network_params.connection_states,
            ReliableServerMessage::SwitchRole(switch_role_message),
        );
    }
    for respawn_player_message in deferred_message_queues
        .respawn_player_messages
        .drain()
        .into_iter()
    {
        broadcast_reliable_game_message(
            &mut network_params.net,
            &network_params.connection_states,
            ReliableServerMessage::RespawnPlayer(respawn_player_message),
        );
    }
    for spawn_level_object_message in deferred_message_queues
        .spawn_level_object_messages
        .drain()
        .into_iter()
    {
        broadcast_reliable_game_message(
            &mut network_params.net,
            &network_params.connection_states,
            ReliableServerMessage::SpawnLevelObject(spawn_level_object_message),
        );
    }
    for update_level_object_message in deferred_message_queues
        .update_level_object_messages
        .drain()
        .into_iter()
    {
        broadcast_reliable_game_message(
            &mut network_params.net,
            &network_params.connection_states,
            ReliableServerMessage::UpdateLevelObject(update_level_object_message),
        );
    }
    for despawn_level_object_message in deferred_message_queues
        .despawn_level_object_messages
        .drain()
        .into_iter()
    {
        broadcast_reliable_game_message(
            &mut network_params.net,
            &network_params.connection_states,
            ReliableServerMessage::DespawnLevelObject(despawn_level_object_message),
        );
    }

    network_params.new_player_connections.clear();
}

fn broadcast_disconnected_players(network_params: &mut NetworkParams) {
    let mut disconnected_players = Vec::new();
    for (&connection_handle, connection_state) in network_params.connection_states.iter_mut() {
        let ConnectionStatus::Disconnecting(reason) = connection_state.status() else {
            continue;
        };

        if let Some(connection_player_net_id) =
            network_params.player_connections.get_id(connection_handle)
        {
            disconnected_players.push(connection_player_net_id);
        }

        if let Err(err) = network_params.net.send_message(
            connection_handle,
            Message {
                session_id: connection_state.session_id,
                message: ReliableServerMessage::Disconnect(reason),
            },
        ) {
            log::error!("Failed to send a message: {:?}", err);
        }
        log::debug!("Marking connection {} as Disconnected", connection_handle);
        *network_params.last_player_disconnected_at = LastPlayerDisconnectedAt(Instant::now());
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
    time: &SimulationTime,
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
        frame_number: time.server_frame,
        acknowledgments: connection_state.incoming_acknowledgments(),
        players: players
            .iter()
            .filter_map(|(&player_net_id, _player)| {
                players_registry
                    .get_entity(player_net_id)
                    .and_then(|entity| {
                        create_player_state(
                            player_net_id,
                            time,
                            connection_state,
                            entity,
                            player_entities,
                        )
                    })
            })
            .collect(),
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

    connection_state.add_outgoing_packet(time.server_frame, Instant::now());
}

fn send_new_player_messages(
    net: &mut NetworkResource,
    new_player_connections: &[(PlayerNetId, u32)],
    players: &HashMap<PlayerNetId, Player>,
    connection_handle: u32,
    connection_state: &ConnectionState,
) {
    if !new_player_connections.is_empty() {
        log::trace!(
            "Sending new players to {}: {:?}",
            connection_handle,
            players
        );
    }
    // Broadcasting updates about new connected players.
    for (connected_player_net_id, _connection_handle) in new_player_connections.iter() {
        let player = players
            .get(connected_player_net_id)
            .expect("Expected a registered Player");
        let message =
            ReliableServerMessage::ConnectedPlayer((*connected_player_net_id, player.clone()));
        send_reliable_game_message(net, connection_handle, connection_state, message);
    }
}

fn broadcast_start_game_messages(
    network_params: &mut NetworkParams,
    time: &SimulationTime,
    level_info: Option<&GetLevelResponse>,
    level_state: &LevelState,
    players: &HashMap<PlayerNetId, Player>,
    player_entities: &Query<(Entity, &Position, &PlayerDirection, &Spawned)>,
    players_registry: &EntityRegistry<PlayerNetId>,
) {
    // Broadcasting updates about new connected players.
    for (connected_player_net_id, connected_player_connection_handle) in
        &*network_params.new_player_connections
    {
        let connection_state = network_params
            .connection_states
            .get_mut(connected_player_connection_handle)
            .expect("Expected a ConnectionState for a new player");
        let connected_player = players
            .get(connected_player_net_id)
            .expect("Expected a new Player to exist");

        assert!(matches!(
            connection_state.status(),
            ConnectionStatus::Handshaking
        ));

        // TODO: prepare the update in another system.
        let players_state: Vec<PlayerState> = players
            .iter()
            .filter_map(|(&iter_player_net_id, _player)| {
                players_registry
                    .get_entity(iter_player_net_id)
                    .and_then(|entity| {
                        if *connected_player_net_id == iter_player_net_id {
                            // The player isn't spawned yet, we'll tell its position in the next
                            // `DeltaUpdate` message.
                            None
                        } else {
                            create_player_state(
                                iter_player_net_id,
                                time,
                                connection_state,
                                entity,
                                player_entities,
                            )
                        }
                    })
            })
            .collect();

        let message = ReliableServerMessage::StartGame(StartGame {
            handshake_id: connection_state.handshake_id,
            net_id: *connected_player_net_id,
            uuid: connected_player.uuid.clone(),
            nickname: connected_player.nickname.clone(),
            level_id: level_info.map(|level_info| level_info.level.id),
            objects: level_state
                .objects
                .iter()
                .map(|(_, level_object)| commands::UpdateLevelObject {
                    object: level_object.clone(),
                    frame_number: time.server_frame,
                })
                .collect(),
            players: players
                .iter()
                .map(|(net_id, player)| (*net_id, player.clone()))
                .collect(),
            generation: time.server_generation,
            game_state: DeltaUpdate {
                frame_number: time.server_frame,
                acknowledgments: connection_state.incoming_acknowledgments(),
                players: players_state,
            },
        });

        log::info!(
            "Sending the StartGame message to player {}: (handle: {}, session_id: {}, handshake_id: {})",
            connected_player_connection_handle,
            connected_player_net_id.0,
            connection_state.session_id,
            connection_state.handshake_id
        );

        let result = network_params.net.send_message(
            *connected_player_connection_handle,
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
    time: &SimulationTime,
    connection_state: &ConnectionState,
    entity: Entity,
    player_entities: &Query<(Entity, &Position, &PlayerDirection, &Spawned)>,
) -> Option<PlayerState> {
    let (_, position, player_direction, spawned) = player_entities.get(entity).unwrap();
    if !spawned.is_spawned(time.server_frame) {
        return None;
    }

    let updates_start_frame = if connection_state.packet_loss() > 0.0 {
        // TODO: avoid doing the same searches when gathering updates for every player?
        connection_state
            .first_unacknowledged_outgoing_packet()
            .unwrap_or(time.server_frame)
    } else {
        time.server_frame
    };

    // TODO: deduplicate updates (the same code is written for client).
    let mut inputs: Vec<RunnerInput> = Vec::new();
    for (frame_number, &direction) in player_direction
        .buffer
        // TODO: avoid iterating from the beginning?
        .iter_with_interpolation()
        .skip_while(|(frame_number, _)| *frame_number < updates_start_frame)
    {
        if Some(direction) != inputs.last().map(|i| i.direction) {
            inputs.push(RunnerInput {
                frame_number,
                direction,
            });
        }
    }
    if inputs.is_empty() && player_direction.buffer.len() > 1 {
        log::debug!(
            "Missing updates for Player {} (updates start frame: {}, last player direction frame: {:?})",
            net_id.0,
            updates_start_frame,
            player_direction.buffer.end_frame(),
        );
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
                    "Player ({}) position for frame {} doesn't exist (current frame: {}, entity: {:?}): {:?}",
                    net_id.0,
                    start_position_frame.value(),
                    time.server_frame.value(),
                    entity,
                    position.buffer,
                )
            }),
        inputs,
    })
}

fn broadcast_reliable_game_message(
    net: &mut NetworkResource,
    connection_states: &HashMap<u32, ConnectionState>,
    message: ReliableServerMessage,
) {
    for (&connection_handle, connection_state) in connection_states.iter() {
        if !matches!(connection_state.status(), ConnectionStatus::Connected) {
            continue;
        }

        send_reliable_game_message(net, connection_handle, connection_state, message.clone());
    }
}

fn send_reliable_game_message(
    net: &mut NetworkResource,
    connection_handle: u32,
    connection_state: &ConnectionState,
    message: ReliableServerMessage,
) {
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

fn listen_addr(
    server_config: &MuddleServerConfig,
    gameserver_status: Option<&rymder::gameserver::Status>,
) -> Option<SocketAddr> {
    let server_port = gameserver_status
        .and_then(|status| {
            status
                .ports
                .iter()
                .find(|p| p.name == "MUDDLE_LISTEN_PORT-udp")
                .map(|p| {
                    log::info!(
                        "Reading MUDDLE_LISTEN_PORT from the Agones config: {}",
                        p.port
                    );
                    p.port
                })
        })
        .or(server_config.listen_port)?;

    let zero_ip_addr = IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0));

    let ip_addr = gameserver_status
        .map(|_| {
            // If we are in the Agones environment, we want our listen address be 0.0.0.0.
            log::info!(
                "Being in the Agones environment, implying the following MUDDLE_LISTEN_IP_ADDR value: {}",
                zero_ip_addr
            );
            zero_ip_addr
        })
        .or(server_config.listen_ip_addr);

    if let Some(ip_addr) = ip_addr {
        return Some(SocketAddr::new(ip_addr, server_port));
    }

    Some(SocketAddr::new(zero_ip_addr, server_port))
}

fn public_id_addr(
    server_config: &MuddleServerConfig,
    gameserver_status: Option<&rymder::gameserver::Status>,
) -> Option<IpAddr> {
    let ip_addr = gameserver_status
        .map(|status| {
            log::info!(
                "Reading MUDDLE_PUBLIC_IP_ADDR from the Agones config: {}",
                status.address
            );
            status.address
        })
        .or(server_config.public_ip_addr);

    if ip_addr.is_some() {
        return ip_addr;
    }

    if let Some(addr) = bevy_networking_turbulence::find_my_ip_address() {
        log::info!(
            "Using an automatically detected public IP address: {}",
            addr.to_string()
        );
        return Some(addr);
    }

    None
}
