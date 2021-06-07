use crate::net::PlayerConnections;
use bevy::{
    ecs::system::{Res, ResMut},
    log,
};
use mr_shared_lib::{
    framebuffer::FrameNumber,
    game::{
        commands::{
            DeferredPlayerQueues, DeferredQueue, DespawnLevelObject, SpawnLevelObject,
            SwitchPlayerRole,
        },
        level::{LevelObject, LevelState},
    },
    messages::{self, DeferredMessagesQueue, EntityNetId, PlayerNetId, RunnerInput},
    net::ConnectionState,
    player::{Player, PlayerDirectionUpdate, PlayerRole, PlayerUpdates},
    registry::IncrementId,
    util::dedup_by_key_unsorted,
    GameTime, SimulationTime, SIMULATIONS_PER_SECOND,
};
use std::collections::HashMap;

pub const SERVER_UPDATES_LIMIT: u16 = 64;
pub const MAX_LAG_COMPENSATION_MILLIS: u16 = 200;

pub fn process_player_input_updates(
    time: Res<GameTime>,
    player_connections: Res<PlayerConnections>,
    connection_states: Res<HashMap<u32, ConnectionState>>,
    mut simulation_time: ResMut<SimulationTime>,
    mut updates: ResMut<PlayerUpdates>,
    mut deferred_updates: ResMut<DeferredPlayerQueues<RunnerInput>>,
) {
    let lag_compensated_frames =
        (MAX_LAG_COMPENSATION_MILLIS as f32 / (1000.0 / SIMULATIONS_PER_SECOND as f32)) as u16;
    let min_frame_number = time.frame_number - FrameNumber::new(lag_compensated_frames);

    let deferred_updates = deferred_updates.drain();
    for (player_net_id, mut player_updates) in deferred_updates {
        let player_connection = player_connections.get_value(player_net_id).unwrap();
        let player_connection_state = connection_states.get(&player_connection).unwrap();
        let player_frame_number = player_connection_state
            .incoming_acknowledgments()
            .0
            // A player has just connected, and it's got only the initial empty update, so it's fine.
            .unwrap_or(time.frame_number);

        let player_update = player_updates
            .first()
            .expect("Expected at least one update for a player hash map entry");
        let updates = updates.get_direction_mut(
            player_net_id,
            player_update.frame_number,
            SERVER_UPDATES_LIMIT,
        );

        // A client might be able to send several messages with the same unacknowledged updates
        // between runs of this system.
        dedup_by_key_unsorted(&mut player_updates, |update| update.frame_number);
        // We want to sort after deduping, to prevent users from re-ordering inputs.
        player_updates.sort_by_key(|update| update.frame_number);

        let mut updates_iter = player_updates.iter().peekable();
        while let Some(player_update) = updates_iter.next() {
            let next_player_update = updates_iter.peek();
            log::trace!(
                "Player ({}) update for frame {}",
                player_net_id.0,
                player_update.frame_number.value()
            );

            let duplicate_updates_from =
                std::cmp::max(player_update.frame_number, min_frame_number);
            let duplicate_updates_to =
                next_player_update.map_or(player_frame_number, |update| update.frame_number);

            let update_to_insert = Some(PlayerDirectionUpdate {
                direction: player_update.direction,
                is_processed_client_input: None,
            });

            // We fill the buffer of player direction commands with the updates that come from
            // clients. We populate each frame until a command changes or we've reached the last
            // acknowledged client's frame (`PlayerUpdate::frame_number`).
            for frame_number in duplicate_updates_from..duplicate_updates_to {
                let existing_update = updates.get(frame_number);
                // We don't want to allow re-writing updates.
                if existing_update.is_none() && updates.can_insert(frame_number) {
                    simulation_time.rewind(frame_number);
                    updates.insert(
                        frame_number,
                        Some(PlayerDirectionUpdate {
                            direction: player_update.direction,
                            is_processed_client_input: None,
                        }),
                    );
                } else if existing_update != Some(&update_to_insert) {
                    // TODO: is just discarding old updates good enough?
                    log::warn!(
                        "Ignoring player {:?} input for frame {} which differs from the existing one (current: {})",
                        player_net_id,
                        frame_number,
                        time.frame_number
                    );
                }
            }
        }
    }
}

pub fn process_switch_role_requests(
    time: Res<GameTime>,
    mut switch_role_requests: ResMut<DeferredPlayerQueues<PlayerRole>>,
    mut switch_role_commands: ResMut<DeferredQueue<SwitchPlayerRole>>,
) {
    for (player_net_id, player_role_requests) in switch_role_requests.drain().into_iter() {
        for player_role in player_role_requests.into_iter() {
            switch_role_commands.push(SwitchPlayerRole {
                net_id: player_net_id,
                role: player_role,
                frame_number: time.frame_number,
                is_player_frame_simulated: false,
            });
        }
    }
}

