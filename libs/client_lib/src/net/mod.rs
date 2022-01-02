use crate::{
    input::{LevelObjectRequestsQueue, PlayerRequestsQueue},
    net::auth::persistence_url,
    websocket::WebSocketStream,
    CurrentPlayerNetId, EstimatedServerTime, InitialRtt, LevelObjectCorrelations, PlayerDelay,
    TargetFramesAhead,
};
use auth::{AuthConfig, AuthMessage, AuthRequest};
use bevy::{ecs::system::SystemParam, log, prelude::*, utils::HashMap};
use bevy_networking_turbulence::{NetworkEvent, NetworkResource};
use chrono::Utc;
use futures::{select, FutureExt, StreamExt, TryStreamExt};
use mr_messages_lib::{MatchmakerMessage, Server};
use mr_shared_lib::{
    framebuffer::FrameNumber,
    game::{
        commands::{
            DeferredQueue, DespawnLevelObject, DespawnPlayer, RestartGame, SpawnPlayer,
            SwitchPlayerRole, UpdateLevelObject,
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
    player::{Player, PlayerDirectionUpdate, PlayerRole, PlayerUpdates},
    registry::EntityRegistry,
    simulations_per_second, try_parse_from_env, GameTime, SimulationTime,
    COMPONENT_FRAMEBUFFER_LIMIT,
};
use std::{
    future::Future,
    net::{IpAddr, Ipv4Addr, SocketAddr},
};
use tokio::sync::mpsc::{
    error::TryRecvError, unbounded_channel, UnboundedReceiver, UnboundedSender,
};
use url::Url;

pub mod auth;

#[cfg(target_arch = "wasm32")]
mod listen_local_storage;
#[cfg(not(target_arch = "wasm32"))]
mod redirect_uri_server;

const DEFAULT_SERVER_PORT: u16 = 3455;
const DEFAULT_SERVER_IP_ADDR: IpAddr = IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1));

#[derive(SystemParam)]
pub struct UpdateParams<'a> {
    simulation_time: ResMut<'a, SimulationTime>,
    game_time: ResMut<'a, GameTime>,
    player_entities: Res<'a, EntityRegistry<PlayerNetId>>,
    estimated_server_time: ResMut<'a, EstimatedServerTime>,
    target_frames_ahead: ResMut<'a, TargetFramesAhead>,
    player_delay: ResMut<'a, PlayerDelay>,
    initial_rtt: ResMut<'a, InitialRtt>,
    player_updates: ResMut<'a, PlayerUpdates>,
    restart_game_commands: ResMut<'a, DeferredQueue<RestartGame>>,
    level_object_correlations: ResMut<'a, LevelObjectCorrelations>,
    spawn_level_object_commands: ResMut<'a, DeferredQueue<UpdateLevelObject>>,
    despawn_level_object_commands: ResMut<'a, DeferredQueue<DespawnLevelObject>>,
    spawn_player_commands: ResMut<'a, DeferredQueue<SpawnPlayer>>,
    despawn_player_commands: ResMut<'a, DeferredQueue<DespawnPlayer>>,
    switch_role_commands: ResMut<'a, DeferredQueue<SwitchPlayerRole>>,
    spawned_query: Query<'a, &'static Spawned>,
}

#[derive(SystemParam)]
pub struct NetworkParams<'a> {
    net: ResMut<'a, NetworkResource>,
    connection_state: ResMut<'a, ConnectionState>,
}

#[derive(Debug)]
pub enum TcpConnectionStatus {
    Disconnected,
    Connecting,
    Connected,
}

pub struct MatchmakerState {
    pub status: TcpConnectionStatus,
    pub id_token: Option<String>,
}

pub struct MainMenuUiChannels {
    pub auth_request_tx: UnboundedSender<AuthRequest>,
    pub auth_message_tx: UnboundedSender<AuthMessage>,
    pub auth_message_rx: UnboundedReceiver<AuthMessage>,
    pub connection_request_tx: UnboundedSender<bool>,
    pub status_rx: UnboundedReceiver<TcpConnectionStatus>,
    pub matchmaker_message_rx: UnboundedReceiver<MatchmakerMessage>,
}

