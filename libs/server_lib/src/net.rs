use crate::player_updates::DeferredUpdates;
use bevy::{ecs::SystemParam, log, prelude::*};
use bevy_networking_turbulence::{NetworkEvent, NetworkResource};
use mr_shared_lib::{
    game::{
        commands::{DespawnPlayer, GameCommands, SpawnLevelObject, SpawnPlayer},
        components::{PlayerDirection, Position},
        level::LevelState,
    },
    messages::{
        ConnectedPlayer, DeltaUpdate, PlayerInput, PlayerNetId, PlayerState, ReliableServerMessage,
        StartGame, UnreliableClientMessage, UnreliableServerMessage,
    },
    net::ConnectionState,
    player::{random_name, Player, PlayerConnectionState},
    registry::{EntityRegistry, Registry},
    GameTime,
};
use std::{
    collections::HashMap,
    net::{IpAddr, SocketAddr},
    time::Instant,
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

#[derive(SystemParam)]
pub struct NetworkParams<'a> {
    net: ResMut<'a, NetworkResource>,
    connection_states: ResMut<'a, HashMap<u32, ConnectionState>>,
    player_connections: ResMut<'a, PlayerConnections>,
}

pub fn process_network_events(
    time: Res<GameTime>,
    mut players: ResMut<HashMap<PlayerNetId, Player>>,
    mut state: ResMut<NetworkReader>,
    network_events: Res<Events<NetworkEvent>>,
    mut network_params: NetworkParams,
    mut update_params: UpdateParams,
) {
    for event in state.network_events.iter(&network_events) {
        match event {
            NetworkEvent::Connected(handle) => {
                log::info!("New connection: {}", handle);
                let player_net_id = network_params.player_connections.register(*handle);
                network_params.connection_states.entry(*handle).or_default();
                let nickname = random_name();
                players.insert(
                    player_net_id,
                    Player {
                        nickname,
                        state: PlayerConnectionState::Connecting,
                    },
                );
                update_params.spawn_player_commands.push(SpawnPlayer {
                    net_id: player_net_id,
                    start_position: Vec2::zero(),
                    is_player_frame_simulated: false,
                });
                // Add an initial update to have something to extrapolate from.
                update_params.deferred_player_updates.push(
                    player_net_id,
                    PlayerInput {
                        frame_number: time.frame_number,
                        direction: Vec2::zero(),
                    },
                );
            }
            NetworkEvent::Disconnected(handle) => {
                log::info!("Disconnected: {}", handle);
                network_params.connection_states.remove(&handle);
                if let Some(player_net_id) =
                    network_params.player_connections.remove_by_value(*handle)
                {
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

    for (handle, connection) in network_params.net.connections.iter_mut() {
        let channels = connection.channels().unwrap();
        while let Some(client_message) = channels.recv::<UnreliableClientMessage>() {
            log::trace!(
                "ClientMessage received on [{}]: {:?}",
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

            match client_message {
                UnreliableClientMessage::PlayerUpdate(update) => {
                    if let Err(err) = connection_state.acknowledge_incoming(update.frame_number) {
                        // TODO: disconnect players.
                        log::error!("Failed to acknowledge an incoming packet (update frame: {}, current frame: {}): {:?}", update.frame_number, time.frame_number, err);
                        break;
                    }
                    if let (Some(frame_number), ack_bit_set) = update.acknowledgments {
                        if let Err(err) = connection_state
                            .apply_outcoming_acknowledgements(frame_number, ack_bit_set)
                        {
                            // TODO: disconnect players.
                            log::error!(
                                "Failed to apply outcoming packet acknowledgments (update frame: {}, current frame: {}): {:?}",
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

        while channels.recv::<ReliableServerMessage>().is_some() {
            log::error!("Unexpected ReliableServerMessage received on [{}]", handle);
        }
        while channels.recv::<UnreliableServerMessage>().is_some() {
            log::error!(
                "Unexpected UnreliableServerMessage received on [{}]",
                handle
            );
        }
    }
}

pub fn send_network_updates(
    mut network_params: NetworkParams,
    time: Res<GameTime>,
    level_state: Res<LevelState>,
    mut players: ResMut<HashMap<PlayerNetId, Player>>,
    player_entities: Query<(Entity, &Position, &PlayerDirection)>,
    players_registry: Res<EntityRegistry<PlayerNetId>>,
) {
    for (&connection_player_net_id, &connection_handle) in network_params.player_connections.iter()
    {
        let connection_state = network_params
            .connection_states
            .get_mut(&connection_handle)
            .expect("Expected a connection state for a connected player");

        // Broadcasting delta updates.
        let player = players.get(&connection_player_net_id).unwrap();
        // Checks that a player hasn't just connected.
        if let PlayerConnectionState::Playing = player.state {
            if let Err(err) = network_params.net.send_message(
                connection_handle,
                UnreliableServerMessage::DeltaUpdate(DeltaUpdate {
                    frame_number: time.frame_number,
                    acknowledgments: connection_state.incoming_acknowledgments(),
                    players: players
                        .iter()
                        .map(|(&player_net_id, _player)| {
                            let entity =
                                players_registry
                                    .get_entity(player_net_id)
                                    .unwrap_or_else(|| {
                                        panic!(
                                            "Player entity ({:?}) is not registered",
                                            player_net_id
                                        )
                                    });
                            create_player_state(
                                player_net_id,
                                &time,
                                connection_state,
                                entity,
                                &player_entities,
                            )
                        })
                        .collect(),
                    confirmed_actions: vec![],
                }),
            ) {
                log::error!("Failed to send a message: {:?}", err);
            }

            if let Err(err) =
                connection_state.add_outcoming_packet(time.frame_number, Instant::now())
            {
                // TODO: disconnect players.
                log::error!("Failed to add an outcoming packet: {:?}", err);
                continue;
            }
        }

        // Broadcasting updates about new connected players.
        for (&connected_player_net_id, player) in players.iter() {
            // Checks that a player hasn't just connected.
            if let PlayerConnectionState::Playing = player.state {
                continue;
            }

            // If a player has just connected, we need to send `StartGame` message to the connected
            // player and broadcast `ConnectedPlayer` to everyone else.

            // TODO: prepare the update in another system.
            let mut players_state = players
                .iter()
                .map(|(&iter_player_net_id, _player)| {
                    let entity = players_registry
                        .get_entity(iter_player_net_id)
                        .unwrap_or_else(|| {
                            panic!("Player entity ({:?}) is not registered", iter_player_net_id)
                        });
                    if connected_player_net_id == iter_player_net_id {
                        PlayerState {
                            net_id: connection_player_net_id,
                            position: Vec2::zero(),
                            inputs: Vec::new(),
                        }
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
                .collect::<Vec<_>>();
            players_state.push(PlayerState {
                net_id: connection_player_net_id,
                position: Vec2::zero(),
                inputs: Vec::new(),
            });

            let result = network_params.net.send_message(
                connection_handle,
                ReliableServerMessage::StartGame(StartGame {
                    net_id: connection_player_net_id,
                    nickname: player.nickname.clone(),
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
                }),
            );
            if let Err(err) = result {
                log::error!("Failed to send a message: {:?}", err);
            }

            broadcast_message_to_others(
                &mut network_params.net,
                &network_params.player_connections,
                connection_handle,
                &ReliableServerMessage::ConnectedPlayer(ConnectedPlayer {
                    net_id: connection_player_net_id,
                    nickname: player.nickname.clone(),
                }),
            );
        }
    }

    for player in players.values_mut() {
        player.state = PlayerConnectionState::Playing;
    }
}

fn create_player_state(
    net_id: PlayerNetId,
    time: &GameTime,
    connection_state: &ConnectionState,
    entity: Entity,
    player_entities: &Query<(Entity, &Position, &PlayerDirection)>,
) -> PlayerState {
    let updates_start_frame = if connection_state.packet_loss() > 0.0 {
        // TODO: avoid doing the same searches when gathering updates for every player?
        connection_state
            .first_unacknowledged_outcoming_packet()
            .unwrap_or(time.frame_number)
    } else {
        time.frame_number
    };

    let (_, position, player_direction) = player_entities.get(entity).unwrap();

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

    PlayerState {
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
