use crate::{CurrentPlayerNetId, EstimatedServerTime, InitialRtt, PlayerDelay, TargetFramesAhead};
use bevy::{ecs::system::SystemParam, log, prelude::*};
use bevy_networking_turbulence::{NetworkEvent, NetworkResource};
use mr_shared_lib::{
    framebuffer::FrameNumber,
    game::{
        commands::{
            DespawnLevelObject, DespawnPlayer, GameCommands, SpawnLevelObject, SpawnPlayer,
        },
        components::PlayerDirection,
    },
    messages::{
        ConnectedPlayer, DeltaUpdate, DisconnectedPlayer, Message, PlayerInput, PlayerNetId,
        PlayerUpdate, ReliableClientMessage, ReliableServerMessage, StartGame,
        UnreliableClientMessage, UnreliableServerMessage,
    },
    net::{AcknowledgeError, ConnectionState, ConnectionStatus, SessionId},
    player::{Player, PlayerDirectionUpdate, PlayerUpdates},
    registry::EntityRegistry,
    GameTime, SimulationTime, COMPONENT_FRAMEBUFFER_LIMIT, SIMULATIONS_PER_SECOND,
};
use std::{
    collections::HashMap,
    net::{IpAddr, SocketAddr},
    time::Instant,
};

const DEFAULT_SERVER_PORT: u16 = 3455;

pub fn initiate_connection(mut net: ResMut<NetworkResource>) {
    if net.connections.is_empty() {
        let server_socket_addr = server_addr().expect("cannot find current ip address");

        log::info!("Starting the client");
        net.connect(server_socket_addr);
    }
}

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
    spawn_level_object_commands: ResMut<'a, GameCommands<SpawnLevelObject>>,
    despawn_level_object_commands: ResMut<'a, GameCommands<DespawnLevelObject>>,
    spawn_player_commands: ResMut<'a, GameCommands<SpawnPlayer>>,
    despawn_player_commands: ResMut<'a, GameCommands<DespawnPlayer>>,
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
                log::info!("Connected: {}", handle);
                if let Err(err) = network_params.net.send_message(
                    *handle,
                    Message {
                        // The server is expected to accept any session id for this message.
                        session_id: SessionId::new(0),
                        message: ReliableClientMessage::Handshake,
                    },
                ) {
                    log::error!("Failed to send a Handshake message: {:?}", err);
                }
                update_params.initial_rtt.sent_at = Some(Instant::now());
                network_params
                    .connection_state
                    .set_status(ConnectionStatus::Handshaking);
            }
            NetworkEvent::Disconnected(handle) => {
                log::info!("Disconnected: {}", handle);
            }
            _ => {}
        }
    }

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

            if session_id != network_params.connection_state.session_id {
                log::warn!(
                    "Ignoring a server message: sent session id {} doesn't match {}",
                    session_id,
                    network_params.connection_state.session_id
                );
                continue;
            }

            match message {
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
                            .apply_outcoming_acknowledgements(ack_frame_number, ack_bit_set)
                        {
                            Err(AcknowledgeError::OutOfRange) => {
                                log::warn!(
                                    "Can't apply acknowledgments for frame {} (current frame: {})",
                                    ack_frame_number,
                                    update_params.game_time.frame_number
                                );
                                skip_update = true;
                            }
                            Err(err) => panic!(
                                "{:?} todo disconnect (frame number: {}, ack frame: {})",
                                err, update_params.game_time.frame_number, ack_frame_number
                            ),
                            _ => {}
                        }
                    }
                    skip_update = skip_update || current_player_net_id.0.is_none();
                    if !skip_update {
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
                ReliableServerMessage::StartGame(start_game) => {
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
                    process_disconnected_player_message(disconnected_player);
                }
                ReliableServerMessage::SpawnLevelObject(spawn_level_object) => {
                    update_params
                        .simulation_time
                        .rewind(spawn_level_object.frame_number);
                    update_params
                        .spawn_level_object_commands
                        .push(spawn_level_object);
                }
                ReliableServerMessage::DespawnLevelObject(despawn_level_object) => {
                    update_params
                        .simulation_time
                        .rewind(despawn_level_object.frame_number);
                    update_params
                        .despawn_level_object_commands
                        .push(despawn_level_object);
                }
                ReliableServerMessage::Disconnect => {
                    todo!("disconnect");
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
}

#[derive(SystemParam)]
pub struct PlayerUpdateParams<'a> {
    player_directions: Query<'a, &'static PlayerDirection>,
}

