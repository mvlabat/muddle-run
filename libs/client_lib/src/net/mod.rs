pub use persistence::{PersistenceMessage, PersistenceMessagePayload, PersistenceRequest};

use crate::{
    input::{LevelObjectRequestsQueue, PlayerRequestsQueue},
    net::{
        auth::AuthConfig,
        matchmaker::MatchmakerRequestsHandler,
        persistence::{PersistenceClient, PersistenceRequestsHandler},
    },
    CurrentPlayerNetId, DelayServerTime, EstimatedServerTime, InitialRtt, LevelObjectCorrelations,
    MuddleClientConfig, TargetFramesAhead,
};
use auth::{AuthMessage, AuthRequest};
use bevy::{ecs::system::SystemParam, log, prelude::*, utils::Instant};
use bevy_disturbulence::{NetworkEvent, NetworkResource};
use futures::{select, FutureExt};
use mr_messages_lib::{MatchmakerMessage, MatchmakerRequest, Server};
use mr_shared_lib::{
    framebuffer::{FrameNumber, Framebuffer},
    game::{
        commands::{
            DeferredQueue, DespawnLevelObject, DespawnPlayer, DespawnReason, RestartGame,
            SpawnPlayer, SwitchPlayerRole, UpdateLevelObject,
        },
        components::{PlayerDirection, Spawned},
    },
    messages::{
        DeltaUpdate, DisconnectReason, DisconnectedPlayer, Message, PlayerInputs, PlayerNetId,
        PlayerUpdate, ReliableClientMessage, ReliableServerMessage, RespawnPlayerReason,
        RunnerInput, StartGame, UnreliableClientMessage, UnreliableServerMessage,
    },
    net::{
        AcknowledgeError, ConnectionState, ConnectionStatus, MessageId, SessionId,
        CONNECTION_TIMEOUT_MILLIS,
    },
    player::{Player, PlayerDirectionUpdate, PlayerRole, PlayerUpdates, Players},
    registry::EntityRegistry,
    GameTime, SimulationTime, COMPONENT_FRAMEBUFFER_LIMIT, SIMULATIONS_PER_SECOND,
};
use std::{
    future::Future,
    marker::PhantomData,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    time::Duration,
};
use tokio::sync::mpsc::{
    error::TryRecvError, unbounded_channel, UnboundedReceiver, UnboundedSender,
};

pub mod auth;

#[cfg(target_arch = "wasm32")]
mod listen_local_storage;
mod matchmaker;
mod persistence;
#[cfg(not(target_arch = "wasm32"))]
mod redirect_uri_server;

pub const DEFAULT_SERVER_PORT: u16 = 3455;
const DEFAULT_SERVER_IP_ADDR: IpAddr = IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1));

#[derive(SystemParam)]
pub struct UpdateParams<'w, 's> {
    simulation_time: ResMut<'w, SimulationTime>,
    game_time: ResMut<'w, GameTime>,
    player_entities: Res<'w, EntityRegistry<PlayerNetId>>,
    estimated_server_time: ResMut<'w, EstimatedServerTime>,
    target_frames_ahead: ResMut<'w, TargetFramesAhead>,
    delay_server_time: ResMut<'w, DelayServerTime>,
    initial_rtt: ResMut<'w, InitialRtt>,
    player_updates: ResMut<'w, PlayerUpdates>,
    restart_game_commands: ResMut<'w, DeferredQueue<RestartGame>>,
    level_object_correlations: ResMut<'w, LevelObjectCorrelations>,
    spawn_level_object_commands: ResMut<'w, DeferredQueue<UpdateLevelObject>>,
    despawn_level_object_commands: ResMut<'w, DeferredQueue<DespawnLevelObject>>,
    spawn_player_commands: ResMut<'w, DeferredQueue<SpawnPlayer>>,
    despawn_player_commands: ResMut<'w, DeferredQueue<DespawnPlayer>>,
    switch_role_commands: ResMut<'w, DeferredQueue<SwitchPlayerRole>>,
    spawned_query: Query<'w, 's, &'static Spawned>,
}

#[derive(SystemParam)]
pub struct NetworkParams<'w, 's> {
    net: NonSendMut<'w, NetworkResource>,
    connection_state: ResMut<'w, ConnectionState>,
    #[system_param(ignore)]
    marker: PhantomData<&'s ()>,
}

#[derive(Debug)]
pub enum TcpConnectionStatus {
    Disconnected,
    Connecting,
    Connected,
}

#[derive(Resource)]
pub struct MatchmakerState {
    pub status: TcpConnectionStatus,
    pub id_token: Option<String>,
    pub user_id: Option<i64>,
}

#[derive(Resource)]
pub struct MainMenuUiChannels {
    pub auth_request_tx: UnboundedSender<AuthRequest>,
    pub auth_message_tx: UnboundedSender<AuthMessage>,
    pub auth_message_rx: UnboundedReceiver<AuthMessage>,
    pub connection_request_tx: UnboundedSender<bool>,
    pub status_rx: UnboundedReceiver<TcpConnectionStatus>,
    pub matchmaker_request_tx: UnboundedSender<MatchmakerRequest>,
    pub matchmaker_message_rx: UnboundedReceiver<MatchmakerMessage>,
    pub persistence_request_tx: UnboundedSender<PersistenceRequest>,
    pub persistence_message_rx: UnboundedReceiver<PersistenceMessage>,
}

#[derive(Resource, DerefMut, Deref, Default)]
pub struct ServerToConnect(pub Option<Server>);