pub struct ServerToConnect(pub Server);

pub fn init_matchmaker_connection(mut commands: Commands) {
    let url = match matchmaker_url() {
        Some(url) => url,
        None => {
            return;
        }
    };

    let auth_config = AuthConfig {
        persistence_url: persistence_url().expect("Expected MUDDLE_PERSISTENCE_URL"),
        google_client_id: auth::google_client_id().expect("Expected MUDDLE_GOOGLE_CLIENT_ID"),
        google_client_secret: auth::google_client_secret(),
        auth0_client_id: auth::auth0_client_id().expect("Expected MUDDLE_AUTH0_CLIENT_ID"),
        #[cfg(feature = "unstoppable_resolution")]
        ud_client_id: auth::ud_client_id().expect("Expected MUDDLE_UD_CLIENT_ID"),
        #[cfg(feature = "unstoppable_resolution")]
        ud_secret_id: auth::ud_client_secret().expect("Expected MUDDLE_UD_CLIENT_SECRET"),
    };
    if cfg!(not(target_arch = "wasm32")) {
        auth_config
            .google_client_secret
            .as_ref()
            .expect("Expected MUDDLE_GOOGLE_CLIENT_SECRET");
    }

    log::info!("Matchmaker address: {}", url);

    let (auth_request_tx, auth_request_rx) = unbounded_channel();
    let (auth_message_tx, auth_message_rx) = unbounded_channel();
    let (connection_request_tx, connection_request_rx) = unbounded_channel();
    let (status_tx, status_rx) = unbounded_channel();
    let (matchmaker_message_tx, matchmaker_message_rx) = unbounded_channel();
    let url = url::Url::parse(&format!("ws://{}", url)).unwrap();

    let auth_request_tx_clone = auth_request_tx.clone();
    let auth_message_tx_clone = auth_message_tx.clone();
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
            auth_config,
            auth_request_rx,
            auth_message_tx_clone,
        ))
        .fuse();
        let mut serve_matchmaker_future = tokio::task::spawn_local(serve_matchmaker_connection(
            url,
            connection_request_rx,
            status_tx,
            matchmaker_message_tx,
        ))
        .fuse();
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
        }
    });

    connection_request_tx.send(true).unwrap();
    commands.insert_resource(MainMenuUiChannels {
        auth_request_tx,
        auth_message_tx,
        auth_message_rx,
        connection_request_tx,
        status_rx,
        matchmaker_message_rx,
    });
    commands.insert_resource(MatchmakerState {
        status: TcpConnectionStatus::Disconnected,
        id_token: None,
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

async fn serve_matchmaker_connection(
    url: Url,
    mut connection_request_rx: UnboundedReceiver<bool>,
    status_tx: UnboundedSender<TcpConnectionStatus>,
    matchmaker_message_tx: UnboundedSender<MatchmakerMessage>,
) {
    let mut current_state = false;

    let mut disconnect_request_tx = None;
    let (disconnect_tx, mut disconnect_rx) = tokio::sync::mpsc::unbounded_channel();
    loop {
        let connect = select! {
            connect = connection_request_rx.recv().fuse() => connect,
            _ = disconnect_rx.recv().fuse() => Some(false),
        };
        let connect = match connect {
            Some(connect) => connect,
            None => break,
        };

        if current_state == connect {
            continue;
        }

        if connect {
            log::info!("Connecting to the matchmaker service...");
            let _ = status_tx.send(TcpConnectionStatus::Connecting);
            let message_tx = matchmaker_message_tx.clone();
            let url = url.clone();
            let ws_status_tx = status_tx.clone();
            let disconnect_request_channel = tokio::sync::oneshot::channel();
            disconnect_request_tx = Some(disconnect_request_channel.0);
            tokio::task::spawn_local(handle_matchmaker_connection(
                message_tx,
                url,
                ws_status_tx,
                disconnect_request_channel.1,
                disconnect_tx.clone(),
            ));
        } else {
            // If this fails, the connection is already closed.
            if disconnect_request_tx.take().unwrap().send(()).is_ok() {
                log::info!("Dropping the connection with the matchmaker service...");
                let _ = status_tx.send(TcpConnectionStatus::Disconnected);
            }
        }

        current_state = connect;
    }
    panic!("Failed to read from a channel (matchmaker connection request");
}

async fn handle_matchmaker_connection(
    message_tx: UnboundedSender<MatchmakerMessage>,
    url: Url,
    ws_status_tx: UnboundedSender<TcpConnectionStatus>,
    disconnect_request_rx: tokio::sync::oneshot::Receiver<()>,
    disconnect_tx: tokio::sync::mpsc::UnboundedSender<()>,
) {
    let mut ws_stream = match WebSocketStream::connect(&url).await {
        Ok(ws_stream) => ws_stream.fuse(),
        Err(err) => {
            log::error!("Failed to connect to matchmaker: {:?}", err);
            let _ = ws_status_tx.send(TcpConnectionStatus::Disconnected);
            disconnect_tx.send(()).unwrap();
            return;
        }
    };
    let mut disconnect_rx = disconnect_request_rx.fuse();

    let _ = ws_status_tx.send(TcpConnectionStatus::Connected);
    log::info!("Successfully connected to the matchmacker");

    loop {
        let message = select! {
            message = ws_stream.try_next() => message,
            _ = disconnect_rx => break,
        };

        let message: crate::websocket::Message = match message {
            Ok(Some(message)) => message,
            Ok(None) => {
                // In practice, the stream is unlikely to get exhausted before receiving an error.
                log::warn!("Matchmaker stream has exhausted, disconnecting");
                break;
            }
            Err(err) => {
                log::error!("Matchmaker connection error: {:?}", err);
                break;
            }
        };

        let matchmaker_message = match message {
            crate::websocket::Message::Binary(data) => {
                match bincode::deserialize::<MatchmakerMessage>(&data) {
                    Ok(server_update) => server_update,
                    Err(err) => {
                        log::error!(
                            "Failed to deserialize matchmaker message, disconnecting: {:?}",
                            err
                        );
                        break;
                    }
                }
            }
            _ => continue,
        };

        if let Err(err) = message_tx.send(matchmaker_message) {
            log::error!("Failed to send a matchmaker update: {:?}", err);
        }
    }

    let _ = ws_status_tx.send(TcpConnectionStatus::Disconnected);
    disconnect_tx.send(()).unwrap();
}

#[derive(SystemParam)]
pub struct MatchmakerParams<'a> {
    matchmaker_state: Option<ResMut<'a, MatchmakerState>>,
    server_to_connect: ResMut<'a, Option<ServerToConnect>>,
    main_menu_ui_channels: Option<Res<'a, MainMenuUiChannels>>,
}

pub fn process_network_events(
    mut network_params: NetworkParams,
    mut network_events: EventReader<NetworkEvent>,
    mut current_player_net_id: ResMut<CurrentPlayerNetId>,
    mut players: ResMut<HashMap<PlayerNetId, Player>>,
    mut update_params: UpdateParams,
    mut matchmaker_params: MatchmakerParams,
) {
    #[cfg(feature = "profiler")]
    puffin::profile_function!();
    for event in network_events.iter() {
        match event {
            NetworkEvent::Connected(handle) => {
                // It doesn't actually mean that we've connected: bevy_networking_turbulence
                // fires the event as soon as we launch. But we also get this even after resetting
                // a connection.
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
            network_params.connection_state.last_message_received_at = Utc::now();
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
                    update_params.initial_rtt.received_at = Some(Utc::now());
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
                    if let Err(err) = network_params
                        .connection_state
                        .acknowledge_incoming(update.frame_number)
                    {
                        log::error!(
                            "Failed to acknowledge with frame {}: {:?}",
                            update.frame_number,
                            err
                        );
                    }
                    let mut skip_update = false;
                    if let (Some(ack_frame_number), ack_bit_set) = update.acknowledgments {
                        match network_params
                            .connection_state
                            .apply_outgoing_acknowledgements(ack_frame_number, ack_bit_set)
                        {
                            Err(err @ AcknowledgeError::OutOfRange { .. }) => {
                                log::warn!(
                                    "Can't apply acknowledgments for frame {} (current frame: {}): {:?}",
                                    ack_frame_number,
                                    update_params.game_time.frame_number,
                                    err
                                );
                                skip_update = true;
                            }
                            Err(err) => {
                                log::error!(
                                    "Can't apply acknowledgment for frame {} (current frame: {}): {:?}",
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
                            _ => {}
                        }
                    }
                    skip_update = skip_update || current_player_net_id.0.is_none();
                    if !skip_update {
                        if !can_process_delta_update_message(&update_params.game_time, &update) {
                            log::error!(
                                "Can't process update for frame {} (current frame: {})",
                                update.frame_number,
                                update_params.game_time.frame_number
                            );
                            network_params.connection_state.set_status(
                                ConnectionStatus::Disconnecting(DisconnectReason::InvalidUpdate),
                            );
                            return;
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
            }
        }

        while let Some(message) = channels.recv::<Message<ReliableServerMessage>>() {
            log::trace!(
                "ReliableServerMessage received on [{}]: {:?}",
                handle,
                message
            );
            network_params.connection_state.last_message_received_at = Utc::now();
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
                    update_params.initial_rtt.sent_at = Some(Utc::now());
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
                            *matchmaker_params.server_to_connect = None;
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

pub fn maintain_connection(
    time: Res<GameTime>,
    matchmaker_state: Option<ResMut<MatchmakerState>>,
    matchmaker_channels: Option<ResMut<MainMenuUiChannels>>,
    mut server_to_connect: ResMut<Option<ServerToConnect>>,
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
        *server_to_connect = None;
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

    // TODO: if a client isn't getting any updates, we may also want to pause the game and wait for
    //  some time for a server to respond.

    let connection_timeout = Utc::now()
        .signed_duration_since(network_params.connection_state.last_message_received_at)
        .to_std()
        .unwrap()
        > std::time::Duration::from_millis(CONNECTION_TIMEOUT_MILLIS);

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
        if let Some((matchmaker_state, matchmaker_channels)) = matchmaker.as_mut() {
            if matches!(matchmaker_state.status, TcpConnectionStatus::Disconnected) {
                log::trace!("Requesting a connection to the matchmaker");
                matchmaker_channels
                    .connection_request_tx
                    .send(true)
                    .expect("Failed to write to a channel (matchmaker connection request)");
                return;
            }

            if let Some(ServerToConnect(server)) = &*server_to_connect {
                log::info!("Connecting to {}: {}", server.name, server.addr);
                network_params.net.connect(server.addr);
            }
        } else {
            let server_socket_addr = server_addr();

            log::info!("Connecting to {}", server_socket_addr);
            network_params.net.connect(server_socket_addr);
        }
    }
}

#[derive(SystemParam)]
pub struct PlayerUpdateParams<'a> {
    player_directions: Query<'a, &'static PlayerDirection>,
}

pub fn send_network_updates(
    time: Res<GameTime>,
    mut network_params: NetworkParams,
    current_player_net_id: Res<CurrentPlayerNetId>,
    players: Res<HashMap<PlayerNetId, Player>>,
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
        .add_outgoing_packet(time.frame_number, Utc::now());

    let inputs = match player.role {
        PlayerRole::Runner => {
            let player_entity = player_entity.unwrap(); // is checked above

            let player_direction = player_update_params
                .player_directions
                .get(player_entity)
                .expect("Expected a created spawned player");

            // TODO: this makes the client send more packets than the server actually needs, as lost packets
            //  never get marked as acknowledged, even though we resend updates in future frames. Fix it.
            let first_unacknowledged_frame = network_params
                .connection_state
                .first_unacknowledged_outgoing_packet()
                .expect("Expected at least the new packet for the current frame");
            let mut inputs: Vec<RunnerInput> = Vec::new();
            // TODO: deduplicate updates (the same code is written for server).
            for (frame_number, &direction) in player_direction
                .buffer
                .iter_with_interpolation()
                // TODO: should client always send redundant inputs or only the current ones (unless packet loss is detected)?
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

pub fn send_requests(
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
    let earliest_frame = delta_update
        .players
        .iter()
        .filter_map(|player| player.inputs.iter().map(|input| input.frame_number).min())
        .min()
        .unwrap_or(delta_update.frame_number);

    let diff_with_earliest = time.frame_number.diff_abs(earliest_frame).value();
    let diff_with_latest = time
        .frame_number
        .diff_abs(delta_update.frame_number)
        .value();
    diff_with_earliest < COMPONENT_FRAMEBUFFER_LIMIT / 2
        && diff_with_latest < COMPONENT_FRAMEBUFFER_LIMIT / 2
}

fn process_delta_update_message(
    delta_update: DeltaUpdate,
    connection_state: &ConnectionState,
    current_player_net_id: Option<PlayerNetId>,
    players: &mut HashMap<PlayerNetId, Player>,
    update_params: &mut UpdateParams,
) {
    log::trace!("Processing DeltaUpdate message: {:?}", delta_update);
    let mut rewind_to_simulation_frame = delta_update.frame_number;

    // Calculating how many frames ahead of the server we want to be (implies resizing input buffer for the server).
    let frames_rtt = simulations_per_second() as f32 * connection_state.rtt_millis() / 1000.0;
    let packet_loss_buffer = frames_rtt * connection_state.packet_loss();
    let jitter_buffer = simulations_per_second() as f32 * connection_state.jitter_millis() / 1000.0;
    let frames_to_be_ahead =
        frames_rtt.ceil() + packet_loss_buffer.ceil() + jitter_buffer.ceil() + 1.0;
    let diff = update_params
        .target_frames_ahead
        .frames_count
        .diff_abs(FrameNumber::new(frames_to_be_ahead.ceil() as u16))
        .value();
    let new_target = FrameNumber::new(frames_to_be_ahead as u16);
    if new_target > update_params.target_frames_ahead.frames_count || diff > jitter_buffer as u16 {
        update_params.target_frames_ahead.frames_count = new_target;
    }

    // Adjusting the speed to synchronize with the server clock.
    let new_estimated_server_time =
        delta_update.frame_number + update_params.target_frames_ahead.frames_count;
    if new_estimated_server_time > update_params.estimated_server_time.frame_number {
        update_params.estimated_server_time.frame_number = new_estimated_server_time;
        update_params.estimated_server_time.updated_at = update_params.game_time.frame_number;
    }
    let target_player_frame = update_params.estimated_server_time.frame_number;
    let player_delay = (target_player_frame.value() as i32
        - update_params.game_time.frame_number.value() as i32) as i16;

    // TODO: any better heuristics here?
    let is_above_threshold = player_delay.abs() as f32
        > update_params.target_frames_ahead.frames_count.value() as f32 / 2.0;
    let is_above_jitter_or_positive = player_delay.abs() as f32 > jitter_buffer || player_delay > 0;
    let needs_compensating = is_above_threshold && is_above_jitter_or_positive;

    let is_not_resizing_input_buffer = update_params.target_frames_ahead.frames_count
        == update_params.simulation_time.player_frame - update_params.simulation_time.server_frame;
    if needs_compensating && is_not_resizing_input_buffer {
        log::trace!("player delay: {}, ahread of server: {}, game frame: {}, update frame: {}, estimated server frame: {}, to be ahead: {}, rtt: {}, packet_loss: {}, jitter: {}",
            player_delay,
            update_params.game_time.frame_number.value() as i32 - update_params.estimated_server_time.frame_number.value() as i32,
            update_params.game_time.frame_number.value(),
            delta_update.frame_number.value(),
            update_params.estimated_server_time.frame_number.value(),
            frames_to_be_ahead.ceil() as u16,
            frames_rtt.ceil() as u16,
            packet_loss_buffer.ceil() as u16,
            jitter_buffer.ceil() as u16
        );
        update_params.player_delay.frame_count = player_delay / 2;
    }

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

        let player_frames_ahead = if current_player_net_id == Some(player_state.net_id) {
            update_params.target_frames_ahead.frames_count
        } else {
            FrameNumber::new(0)
        };

        let direction_updates = update_params.player_updates.get_direction_mut(
            player_state.net_id,
            delta_update.frame_number,
            COMPONENT_FRAMEBUFFER_LIMIT,
        );
        let frame_to_update_position = if let Some(earliest_input) = player_state.inputs.first() {
            for (_, update) in direction_updates
                .iter_mut()
                .skip_while(|(frame_number, _)| earliest_input.frame_number < *frame_number)
            {
                let is_unactual_client_input = update.as_ref().map_or(false, |update| {
                    update.is_processed_client_input != Some(false)
                });
                if is_unactual_client_input {
                    *update = None;
                }
            }
            earliest_input.frame_number
        } else {
            delta_update.frame_number
        };
        for input in player_state.inputs {
            direction_updates.insert(
                input.frame_number,
                Some(PlayerDirectionUpdate {
                    direction: input.direction,
                    is_processed_client_input: None,
                }),
            );
        }

        // TODO: detect whether a misprediction indeed happened to avoid redundant rewinding.
        rewind_to_simulation_frame = std::cmp::min(
            rewind_to_simulation_frame,
            frame_to_update_position - player_frames_ahead,
        );

        let position_updates = update_params.player_updates.get_position_mut(
            player_state.net_id,
            delta_update.frame_number,
            COMPONENT_FRAMEBUFFER_LIMIT,
        );
        log::trace!(
            "Updating position for player {} (frame_number: {}): {:?}",
            player_state.net_id.0,
            frame_to_update_position,
            player_state.position
        );
        position_updates.insert(frame_to_update_position, Some(player_state.position));
    }

    // There's no need to rewind if we haven't started the game.
    if let ConnectionStatus::Connected = connection_state.status() {
        log::trace!(
            "Rewinding to frame {} (current server frame: {}, current player frame: {})",
            rewind_to_simulation_frame,
            update_params.simulation_time.server_frame,
            update_params.simulation_time.player_frame
        );
        update_params
            .simulation_time
            .rewind(rewind_to_simulation_frame);
    }
}

fn process_start_game_message(
    start_game: StartGame,
    connection_state: &mut ConnectionState,
    current_player_net_id: &mut CurrentPlayerNetId,
    players: &mut HashMap<PlayerNetId, Player>,
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
    let rtt_frames = FrameNumber::new(
        (simulations_per_second() as f32 * connection_state.rtt_millis() / 1000.0) as u16,
    );
    let half_rtt_frames = FrameNumber::new(
        (simulations_per_second() as f32 * connection_state.rtt_millis() / 1000.0 / 2.0) as u16,
    );
    update_params.target_frames_ahead.frames_count = rtt_frames;
    update_params.simulation_time.server_generation = start_game.generation;
    update_params.simulation_time.player_generation = start_game.generation;
    update_params.simulation_time.server_frame = start_game.game_state.frame_number;
    let (player_frame, overflown) = start_game.game_state.frame_number.add(rtt_frames);
    update_params.simulation_time.player_frame = player_frame;
    if overflown {
        update_params.simulation_time.player_generation += 1;
    }

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
    players: &mut HashMap<PlayerNetId, Player>,
) {
    // Player is spawned when the first DeltaUpdate with it arrives, so we don't do it here.
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
    players: &mut HashMap<PlayerNetId, Player>,
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

fn matchmaker_url() -> Option<String> {
    std::option_env!("MUDDLE_MATCHMAKER_URL").map(str::to_owned)
}

fn server_addr() -> SocketAddr {
    let port: u16 = try_parse_from_env!("MUDDLE_SERVER_PORT").unwrap_or(DEFAULT_SERVER_PORT);
    let ip_addr: IpAddr =
        try_parse_from_env!("MUDDLE_SERVER_IP_ADDR").unwrap_or(DEFAULT_SERVER_IP_ADDR);
    SocketAddr::new(ip_addr, port)
}

pub fn server_addr_optional() -> Option<SocketAddr> {
    let port: u16 = try_parse_from_env!("MUDDLE_SERVER_PORT").unwrap_or(DEFAULT_SERVER_PORT);
    try_parse_from_env!("MUDDLE_SERVER_IP_ADDR").map(|ip_addr| SocketAddr::new(ip_addr, port))
}
