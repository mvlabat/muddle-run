use crate::{
    input::{LevelObjectRequestsQueue, PlayerRequestsQueue},
    CurrentPlayerNetId, EstimatedServerTime, InitialRtt, LevelObjectCorrelations, PlayerDelay,
    TargetFramesAhead,
};
use bevy::{ecs::system::SystemParam, log, prelude::*};
use bevy_networking_turbulence::{NetworkEvent, NetworkResource};
use chrono::Utc;
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
        ConnectedPlayer, DeltaUpdate, DisconnectedPlayer, Message, PlayerInputs, PlayerNetId,
        PlayerUpdate, ReliableClientMessage, ReliableServerMessage, RunnerInput, StartGame,
        UnreliableClientMessage, UnreliableServerMessage,
    },
    net::{
        AcknowledgeError, ConnectionState, ConnectionStatus, MessageId, SessionId,
        CONNECTION_TIMEOUT_MILLIS,
    },
    player::{Player, PlayerDirectionUpdate, PlayerRole, PlayerUpdates},
    registry::EntityRegistry,
    GameTime, SimulationTime, COMPONENT_FRAMEBUFFER_LIMIT, SIMULATIONS_PER_SECOND,
};
use std::{
    collections::HashMap,
    net::{IpAddr, SocketAddr},
};

const DEFAULT_SERVER_PORT: u16 = 3455;
const DEFAULT_SERVER_IP_ADDR: &str = "127.0.0.1";

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