pub fn init_matchmaker_connection_system(
    mut commands: Commands,
    client_config: Res<MuddleClientConfig>,
) {
    let matchmaker_url = match &client_config.matchmaker_url {
        Some(url) => url.clone(),
        None => {
            log::info!("Matchmaker address wasn't passed, skipping the initialization");
            return;
        }
    };

    let auth_config = AuthConfig {
        google_client_id: client_config
            .google_client_id
            .clone()
            .expect("Expected MUDDLE_GOOGLE_CLIENT_ID"),
        google_client_secret: client_config.google_client_secret.clone(),
        auth0_client_id: client_config
            .auth0_client_id
            .clone()
            .expect("Expected MUDDLE_AUTH0_CLIENT_ID"),
    };
    if cfg!(not(target_arch = "wasm32")) {
        auth_config
            .google_client_secret
            .as_ref()
            .expect("Expected MUDDLE_GOOGLE_CLIENT_SECRET");
    }

    log::info!("Matchmaker address: {}", matchmaker_url);

    let (auth_request_tx, auth_request_rx) = unbounded_channel();
    let (auth_message_tx, auth_message_rx) = unbounded_channel();
    let (connection_request_tx, connection_request_rx) = unbounded_channel();
    let (status_tx, status_rx) = unbounded_channel();
    let (matchmaker_request_tx, matchmaker_request_rx) = unbounded_channel();
    let (matchmaker_message_tx, matchmaker_message_rx) = unbounded_channel();
    let (persistence_request_tx, persistence_request_rx) = unbounded_channel();
    let (persistence_message_tx, persistence_message_rx) = unbounded_channel();

    let auth_request_tx_clone = auth_request_tx.clone();
    let auth_message_tx_clone = auth_message_tx.clone();
    let persistence_url = client_config
        .persistence_url
        .clone()
        .expect("Expected MUDDLE_PUBLIC_PERSISTENCE_URL");
    let persistence_client = PersistenceClient::new(Default::default(), persistence_url);
    run_async(async move {
        #[cfg(not(target_arch = "wasm32"))]
        let mut serve_redirect_uri_future =
            tokio::task::spawn_local(redirect_uri_server::serve(auth_request_tx_clone.clone()))
                .fuse();
        #[cfg(target_arch = "wasm32")]
        let mut serve_redirect_uri_future =
            tokio::task::spawn_local(listen_local_storage::serve(auth_request_tx_clone.clone()))
                .fuse();
        let mut serve_auth_future = tokio::task::spawn_local(auth::serve_auth_requests(
            persistence_client.clone(),
            auth_config,
            auth_request_rx,
            auth_message_tx_clone,
        ))
        .fuse();
        let matchmaker_requests_handler = MatchmakerRequestsHandler {
            url: matchmaker_url,
            connection_request_rx,
            status_tx,
            matchmaker_message_tx,
        };
        let mut serve_matchmaker_future =
            tokio::task::spawn_local(matchmaker_requests_handler.serve(matchmaker_request_rx))
                .fuse();
        let persistence_requests_handler = PersistenceRequestsHandler {
            client: persistence_client.clone(),
            request_rx: persistence_request_rx,
            message_tx: persistence_message_tx,
        };
        let mut serve_persistence_future =
            tokio::task::spawn_local(persistence_requests_handler.serve()).fuse();
        select! {
            _ = serve_redirect_uri_future => {
                log::warn!("Redirect uri server task finished");
            },
            _ = serve_auth_future => {
                log::warn!("Auth task finished");
            },
            _ = serve_matchmaker_future => {
                log::warn!("Matchmaker task finished");
            },
            _ = serve_persistence_future => {
                log::warn!("Persistence task finished");
            },
        }
    });

    connection_request_tx.send(true).unwrap();
    commands.insert_resource(MainMenuUiChannels {
        auth_request_tx,
        auth_message_tx,
        auth_message_rx,
        connection_request_tx,
        status_rx,
        matchmaker_request_tx,
        matchmaker_message_rx,
        persistence_request_tx,
        persistence_message_rx,
    });
    commands.insert_resource(MatchmakerState {
        status: TcpConnectionStatus::Disconnected,
        id_token: None,
        user_id: None,
    });
}

#[cfg(not(target_arch = "wasm32"))]
pub fn run_async<F>(future: F)
where
    F: Future<Output = ()> + Send + 'static,
{
    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("Cannot start tokio runtime");

        rt.block_on(async move {
            let local = tokio::task::LocalSet::new();
            local
                .run_until(async move {
                    tokio::task::spawn_local(future).await.unwrap();
                })
                .await;
        });
    });
}

#[cfg(target_arch = "wasm32")]
pub fn run_async<F>(future: F)
where
    F: Future<Output = ()> + Send + 'static,
{
    wasm_bindgen_futures::spawn_local(async move {
        let local = tokio::task::LocalSet::new();
        local
            .run_until(async move {
                tokio::task::spawn_local(future).await.unwrap();
            })
            .await;
    });
}

#[derive(SystemParam)]
pub struct MatchmakerParams<'w, 's> {
    matchmaker_state: Option<ResMut<'w, MatchmakerState>>,
    server_to_connect: ResMut<'w, ServerToConnect>,
    main_menu_ui_channels: Option<Res<'w, MainMenuUiChannels>>,
    #[system_param(ignore)]
    marker: PhantomData<&'s ()>,
}