pub fn send_network_updates(
    time: Res<GameTime>,
    mut network_params: NetworkParams,
    current_player_net_id: Res<CurrentPlayerNetId>,
    player_registry: Res<EntityRegistry<PlayerNetId>>,
    player_update_params: PlayerUpdateParams,
) {
    log::trace!("Broadcast updates for frame {}", time.frame_number);
    let (connection_handle, address) = match network_params.net.connections.iter_mut().next() {
        Some((&handle, connection)) => (handle, connection.remote_address()),
        None => return,
    };
    let player_entity = match current_player_net_id.0 {
        Some(net_id) => player_registry
            .get_entity(net_id)
            .expect("Expected a registered entity for the player"),
        None => return,
    };

    let player_direction = player_update_params
        .player_directions
        .get(player_entity)
        .expect("Expected a created spawned player");

    network_params
        .connection_state
        // Clients don't resend updates, so we can forget about unacknowledged packets.
        .add_outcoming_packet(time.frame_number, Instant::now());
    let first_unacknowledged_frame = network_params
        .connection_state
        .first_unacknowledged_outcoming_packet()
        .expect("Expected at least the new packet for the current frame");
    let mut inputs: Vec<PlayerInput> = Vec::new();
    // TODO: deduplicate updates (the same code is written for server).
    for (frame_number, &direction) in player_direction
        .buffer
        .iter_with_interpolation()
        // TODO: should client always sent redundant inputs or only the current ones (unless packet loss is detected)?
        .skip_while(|(frame_number, _)| *frame_number < first_unacknowledged_frame)
    {
        if Some(direction) != inputs.last().map(|i| i.direction) {
            inputs.push(PlayerInput {
                frame_number,
                direction,
            });
        }
    }

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

fn process_delta_update_message(
    delta_update: DeltaUpdate,
    connection_state: &ConnectionState,
    current_player_net_id: Option<PlayerNetId>,
    players: &mut HashMap<PlayerNetId, Player>,
    update_params: &mut UpdateParams,
) {
    let mut rewind_to_simulation_frame = delta_update.frame_number;

    // Calculating how many frames ahead of the server we want to be (implies resizing input buffer for the server).
    let frames_rtt = SIMULATIONS_PER_SECOND as f32 * connection_state.rtt_millis() / 1000.0;
    let packet_loss_buffer = frames_rtt * connection_state.packet_loss();
    let jitter_buffer = SIMULATIONS_PER_SECOND as f32 * connection_state.jitter_millis() / 1000.0;
    let frames_to_be_ahead =
        frames_rtt.ceil() + packet_loss_buffer.ceil() + jitter_buffer.ceil() + 1.0;
    let diff = (update_params.target_frames_ahead.frames_count.value() as i32
        - FrameNumber::new(frames_to_be_ahead.ceil() as u16).value() as i32)
        .abs() as u16;
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
        log::debug!("player delay: {}, ahread of server: {}, game frame: {}, update frame: {}, estimated server frame: {}, to be ahead: {}, rtt: {}, packet_loss: {}, jitter: {}",
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
        .keys()
        .copied()
        .filter(|player_net_id| {
            !delta_update
                .players
                .iter()
                .any(|player| player.net_id == *player_net_id)
        })
        .collect();

    for player_net_id in players_to_remove {
        players.remove(&player_net_id);
        update_params.despawn_player_commands.push(DespawnPlayer {
            net_id: player_net_id,
            frame_number: delta_update.frame_number,
        });
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
                is_player_frame_simulated: false,
            });
            players
                .entry(player_state.net_id)
                .or_insert_with(|| Player {
                    nickname: "?".to_owned(),
                });
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
    update_params.initial_rtt.received_at = Some(Instant::now());
    connection_state
        .set_initial_rtt_millis(update_params.initial_rtt.duration_secs().unwrap() * 1000.0);

    if let Some(start_position) = player_start_position(start_game.net_id, &start_game.game_state) {
        current_player_net_id.0 = Some(start_game.net_id);
        players.insert(
            start_game.net_id,
            Player {
                nickname: start_game.nickname,
            },
        );
        update_params.game_time.generation += 1;
        let rtt_frames = FrameNumber::new(
            (SIMULATIONS_PER_SECOND as f32 * connection_state.rtt_millis() / 1000.0) as u16,
        );
        let half_rtt_frames = FrameNumber::new(
            (SIMULATIONS_PER_SECOND as f32 * connection_state.rtt_millis() / 1000.0 / 2.0) as u16,
        );
        update_params.target_frames_ahead.frames_count = rtt_frames;
        update_params.simulation_time.server_frame = start_game.game_state.frame_number;
        update_params.simulation_time.player_frame =
            start_game.game_state.frame_number + rtt_frames;
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
                Player {
                    nickname: player.nickname,
                },
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
    players.insert(
        connected_player.net_id,
        Player {
            nickname: connected_player.nickname,
        },
    );
}

fn process_disconnected_player_message(disconnected_player: DisconnectedPlayer) {
    // We actually remove players if there's no mention of them in a DeltaUpdate message.
    log::info!("A player ({}) disconnected", disconnected_player.net_id.0);
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