pub fn process_network_events(
    mut network_params: NetworkParams,
    mut network_events: EventReader<NetworkEvent>,
    mut current_player_net_id: ResMut<CurrentPlayerNetId>,
    mut players: ResMut<HashMap<PlayerNetId, Player>>,
    mut update_params: UpdateParams,
) {
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
                    handshake_message_to_send = Some((
                        *handle,
                        Message {
                            session_id: MessageId::new(0),
                            message: ReliableClientMessage::Handshake(message_id),
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
                                network_params
                                    .connection_state
                                    .set_status(ConnectionStatus::Disconnecting);
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
                            network_params
                                .connection_state
                                .set_status(ConnectionStatus::Disconnecting);
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
                        ConnectionStatus::Uninitialized
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
                ReliableServerMessage::ConnectedPlayer(connected_player) => {
                    process_connected_player_message(connected_player, &mut players);
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
                ReliableServerMessage::Disconnect => {
                    network_params
                        .connection_state
                        .set_status(ConnectionStatus::Disconnecting);
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
    mut network_params: NetworkParams,
    mut initial_rtt: ResMut<InitialRtt>,
) {
    // TODO: if a client isn't getting any updates, we may also want to pause the game and wait for
    //  some time for a server to respond.

    let connection_timeout = Utc::now()
        .signed_duration_since(network_params.connection_state.last_message_received_at)
        .to_std()
        .unwrap()
        > std::time::Duration::from_millis(CONNECTION_TIMEOUT_MILLIS);

    if connection_timeout {
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

    if is_falling_behind {
        log::warn!(
            "The client is falling behind, resetting (newest acknowledged frame: {}, current frame: {})",
            newest_acknowledged_incoming_packet.unwrap(),
            time.frame_number
        );
    }

    if connection_timeout
        || is_falling_behind
        || matches!(
            network_params.connection_state.status(),
            ConnectionStatus::Disconnecting | ConnectionStatus::Disconnected
        )
    {
        network_params.net.connections.clear();
        initial_rtt.sent_at = None;
        network_params
            .connection_state
            .set_status(ConnectionStatus::Uninitialized);
    }

    if network_params.net.connections.is_empty() {
        let server_socket_addr = server_addr();

        log::info!("Connecting to {}", server_socket_addr);
        network_params.net.connect(server_socket_addr);
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
                // TODO: should client always sent redundant inputs or only the current ones (unless packet loss is detected)?
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
    let frames_rtt = SIMULATIONS_PER_SECOND as f32 * connection_state.rtt_millis() / 1000.0;
    let packet_loss_buffer = frames_rtt * connection_state.packet_loss();
    let jitter_buffer = SIMULATIONS_PER_SECOND as f32 * connection_state.jitter_millis() / 1000.0;
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

    for player_state in delta_update.players {
        if update_params
            .player_entities
            .get_entity(player_state.net_id)
            .is_none()
        {
            log::info!("First update with the new player {}", player_state.net_id.0);
            update_params.spawn_player_commands.push(SpawnPlayer {
                net_id: player_state.net_id,
                start_position: player_state.position,
                is_player_frame_simulated: current_player_net_id
                    .map_or(false, |current_player_net_id| {
                        current_player_net_id == player_state.net_id
                    }),
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
    let initial_rtt = update_params.initial_rtt.duration_secs().unwrap() * 1000.0;
    log::debug!("Initial rtt: {}", initial_rtt);
    connection_state
        .set_initial_rtt_millis(update_params.initial_rtt.duration_secs().unwrap() * 1000.0);

    if let Some(start_position) = player_start_position(start_game.net_id, &start_game.game_state) {
        current_player_net_id.0 = Some(start_game.net_id);
        players.insert(
            start_game.net_id,
            Player::new_with_nickname(PlayerRole::Runner, start_game.nickname),
        );
        update_params.game_time.session += 1;
        let rtt_frames = FrameNumber::new(
            (SIMULATIONS_PER_SECOND as f32 * connection_state.rtt_millis() / 1000.0) as u16,
        );
        let half_rtt_frames = FrameNumber::new(
            (SIMULATIONS_PER_SECOND as f32 * connection_state.rtt_millis() / 1000.0 / 2.0) as u16,
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

        log::debug!(
            "Spawning the current player ({})",
            current_player_net_id.0.unwrap().0
        );
        update_params.spawn_player_commands.push(SpawnPlayer {
            net_id: start_game.net_id,
            start_position,
            is_player_frame_simulated: true,
        });
    } else {
        log::error!("Player's position isn't found in the game state");
    }

    for player in start_game.players {
        if player.net_id == current_player_net_id.0.unwrap() {
            continue;
        }

        if let Some(start_position) = player_start_position(player.net_id, &start_game.game_state) {
            log::info!("Spawning player {}: {}", player.net_id.0, player.nickname);
            players.insert(
                player.net_id,
                Player::new_with_nickname(PlayerRole::Runner, player.nickname),
            );
            update_params.spawn_player_commands.push(SpawnPlayer {
                net_id: player.net_id,
                start_position,
                is_player_frame_simulated: false,
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
) {
    // Player is spawned when the first DeltaUpdate with it arrives, so we don't do it here.
    log::info!(
        "A new player ({}) connected: {}",
        connected_player.net_id.0,
        connected_player.nickname
    );
    players
        .entry(connected_player.net_id)
        .and_modify(|player| player.nickname = connected_player.nickname.clone())
        .or_insert_with(|| {
            Player::new_with_nickname(PlayerRole::Runner, connected_player.nickname)
        });
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

fn server_addr() -> SocketAddr {
    let server_port = std::env::var("MUDDLE_SERVER_PORT")
        .ok()
        .or_else(|| std::option_env!("MUDDLE_SERVER_PORT").map(str::to_owned))
        .map(|port| port.parse::<u16>().expect("invalid port"))
        .unwrap_or(DEFAULT_SERVER_PORT);

    let env_ip_addr = std::env::var("MUDDLE_SERVER_IP_ADDR")
        .ok()
        .or_else(|| std::option_env!("MUDDLE_SERVER_IP_ADDR").map(str::to_owned));

    let ip_addr = env_ip_addr.as_deref().unwrap_or(DEFAULT_SERVER_IP_ADDR);

    SocketAddr::new(
        ip_addr.parse::<IpAddr>().expect("invalid socket address"),
        server_port,
    )
}