pub fn process_network_events_system(
    mut network_params: NetworkParams,
    mut network_events: EventReader<NetworkEvent>,
    mut current_player_net_id: ResMut<CurrentPlayerNetId>,
    mut players: ResMut<Players>,
    mut update_params: UpdateParams,
    mut matchmaker_params: MatchmakerParams,
) {
    #[cfg(feature = "profiler")]
    puffin::profile_function!();
    for event in network_events.iter() {
        match event {
            NetworkEvent::Connected(handle) => {
                // It doesn't actually mean that we've connected: bevy_networking_turbulence
                // fires the event as soon as we launch. But we also get this even after
                // resetting a connection.
                log::info!("Connected: {}", handle);
                log::info!(
                    "Sending an Initialize message: {}",
                    network_params.connection_state.handshake_id
                );
                network_params
                    .connection_state
                    .set_status(ConnectionStatus::Initialized);
                if let Err(err) = network_params.net.send_message(
                    *handle,
                    Message {
                        // The server is expected to accept any session id for this message.
                        session_id: SessionId::new(0),
                        message: ReliableClientMessage::Initialize,
                    },
                ) {
                    log::error!("Failed to send an Initialize message: {:?}", err);
                }
            }
            NetworkEvent::Disconnected(handle) => {
                log::info!("Disconnected: {}", handle);
            }
            NetworkEvent::Error(handle, err) => {
                log::error!("Network error ({}): {:?}", handle, err);
            }
            _ => {}
        }
    }

    let mut connect_message_to_send = None;
    let mut handshake_message_to_send = None;

    for (handle, connection) in network_params.net.connections.iter_mut() {
        let channels = connection.channels().unwrap();

        while let Some(message) = channels.recv::<Message<UnreliableServerMessage>>() {
            log::trace!(
                "UnreliableServerMessage received on [{}]: {:?}",
                handle,
                message
            );
            let Message {
                message,
                session_id,
            } = message;

            if session_id != network_params.connection_state.session_id
                && matches!(
                    network_params.connection_state.status(),
                    ConnectionStatus::Connected
                )
            {
                log::warn!(
                    "Ignoring a server message: sent session id {} doesn't match {}",
                    session_id,
                    network_params.connection_state.session_id
                );
                continue;
            }

            match message {
                UnreliableServerMessage::Handshake(message_id) => {
                    log::info!("Received Handshake message: {}", message_id);
                    let expected_handshake_id =
                        network_params.connection_state.handshake_id - MessageId::new(1);
                    if !matches!(
                        network_params.connection_state.status(),
                        ConnectionStatus::Connecting
                    ) || message_id != expected_handshake_id
                    {
                        log::warn!(
                            "Ignoring Handshake message. Connection status: {:?}, expected handshake id: {}, received handshake id: {}",
                            network_params.connection_state.status(),
                            network_params.connection_state.handshake_id,
                            message_id
                        );
                        continue;
                    }
                    network_params
                        .connection_state
                        .set_status(ConnectionStatus::Handshaking);
                    update_params.initial_rtt.received_at = Some(Instant::now());
                    let id_token = matchmaker_params
                        .matchmaker_state
                        .as_ref()
                        .and_then(|state| state.id_token.clone());
                    handshake_message_to_send = Some((
                        *handle,
                        Message {
                            session_id: MessageId::new(0),
                            message: ReliableClientMessage::Handshake {
                                message_id,
                                id_token,
                            },
                        },
                    ));

                    // This seems to be the most reliable place to do this. `StartGame` might come
                    // after the first `DeltaUpdate`, so it's not super reliable to restart a game
                    // there. `Handshake`, on the contrary, always comes before both `DeltaUpdate`
                    // and `StartGame`. Restarting on disconnect might work just fine too, but I
                    // thought that `Handshake` probably comes with less edge-cases, since we
                    // always get it before starting the game.
                    current_player_net_id.0 = None;
                    update_params.restart_game_commands.push(RestartGame);
                }
                UnreliableServerMessage::DeltaUpdate(update) => {
                    let mut skip_update = false;
                    if let Err(err) = network_params
                        .connection_state
                        .acknowledge_incoming(update.frame_number)
                    {
                        log::warn!(
                            "Failed to acknowledge with frame {}, skipping: {:?}",
                            update.frame_number,
                            err
                        );
                        skip_update = true;
                    }
                    if let (Some(ack_frame_number), ack_bit_set) = update.acknowledgments {
                        match network_params
                            .connection_state
                            .apply_outgoing_acknowledgements(ack_frame_number, ack_bit_set)
                        {
                            Err(err @ AcknowledgeError::OutOfRange { .. }) => {
                                log::warn!(
                                    "Can't apply acknowledgments for frame {} (current frame: {}), skipping: {:?}",
                                    ack_frame_number,
                                    update_params.game_time.frame_number,
                                    err
                                );
                                skip_update = true;
                            }
                            Err(
                                err @ AcknowledgeError::Inconsistent
                                | err @ AcknowledgeError::InvalidStep,
                            ) => {
                                log::warn!(
                                    "Can't apply acknowledgment for frame {} (current frame: {}), disconnecting: {:?}",
                                    ack_frame_number,
                                    update_params.game_time.frame_number,
                                    err
                                );
                                network_params.connection_state.set_status(
                                    ConnectionStatus::Disconnecting(
                                        DisconnectReason::InvalidUpdate,
                                    ),
                                );
                                return;
                            }
                            Ok(_) => {
                                if !skip_update {
                                    let (newest_incoming_ack, _) =
                                        network_params.connection_state.incoming_acknowledgments();
                                    if newest_incoming_ack.unwrap() > update.frame_number {
                                        log::debug!(
                                            "Old delta update (current: {}, newest: {}), skipping",
                                            update.frame_number,
                                            newest_incoming_ack.unwrap()
                                        );
                                        skip_update = true;
                                    }
                                }
                            }
                        }
                    }
                    skip_update = skip_update || current_player_net_id.0.is_none();
                    if skip_update {
                        continue;
                    }

                    if !can_process_delta_update_message(&update_params.game_time, &update) {
                        log::warn!(
                            "Can't process update for frame {} (current frame: {}), skipping",
                            update.frame_number,
                            update_params.game_time.frame_number
                        );
                        continue;
                    }

                    process_delta_update_message(
                        update,
                        &network_params.connection_state,
                        current_player_net_id.0,
                        &mut players,
                        &mut update_params,
                    );
                }
            }

            network_params
                .connection_state
                .last_valid_message_received_at = Instant::now();
        }

        while let Some(message) = channels.recv::<Message<ReliableServerMessage>>() {
            log::trace!(
                "ReliableServerMessage received on [{}]: {:?}",
                handle,
                message
            );
            let Message {
                message,
                session_id,
            } = message;

            // It is assumed that we can't get the same reliable message twice.
            // (Hopefully, the underlying stack does guarantee that.)
            let ignore_session_id_check = matches!(message, ReliableServerMessage::StartGame(_));

            if session_id != network_params.connection_state.session_id && !ignore_session_id_check
            {
                log::warn!(
                    "Ignoring a server message: sent session id {} doesn't match {}",
                    session_id,
                    network_params.connection_state.session_id
                );
                continue;
            }

            match message {
                ReliableServerMessage::Initialize => {
                    if !matches!(
                        network_params.connection_state.status(),
                        ConnectionStatus::Initialized,
                    ) {
                        continue;
                    }

                    log::info!("Initialize message received");
                    connect_message_to_send = Some((
                        *handle,
                        Message {
                            // The server is expected to accept any session id for this message.
                            session_id: SessionId::new(0),
                            message: UnreliableClientMessage::Connect(
                                network_params.connection_state.handshake_id,
                            ),
                        },
                    ));
                    update_params.initial_rtt.sent_at = Some(Instant::now());
                    network_params.connection_state.handshake_id += MessageId::new(1);
                    network_params
                        .connection_state
                        .set_status(ConnectionStatus::Connecting);
                }
                ReliableServerMessage::StartGame(start_game) => {
                    let expected_handshake_id =
                        network_params.connection_state.handshake_id - MessageId::new(1);
                    if start_game.handshake_id != expected_handshake_id {
                        log::warn!(
                            "Ignoring a StartGame message: handshake id {} doesn't match {}",
                            start_game.handshake_id,
                            expected_handshake_id
                        );
                        continue;
                    }

                    network_params.connection_state.session_id = session_id;
                    network_params
                        .connection_state
                        .set_status(ConnectionStatus::Connected);
                    log::info!(
                        "Starting the game (update frame: {})",
                        start_game.game_state.frame_number
                    );
                    process_start_game_message(
                        start_game,
                        &mut network_params.connection_state,
                        &mut current_player_net_id,
                        &mut players,
                        &mut update_params,
                    );
                }
                ReliableServerMessage::ConnectedPlayer((net_id, connected_player)) => {
                    process_connected_player_message(net_id, connected_player, &mut players);
                }
                ReliableServerMessage::DisconnectedPlayer(disconnected_player) => {
                    process_disconnected_player_message(disconnected_player, &mut players);
                }
                ReliableServerMessage::SpawnLevelObject(spawn_level_object) => {
                    update_params
                        .simulation_time
                        .rewind(spawn_level_object.command.frame_number);
                    update_params.level_object_correlations.correlate(
                        spawn_level_object.correlation_id,
                        spawn_level_object.command.object.net_id,
                    );
                    update_params
                        .spawn_level_object_commands
                        .push(spawn_level_object.command);
                }
                ReliableServerMessage::UpdateLevelObject(update_level_object) => {
                    update_params
                        .simulation_time
                        .rewind(update_level_object.frame_number);
                    update_params
                        .spawn_level_object_commands
                        .push(update_level_object);
                }
                ReliableServerMessage::DespawnLevelObject(despawn_level_object) => {
                    update_params
                        .simulation_time
                        .rewind(despawn_level_object.frame_number);
                    update_params
                        .despawn_level_object_commands
                        .push(despawn_level_object);
                }
                ReliableServerMessage::SwitchRole(switch_role) => {
                    update_params
                        .simulation_time
                        .rewind(switch_role.frame_number);
                    let net_id = switch_role.net_id;
                    update_params.switch_role_commands.push(SwitchPlayerRole {
                        net_id,
                        role: switch_role.role,
                        frame_number: switch_role.frame_number,
                        is_player_frame_simulated: current_player_net_id
                            .0
                            .map_or(false, |current_player_net_id| {
                                current_player_net_id == net_id
                            }),
                    });
                }
                ReliableServerMessage::RespawnPlayer(respawn_player) => {
                    if let Some(player) = players.get_mut(&respawn_player.net_id) {
                        player.respawning_at =
                            Some((respawn_player.frame_number, respawn_player.reason));
                        match respawn_player.reason {
                            RespawnPlayerReason::Finish => {
                                player.finishes += 1;
                            }
                            RespawnPlayerReason::Death => {
                                player.deaths += 1;
                            }
                        }
                    } else {
                        log::warn!(
                            "Received RespawnPlayer message for a player that doesn't exist: {:?}",
                            respawn_player.net_id
                        );
                    }
                }
                ReliableServerMessage::Disconnect(reason) => {
                    log::info!("Server closed the connection: {:?}", reason);
                    if let DisconnectReason::InvalidJwt = reason {
                        if let Some(matchmaker_state) = matchmaker_params.matchmaker_state.as_mut()
                        {
                            matchmaker_state.id_token = None;
                            **matchmaker_params.server_to_connect = None;
                            matchmaker_params
                                .main_menu_ui_channels
                                .unwrap()
                                .auth_message_tx
                                .send(AuthMessage::InvalidOrExpiredAuthError)
                                .expect("Failed to send an auth update");
                        }
                    }
                    network_params
                        .connection_state
                        .set_status(ConnectionStatus::Disconnecting(reason));
                    return;
                }
            }

            network_params
                .connection_state
                .last_valid_message_received_at = Instant::now();
        }

        while channels
            .recv::<Message<UnreliableClientMessage>>()
            .is_some()
        {
            log::error!(
                "Unexpected UnreliableClientMessage received on [{}]",
                handle
            );
        }
        while channels.recv::<Message<ReliableClientMessage>>().is_some() {
            log::error!("Unexpected ReliableClientMessage received on [{}]", handle);
        }
    }

    if let Some((handle, message)) = connect_message_to_send {
        if let Err(err) = network_params.net.send_message(handle, message) {
            log::error!("Failed to send Connect message: {:?}", err);
        }
    }
    if let Some((handle, message)) = handshake_message_to_send {
        if let Err(err) = network_params.net.send_message(handle, message) {
            log::error!("Failed to send Handshake message: {:?}", err);
        }
    }
}

