use crate::{CurrentPlayerNetId, EstimatedServerTime, InitialRtt};
use bevy::{ecs::SystemParam, log, prelude::*};
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
        ConnectedPlayer, DeltaUpdate, DisconnectedPlayer, PlayerInput, PlayerNetId, PlayerUpdate,
        ReliableClientMessage, ReliableServerMessage, StartGame, UnreliableClientMessage,
        UnreliableServerMessage,
    },
    net::{AcknowledgeError, ConnectionState},
    player::{Player, PlayerConnectionState, PlayerDirectionUpdate, PlayerUpdates},
    registry::EntityRegistry,
    GameTime, SimulationTime, TargetFramesAhead, COMPONENT_FRAMEBUFFER_LIMIT,
    SIMULATIONS_PER_SECOND,
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

#[derive(Default)]
pub struct NetworkReader {
    network_events: EventReader<NetworkEvent>,
}

#[derive(SystemParam)]
pub struct UpdateParams<'a> {
    simulation_time: ResMut<'a, SimulationTime>,
    game_time: ResMut<'a, GameTime>,
    estimated_server_time: ResMut<'a, EstimatedServerTime>,
    target_frames_ahead: ResMut<'a, TargetFramesAhead>,
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
                if let Err(err) = network_params
                    .net
                    .send_message(*handle, ReliableClientMessage::Handshake)
                {
                    log::error!("Failed to send a Handshake message: {:?}", err);
                }
                update_params.initial_rtt.sent_at = Some(Instant::now());
            }
            NetworkEvent::Disconnected(handle) => {
                log::info!("Disconnected: {}", handle);
            }
            _ => {}
        }
    }

    for (handle, connection) in network_params.net.connections.iter_mut() {
        let channels = connection.channels().unwrap();

        while let Some(message) = channels.recv::<UnreliableServerMessage>() {
            log::trace!(
                "UnreliableServerMessage received on [{}]: {:?}",
                handle,
                message
            );

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
                    process_disconnected_player_message(
                        disconnected_player,
                        &mut players,
                        &mut update_params,
                    );
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
            }
        }

        while channels.recv::<UnreliableClientMessage>().is_some() {
            log::error!("Unexpected ClientMessage received on [{}]", handle);
        }
    }
}

#[derive(SystemParam)]
pub struct PlayerUpdateParams<'a> {
    player_directions: Query<'a, &'a PlayerDirection>,
}

pub fn send_network_updates(
    time: Res<GameTime>,
    initial_rtt: Res<InitialRtt>,
    mut network_params: NetworkParams,
    current_player_net_id: Res<CurrentPlayerNetId>,
    player_registry: Res<EntityRegistry<PlayerNetId>>,
    player_update_params: PlayerUpdateParams,
) {
    let frames_ahead = match initial_rtt.frames() {
        Some(frames) => frames,
        None => {
            log::trace!("Handshake is not yet complete, skipping");
            return;
        }
    };
    let server_frame = time.frame_number + frames_ahead;
    log::trace!(
        "Broadcast updates for frame {} (sent to server: {})",
        time.frame_number,
        server_frame
    );
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
        .add_outcoming_packet_unchecked(server_frame, Instant::now());
    let first_unacknowledged_frame = network_params
        .connection_state
        .first_unacknowledged_outcoming_packet()
        .expect("Expected at least the new packet for the current frame")
        - frames_ahead;
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

    let result = network_params.net.send_message(
        connection_handle,
        UnreliableClientMessage::PlayerUpdate(PlayerUpdate {
            frame_number: server_frame,
            acknowledgments: network_params.connection_state.incoming_acknowledgments(),
            inputs,
        }),
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

    // Calculating how many frames ahead of the server we want to be.
    let frames_rtt = SIMULATIONS_PER_SECOND as f32 * connection_state.rtt_millis() / 2.0 / 1000.0;
    update_params.estimated_server_time.frame_number = std::cmp::max(
        update_params.estimated_server_time.frame_number,
        delta_update.frame_number + FrameNumber::new(frames_rtt as u16),
    );
    let packet_loss_buffer = frames_rtt * connection_state.packet_loss();
    let jitter_buffer = SIMULATIONS_PER_SECOND as f32 * connection_state.jitter_millis() / 1000.0;
    let frames_to_be_ahead =
        FrameNumber::new((frames_rtt + packet_loss_buffer + jitter_buffer) as u16);
    let diff = (update_params.target_frames_ahead.frames_count.value() as i16
        - frames_to_be_ahead.value() as i16)
        .abs() as u16;
    if diff > jitter_buffer as u16 {
        update_params.target_frames_ahead.frames_count = frames_to_be_ahead;
    }

    for player_state in delta_update.players {
        players.entry(player_state.net_id).or_insert_with(|| {
            log::info!("First update with the new player {}", player_state.net_id.0);
            update_params.spawn_player_commands.push(SpawnPlayer {
                net_id: player_state.net_id,
                start_position: player_state.position,
                is_player_frame_simulated: false,
            });
            Player {
                nickname: "?".to_owned(),
                state: PlayerConnectionState::Connecting,
            }
        });

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
    // TODO: deduce whether we started the game or not from some other state.
    if current_player_net_id.is_some() {
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
                state: PlayerConnectionState::Playing,
            },
        );
        update_params.game_time.generation += 1;
        update_params.target_frames_ahead.frames_count = FrameNumber::new(
            (SIMULATIONS_PER_SECOND as f32 * connection_state.rtt_millis() / 1000.0 / 2.0) as u16,
        );
        update_params.simulation_time.server_frame = start_game.game_state.frame_number;
        update_params.simulation_time.player_frame =
            start_game.game_state.frame_number + update_params.target_frames_ahead.frames_count;
        update_params.game_time.frame_number =
            start_game.game_state.frame_number + update_params.target_frames_ahead.frames_count;
        update_params.spawn_player_commands.push(SpawnPlayer {
            net_id: start_game.net_id,
            start_position,
            is_player_frame_simulated: true,
        });
    } else {
        log::error!("Player's position isn't found in the game state");
    }

    for player in start_game.players {
        if let Some(start_position) = player_start_position(player.net_id, &start_game.game_state) {
            log::info!("Spawning player {}: {}", player.net_id.0, player.nickname);
            players.insert(
                player.net_id,
                Player {
                    nickname: player.nickname,
                    state: PlayerConnectionState::Playing,
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
            state: PlayerConnectionState::Playing,
        },
    );
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