pub fn process_spawn_level_object_requests(
    time: Res<GameTime>,
    players: Res<HashMap<PlayerNetId, Player>>,
    level_state: Res<LevelState>,
    mut spawn_level_object_requests: ResMut<DeferredPlayerQueues<messages::SpawnLevelObject>>,
    mut entity_net_id_counter: ResMut<EntityNetId>,
    mut update_level_object_commands: ResMut<DeferredQueue<SpawnLevelObject>>,
    mut update_level_object_messages: ResMut<DeferredMessagesQueue<SpawnLevelObject>>,
) {
    'player_requests: for (player_net_id, spawn_level_object_requests) in
        spawn_level_object_requests.drain()
    {
        match players.get(&player_net_id) {
            Some(Player {
                role: PlayerRole::Builder,
                ..
            }) => {}
            Some(_) => {
                log::warn!(
                    "Ignoring Player ({}) spawn requests: player is not a builder",
                    player_net_id.0
                );
                continue 'player_requests;
            }
            None => {
                log::error!(
                    "Ignoring Player ({}) spawn requests: player is not found",
                    player_net_id.0
                );
                continue 'player_requests;
            }
        }

        for spawn_level_object_request in spawn_level_object_requests {
            let desc = match spawn_level_object_request {
                messages::SpawnLevelObject::New(desc) => desc,
                messages::SpawnLevelObject::Copy(entity_net_id) => {
                    if let Some(object) = level_state.objects.get(&entity_net_id) {
                        object.desc.clone()
                    } else {
                        log::warn!(
                            "Ignoring Player ({}) spawn request: copied level object ({}) doesn't exist",
                            player_net_id.0,
                            entity_net_id.0
                        );
                        continue;
                    }
                }
            };
            let spawn_level_object = SpawnLevelObject {
                object: LevelObject {
                    net_id: entity_net_id_counter.increment(),
                    desc,
                },
                frame_number: time.frame_number,
            };
            update_level_object_commands.push(spawn_level_object.clone());
            update_level_object_messages.push(spawn_level_object);
        }
    }
}

pub fn process_update_level_object_requests(
    time: Res<GameTime>,
    players: Res<HashMap<PlayerNetId, Player>>,
    level_state: Res<LevelState>,
    mut update_level_object_requests: ResMut<DeferredPlayerQueues<LevelObject>>,
    mut spawn_level_object_commands: ResMut<DeferredQueue<SpawnLevelObject>>,
    mut update_level_object_messages: ResMut<DeferredMessagesQueue<SpawnLevelObject>>,
) {
    'player_requests: for (player_net_id, update_level_object_requests) in
        update_level_object_requests.drain()
    {
        match players.get(&player_net_id) {
            Some(Player {
                role: PlayerRole::Builder,
                ..
            }) => {}
            Some(_) => {
                log::warn!(
                    "Ignoring Player ({}) update requests: player is not a builder",
                    player_net_id.0
                );
                continue 'player_requests;
            }
            None => {
                log::error!(
                    "Ignoring Player ({}) update requests: player is not found",
                    player_net_id.0
                );
                continue 'player_requests;
            }
        }

        for update_level_object_request in update_level_object_requests {
            if !level_state
                .objects
                .contains_key(&update_level_object_request.net_id)
            {
                log::warn!(
                    "Ignoring Player ({}) update request: updated level object ({}) doesn't exist",
                    player_net_id.0,
                    update_level_object_request.net_id.0
                );
                continue;
            }
            let spawn_level_object = SpawnLevelObject {
                object: update_level_object_request,
                frame_number: time.frame_number,
            };
            spawn_level_object_commands.push(spawn_level_object.clone());
            update_level_object_messages.push(spawn_level_object);
        }
    }
}

pub fn process_despawn_level_object_requests(
    time: Res<GameTime>,
    players: Res<HashMap<PlayerNetId, Player>>,
    level_state: Res<LevelState>,
    mut despawn_level_object_requests: ResMut<DeferredPlayerQueues<EntityNetId>>,
    mut despawn_level_object_commands: ResMut<DeferredQueue<DespawnLevelObject>>,
    mut despawn_level_object_messages: ResMut<DeferredMessagesQueue<DespawnLevelObject>>,
) {
    'player_requests: for (player_net_id, despawn_level_object_requests) in
        despawn_level_object_requests.drain()
    {
        match players.get(&player_net_id) {
            Some(Player {
                role: PlayerRole::Builder,
                ..
            }) => {}
            Some(_) => {
                log::warn!(
                    "Ignoring Player ({}) despawn requests: player is not a builder",
                    player_net_id.0
                );
                continue 'player_requests;
            }
            None => {
                log::error!(
                    "Ignoring Player ({}) despawn requests: player is not found",
                    player_net_id.0
                );
                continue 'player_requests;
            }
        }

        for despawned_level_object_net_id in despawn_level_object_requests {
            if !level_state
                .objects
                .contains_key(&despawned_level_object_net_id)
            {
                log::warn!(
                    "Ignoring Player ({}) despawn request: updated level object ({}) doesn't exist",
                    player_net_id.0,
                    despawned_level_object_net_id.0
                );
                continue;
            }
            let despawn_level_object = DespawnLevelObject {
                net_id: despawned_level_object_net_id,
                frame_number: time.frame_number,
            };
            despawn_level_object_commands.push(despawn_level_object.clone());
            despawn_level_object_messages.push(despawn_level_object);
        }
    }
}