pub fn maintain_connection_system(
    time: Res<GameTime>,
    client_config: Res<MuddleClientConfig>,
    matchmaker_state: Option<ResMut<MatchmakerState>>,
    matchmaker_channels: Option<ResMut<MainMenuUiChannels>>,
    mut server_to_connect: ResMut<ServerToConnect>,
    mut network_params: NetworkParams,
    mut initial_rtt: ResMut<InitialRtt>,
) {
    #[cfg(feature = "profiler")]
    puffin::profile_function!();

    if matches!(
        network_params.connection_state.status(),
        ConnectionStatus::Connected
    ) && server_to_connect.is_some()
    {
        **server_to_connect = None;
        if let Some(matchmaker_channels) = matchmaker_channels.as_ref() {
            matchmaker_channels
                .connection_request_tx
                .send(false)
                .expect("Failed to write to a channel (matchmaker connection request)");
        }
    }

    let mut matchmaker = matchmaker_state.zip(matchmaker_channels);
    if let Some((matchmaker_state, matchmaker_channels)) = matchmaker.as_mut() {
        loop {
            match matchmaker_channels.status_rx.try_recv() {
                Ok(status) => {
                    log::debug!("Updating matchmaker status: {:?}", status);
                    matchmaker_state.status = status;
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    panic!("Failed to read from a channel (matchmaker status)")
                }
            }
        }
    }

    let connection_is_uninitialized = matches!(
        network_params.connection_state.status(),
        ConnectionStatus::Uninitialized
    );

    // TODO: if a client isn't getting any updates, we may also want to pause the
    // game and wait for  some time for a server to respond.

    let connection_timeout = Instant::now().duration_since(
        network_params
            .connection_state
            .last_valid_message_received_at,
    ) > Duration::from_millis(CONNECTION_TIMEOUT_MILLIS);

    if connection_timeout && !connection_is_uninitialized {
        log::warn!("Connection timeout, resetting");
    }

    let (newest_acknowledged_incoming_packet, _) =
        network_params.connection_state.incoming_acknowledgments();
    let is_falling_behind = matches!(
        network_params.connection_state.status(),
        ConnectionStatus::Connected
    ) && newest_acknowledged_incoming_packet.map_or(false, |packet| {
        if packet > time.frame_number {
            (packet - time.frame_number).value() > COMPONENT_FRAMEBUFFER_LIMIT / 2
        } else {
            false
        }
    });

    if is_falling_behind && !connection_is_uninitialized {
        log::warn!(
            "The client is falling behind, resetting (newest acknowledged frame: {}, current frame: {})",
            newest_acknowledged_incoming_packet.unwrap(),
            time.frame_number
        );
    }

    if !connection_is_uninitialized && connection_timeout
        || is_falling_behind
        || matches!(
            network_params.connection_state.status(),
            ConnectionStatus::Disconnecting(_) | ConnectionStatus::Disconnected
        )
    {
        network_params.net.connections.clear();
        initial_rtt.sent_at = None;
        network_params
            .connection_state
            .set_status(ConnectionStatus::Uninitialized);
    }

    if network_params.net.connections.is_empty() {
        let addr = if let Some((matchmaker_state, matchmaker_channels)) = matchmaker.as_mut() {
            if matches!(matchmaker_state.status, TcpConnectionStatus::Disconnected) {
                log::trace!("Requesting a connection to the matchmaker");
                matchmaker_channels
                    .connection_request_tx
                    .send(true)
                    .expect("Failed to write to a channel (matchmaker connection request)");
                return;
            }

            let Some(server) = &**server_to_connect else {
                return;
            };
            log::info!("Connecting to {}: {}", server.name, server.addr);
            format!("http://{}", server.addr)
        } else {
            let server_socket_addr = client_config
                .server_addr
                .unwrap_or_else(|| SocketAddr::new(DEFAULT_SERVER_IP_ADDR, DEFAULT_SERVER_PORT));

            log::info!("Connecting to {}", server_socket_addr);
            format!("http://{server_socket_addr}")
        };

        network_params.net.connect(&addr);
    }
}

#[derive(SystemParam)]
pub struct PlayerUpdateParams<'w, 's> {
    player_directions: Query<'w, 's, &'static PlayerDirection>,
}

pub fn send_network_updates_system(
    time: Res<GameTime>,
    mut network_params: NetworkParams,
    current_player_net_id: Res<CurrentPlayerNetId>,
    players: Res<Players>,
    player_registry: Res<EntityRegistry<PlayerNetId>>,
    player_update_params: PlayerUpdateParams,
) {
    #[cfg(feature = "profiler")]
    puffin::profile_function!();
    let (connection_handle, address) = match network_params.net.connections.iter_mut().next() {
        Some((&handle, connection)) => (handle, connection.remote_address()),
        None => return,
    };

    if !matches!(
        network_params.connection_state.status(),
        ConnectionStatus::Connected
    ) {
        return;
    }

    log::trace!("Broadcast updates for frame {}", time.frame_number);
    let current_player_net_id = match current_player_net_id.0 {
        Some(net_id) => net_id,
        None => return,
    };

    let player = players
        .get(&current_player_net_id)
        .expect("Expected a registered player when current_player_net_id is set");

    let player_entity = player_registry.get_entity(current_player_net_id);
    if matches!(player.role, PlayerRole::Runner) && player_entity.is_none() {
        return;
    }

    network_params
        .connection_state
        // Clients don't resend updates, so we can forget about unacknowledged packets.
        .add_outgoing_packet(time.frame_number, Instant::now());

    let inputs = match player.role {
        PlayerRole::Runner => {
            let player_entity = player_entity.unwrap(); // is checked above

            let player_direction = player_update_params
                .player_directions
                .get(player_entity)
                .expect("Expected a created spawned player");

            // TODO: this makes the client send more packets than the server actually needs,
            // as lost packets  never get marked as acknowledged, even though we
            // resend updates in future frames. Fix it.
            let first_unacknowledged_frame = network_params
                .connection_state
                .first_unacknowledged_outgoing_packet()
                .expect("Expected at least the new packet for the current frame");
            let mut inputs: Vec<RunnerInput> = Vec::new();
            // TODO: deduplicate updates (the same code is written for server).
            for (frame_number, &direction) in player_direction
                .buffer
                .iter_with_interpolation()
                // TODO: should client always send redundant inputs or only the current ones (unless
                // packet loss is detected)?
                .skip_while(|(frame_number, _)| *frame_number < first_unacknowledged_frame)
            {
                if Some(direction) != inputs.last().map(|i| i.direction) {
                    inputs.push(RunnerInput {
                        frame_number,
                        direction,
                    });
                }
            }
            PlayerInputs::Runner { inputs }
        }
        PlayerRole::Builder => PlayerInputs::Builder,
    };

    let message = UnreliableClientMessage::PlayerUpdate(PlayerUpdate {
        frame_number: time.frame_number,
        acknowledgments: network_params.connection_state.incoming_acknowledgments(),
        inputs,
    });
    let result = network_params.net.send_message(
        connection_handle,
        Message {
            session_id: network_params.connection_state.session_id,
            message,
        },
    );
    if let Err(err) = result {
        log::error!("Failed to send a message to {:?}: {:?}", address, err);
    }
}

pub fn send_requests_system(
    mut network_params: NetworkParams,
    mut player_requests: ResMut<PlayerRequestsQueue>,
    mut level_object_requests: ResMut<LevelObjectRequestsQueue>,
) {
    #[cfg(feature = "profiler")]
    puffin::profile_function!();
    let (connection_handle, _) = match network_params.net.connections.iter_mut().next() {
        Some((&handle, connection)) => (handle, connection.remote_address()),
        None => return,
    };

    // TODO: refactor this to be a run-criteria.
    if !matches!(
        network_params.connection_state.status(),
        ConnectionStatus::Connected
    ) {
        return;
    }

    for switch_role_request in std::mem::take(&mut player_requests.switch_role) {
        if let Err(err) = network_params.net.send_message(
            connection_handle,
            Message {
                session_id: network_params.connection_state.session_id,
                message: ReliableClientMessage::SwitchRole(switch_role_request),
            },
        ) {
            log::error!("Failed to send SwitchRole message: {:?}", err);
        }
    }
    for spawn_request in std::mem::take(&mut level_object_requests.spawn_requests) {
        if let Err(err) = network_params.net.send_message(
            connection_handle,
            Message {
                session_id: network_params.connection_state.session_id,
                message: ReliableClientMessage::SpawnLevelObject(spawn_request),
            },
        ) {
            log::error!("Failed to send SwitchRole message: {:?}", err);
        }
    }
    for update_request in std::mem::take(&mut level_object_requests.update_requests) {
        if let Err(err) = network_params.net.send_message(
            connection_handle,
            Message {
                session_id: network_params.connection_state.session_id,
                message: ReliableClientMessage::UpdateLevelObject(update_request),
            },
        ) {
            log::error!("Failed to send SwitchRole message: {:?}", err);
        }
    }
    for despawn_request in std::mem::take(&mut level_object_requests.despawn_requests) {
        if let Err(err) = network_params.net.send_message(
            connection_handle,
            Message {
                session_id: network_params.connection_state.session_id,
                message: ReliableClientMessage::DespawnLevelObject(despawn_request),
            },
        ) {
            log::error!("Failed to send SwitchRole message: {:?}", err);
        }
    }
}

fn can_process_delta_update_message(time: &GameTime, delta_update: &DeltaUpdate) -> bool {
    let frames_diff = time
        .frame_number
        .diff_abs(delta_update.frame_number)
        .value();
    frames_diff < COMPONENT_FRAMEBUFFER_LIMIT / 2
}

/// We need to access an actual value on each (fresh) delta update message, so
/// we write it for every frame, as we can't predict when we'll receive those.
pub fn fill_actual_frames_ahead_system(
    time: Res<GameTime>,
    simulation_time: Res<SimulationTime>,
    mut target_frames_ahead: ResMut<TargetFramesAhead>,
) {
    target_frames_ahead
        .actual_frames_ahead
        .insert(time.frame_number, simulation_time.player_frames_ahead());
}

fn process_delta_update_message(
    delta_update: DeltaUpdate,
    connection_state: &ConnectionState,
    current_player_net_id: Option<PlayerNetId>,
    players: &mut Players,
    update_params: &mut UpdateParams,
) {
    log::trace!("Processing DeltaUpdate message: {:?}", delta_update);
    if delta_update.frame_number < update_params.simulation_time.server_frame {
        log::trace!(
            "Delta update is late (server simulation frame: {}, update frame: {})",
            update_params.simulation_time.server_frame,
            delta_update.frame_number
        );
    }

    sync_clock(&delta_update, connection_state, update_params);

    // Despawning players that aren't mentioned in the delta update.
    let players_to_remove: Vec<PlayerNetId> = players
        .iter()
        .filter_map(|(player_net_id, player)| {
            if !delta_update
                .players
                .iter()
                .any(|player| player.net_id == *player_net_id)
                && matches!(player.role, PlayerRole::Runner)
            {
                Some(*player_net_id)
            } else {
                None
            }
        })
        .collect();

    for player_net_id in players_to_remove {
        let is_spawned = update_params
            .player_entities
            .get_entity(player_net_id)
            .and_then(|player_entity| update_params.spawned_query.get(player_entity).ok())
            .map_or(false, |spawned| {
                spawned.is_spawned(delta_update.frame_number)
            });
        if is_spawned {
            log::debug!(
                "Player ({}) is not mentioned in the delta update (update frame: {}, current frame: {})",
                player_net_id.0,
                delta_update.frame_number,
                update_params.game_time.frame_number
            );
            update_params.despawn_player_commands.push(DespawnPlayer {
                net_id: player_net_id,
                frame_number: delta_update.frame_number,
                reason: DespawnReason::NetworkUpdate,
            });
        }
    }

    let delta_update_frame = delta_update.frame_number;
    for player_state in delta_update.players {
        let is_spawned = update_params
            .player_entities
            .get_entity(player_state.net_id)
            .and_then(|player_entity| update_params.spawned_query.get(player_entity).ok())
            .map_or(false, |spawned| spawned.is_spawned(delta_update_frame));
        if !is_spawned {
            log::info!("First update with the new player {}", player_state.net_id.0);
            update_params.spawn_player_commands.push(SpawnPlayer {
                net_id: player_state.net_id,
                start_position: player_state.position,
                is_player_frame_simulated: current_player_net_id.expect(
                    "Processing delta updates isn't expected before processing StartGame message",
                ) == player_state.net_id,
            });
            players
                .entry(player_state.net_id)
                .or_insert_with(|| Player::new(PlayerRole::Runner));
        }

        let direction_updates = update_params.player_updates.get_direction_mut(
            player_state.net_id,
            delta_update.frame_number,
            COMPONENT_FRAMEBUFFER_LIMIT,
        );
        direction_updates.insert(
            delta_update_frame,
            Some(PlayerDirectionUpdate {
                direction: player_state.direction,
                is_processed_client_input: None,
            }),
        );

        let position_updates = update_params.player_updates.get_position_mut(
            player_state.net_id,
            delta_update.frame_number,
            COMPONENT_FRAMEBUFFER_LIMIT,
        );
        log::trace!(
            "Updating position for player {} (frame_number: {}): {:?}",
            player_state.net_id.0,
            delta_update.frame_number,
            player_state.position
        );
        position_updates.insert(delta_update.frame_number, Some(player_state.position));
    }

    // There's no need to rewind if we haven't started the game.
    if let ConnectionStatus::Connected = connection_state.status() {
        log::trace!(
            "Rewinding to frame {} (current server frame: {}, current player frame: {})",
            delta_update.frame_number,
            update_params.simulation_time.server_frame,
            update_params.simulation_time.player_frame
        );
        update_params
            .simulation_time
            .rewind(delta_update.frame_number);
    }
}

/// Returns the "frame ahead" number that has to be applied to this delta
/// update.
///
/// ## Terms
///
/// - `target_frames_ahead` - for how many frames we want the local player time
///   to be ahead of the local server time.
/// - `delay_server_time` - for how many frames we want the local server time to
///   be behind the delta updates.
/// - `server_reported_frames_ahead` - by how many frames we want to decrease
///   our `target_frames_ahead`.
///
/// ## Scenarios
///
/// ### Client has a lag spike
///
/// Multiple packets may pile up in the buffer during the spike. Once the last
/// one is processed, `delay_server_time` becomes negative. The client starts
/// running faster, while also shrinking `target_frames_ahead`. In RTT frames,
/// the server reports with a negative `server_reported_frames_ahead`. which is
/// expected to be about equal to the initial `delay_server_time` once recovered
/// from the spike. The client will start extending `target_frames_ahead` back
/// and keep running faster until it's caught up.
///
/// When a server has a lag spike, all the mentioned variables get the opposite
/// signs, and clients starts running slower.
///
/// ### Ping increases
///
/// Packets won't be delivered for some time, which may result into
/// extrapolating other players' state. When the next packet arrives,
/// `delay_server_time` becomes positive, which will make the client run slower.
/// In about from 0.5 to 1.0 RTT frames, the server will report a decreased
/// `server_reported_frames_ahead`, which will keep decreasing because the
/// client was running slower for some time.
fn sync_clock(
    delta_update: &DeltaUpdate,
    connection_state: &ConnectionState,
    update_params: &mut UpdateParams,
) {
    let (newest_acknowledged_input, _) = delta_update.acknowledgments;

    let actual_frames_ahead = update_params.simulation_time.player_frames_ahead();

    // We store a history of `actual_frames_ahead` values to be able to correlate
    // them to `server_reported_frames_ahead` values that we calculate. If we
    // see that we mispredicted `target_frames_ahead` for that particular frame
    // and updates came earlier/later, we correct the target value.
    let frames_ahead_at_input = newest_acknowledged_input.map(|newest_acknowledged_input| {
        update_params
            .target_frames_ahead
            .actual_frames_ahead
            .get(newest_acknowledged_input)
            .copied()
            .unwrap_or_else(|| {
                log::warn!("Acknowledged input isn't stored in the `actual_frames_ahead` buffer: {newest_acknowledged_input}");
                actual_frames_ahead
            })
    }).unwrap_or(actual_frames_ahead);

    let server_reported_frames_ahead =
        newest_acknowledged_input.map_or(0i32, |newest_input_frame| {
            newest_input_frame.value() as i32 - delta_update.frame_number.value() as i32
        }) as i16;

    // Update rtt, packet loss and jitter values.
    let frames_rtt = SIMULATIONS_PER_SECOND * connection_state.rtt_millis() / 1000.0;
    let packet_loss_buffer = frames_rtt * connection_state.packet_loss();
    let jitter_buffer = packet_loss_buffer
        + SIMULATIONS_PER_SECOND * connection_state.jitter_millis() * 2.0 / 1000.0;

    // Adjusting the speed to synchronize with the server clock.
    let new_delay = (update_params.simulation_time.server_frame.value() as i32
        + jitter_buffer.ceil() as i32
        - delta_update.frame_number.value() as i32) as i16;
    let delay_diff = new_delay - update_params.delay_server_time.frame_count;
    update_params.delay_server_time.frame_count = new_delay;

    // Calculate how many frames ahead of the server we want to be.
    let jitter_buffer_len_to_add =
        jitter_buffer.ceil() - update_params.target_frames_ahead.jitter_buffer_len as f32;
    let new_target_frames_ahead = (((frames_ahead_at_input as i16 - server_reported_frames_ahead)
        .max(0)
        + jitter_buffer_len_to_add.ceil() as i16) as u16)
        .saturating_add_signed(delay_diff);

    let diff = new_target_frames_ahead as i16 - update_params.target_frames_ahead.target as i16;

    update_params.target_frames_ahead.target = new_target_frames_ahead;
    update_params.target_frames_ahead.jitter_buffer_len =
        (update_params.target_frames_ahead.jitter_buffer_len as i16
            + jitter_buffer_len_to_add.ceil() as i16) as u16;

    // Estimating the actual server time.
    let new_estimated_server_time =
        delta_update.frame_number + FrameNumber::new(new_target_frames_ahead);
    if new_estimated_server_time > update_params.estimated_server_time.frame_number {
        update_params.estimated_server_time.frame_number = new_estimated_server_time;
        update_params.estimated_server_time.updated_at = update_params.game_time.frame_number;
    }

    if cfg!(debug_assertions) {
        log::trace!("server_reported_frames_ahead: {}, at_input: {}, new_target_frames_ahead: {}, diff: {}, jitter: {}",
            server_reported_frames_ahead,
            frames_ahead_at_input,
            new_target_frames_ahead,
            diff,
            jitter_buffer.ceil() as i16,
        );
        log::trace!(
            "pf: {}, sf: {}, uf: {}, delay: {}, estimated server frame: {}",
            update_params.simulation_time.player_frame.value(),
            update_params.simulation_time.server_frame.value(),
            delta_update.frame_number.value(),
            update_params.delay_server_time.frame_count,
            update_params.estimated_server_time.frame_number.value(),
        );
    }
}

fn process_start_game_message(
    start_game: StartGame,
    connection_state: &mut ConnectionState,
    current_player_net_id: &mut CurrentPlayerNetId,
    players: &mut Players,
    update_params: &mut UpdateParams,
) {
    log::debug!("Processing StartGame message: {:?}", start_game);
    let initial_rtt = update_params.initial_rtt.duration_secs().unwrap() * 1000.0;
    log::debug!("Initial rtt: {}", initial_rtt);
    connection_state
        .set_initial_rtt_millis(update_params.initial_rtt.duration_secs().unwrap() * 1000.0);

    current_player_net_id.0 = Some(start_game.net_id);
    players.insert(
        start_game.net_id,
        Player {
            uuid: start_game.uuid,
            ..Player::new_with_nickname(PlayerRole::Runner, start_game.nickname)
        },
    );
    update_params.game_time.session += 1;
    let rtt_frames =
        FrameNumber::new((SIMULATIONS_PER_SECOND * connection_state.rtt_millis() / 1000.0) as u16);
    let half_rtt_frames = FrameNumber::new(
        (SIMULATIONS_PER_SECOND * connection_state.rtt_millis() / 1000.0 / 2.0) as u16,
    );
    update_params.simulation_time.server_generation = start_game.generation;
    update_params.simulation_time.player_generation = start_game.generation;
    update_params.simulation_time.server_frame = start_game.game_state.frame_number;
    let (player_frame, overflown) = start_game.game_state.frame_number.add(rtt_frames);
    update_params.simulation_time.player_frame = player_frame;
    if overflown {
        update_params.simulation_time.player_generation += 1;
    }

    // Re-init the buffer. If we just attempt to insert on start, it may panic due
    // to the big difference between the buffer start frame (0) and game start
    // frame.
    update_params.target_frames_ahead.target = rtt_frames.value();
    let mut new_buffer = Framebuffer::new(
        player_frame,
        update_params
            .target_frames_ahead
            .actual_frames_ahead
            .limit(),
    );
    new_buffer.insert(player_frame, update_params.target_frames_ahead.target);
    update_params.target_frames_ahead.actual_frames_ahead = new_buffer;

    update_params.game_time.frame_number = update_params.simulation_time.player_frame;

    update_params.estimated_server_time.frame_number =
        start_game.game_state.frame_number + half_rtt_frames;
    update_params.estimated_server_time.updated_at = update_params.game_time.frame_number;

    for (player_net_id, connected_player) in start_game.players {
        if player_net_id == current_player_net_id.0.unwrap() {
            continue;
        }

        players
            .entry(player_net_id)
            .and_modify(|player| {
                let deaths = player.deaths;
                let finishes = player.finishes;
                *player = connected_player.clone();
                player.deaths += deaths;
                player.finishes += finishes;
            })
            .or_insert_with(|| connected_player.clone());
        if connected_player.role == PlayerRole::Runner && connected_player.respawning_at.is_none() {
            if let Some(start_position) =
                player_start_position(player_net_id, &start_game.game_state)
            {
                log::info!(
                    "Spawning player {}: {}",
                    player_net_id.0,
                    connected_player.nickname
                );

                update_params.spawn_player_commands.push(SpawnPlayer {
                    net_id: player_net_id,
                    start_position,
                    is_player_frame_simulated: false,
                });
            } else {
                log::error!(
                    "Player ({}) position isn't found in the game state",
                    player_net_id.0
                );
            }
        } else {
            log::info!(
                "Adding player {} as a Builder: {}",
                player_net_id.0,
                connected_player.nickname
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
    player_net_id: PlayerNetId,
    connected_player: Player,
    players: &mut Players,
) {
    // Player is spawned when the first DeltaUpdate with it arrives, so we don't do
    // it here.
    log::info!(
        "A new player ({}) connected: {}",
        player_net_id.0,
        connected_player.nickname
    );
    players
        .entry(player_net_id)
        .and_modify(|player| {
            let deaths = player.deaths;
            let finishes = player.finishes;
            *player = connected_player.clone();
            player.deaths += deaths;
            player.finishes += finishes;
        })
        .or_insert(connected_player);
}

fn process_disconnected_player_message(
    disconnected_player: DisconnectedPlayer,
    players: &mut Players,
) {
    log::info!("A player ({}) disconnected", disconnected_player.net_id.0);
    if let Some(player) = players.get_mut(&disconnected_player.net_id) {
        player.is_connected = false;
    } else {
        log::error!(
            "A disconnected player didn't exist: {}",
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
