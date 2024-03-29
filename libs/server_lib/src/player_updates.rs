use crate::net::{ConnectionStates, PlayerConnections};
use bevy::{
    ecs::system::{Res, ResMut},
    log,
};
use mr_shared_lib::{
    framebuffer::FrameNumber,
    game::{
        commands::{
            DeferredPlayerQueues, DeferredQueue, DespawnLevelObject, SwitchPlayerRole,
            UpdateLevelObject,
        },
        level::{CollisionLogic, LevelObject, LevelState},
    },
    messages::{self, DeferredMessagesQueue, EntityNetId, EntityNetIdCounter, RunnerInput},
    player::{Player, PlayerDirectionUpdate, PlayerRole, PlayerUpdates, Players},
    registry::IncrementId,
    util::dedup_by_key_unsorted,
    GameTime, SimulationTime, LAG_COMPENSATED_FRAMES,
};

pub const SERVER_UPDATES_LIMIT: u16 = 64;

pub fn process_player_input_updates_system(
    time: Res<GameTime>,
    player_connections: Res<PlayerConnections>,
    connection_states: Res<ConnectionStates>,
    mut simulation_time: ResMut<SimulationTime>,
    mut updates: ResMut<PlayerUpdates>,
    mut deferred_updates: ResMut<DeferredPlayerQueues<RunnerInput>>,
) {
    #[cfg(feature = "profiler")]
    puffin::profile_function!();
    let min_frame_number = time.frame_number - LAG_COMPENSATED_FRAMES;

    let deferred_updates = deferred_updates.drain();
    for (player_net_id, mut player_updates) in deferred_updates {
        let player_connection = player_connections.get_value(player_net_id).unwrap();
        let player_connection_state = connection_states.get(&player_connection).unwrap();
        let player_frame_number = player_connection_state
            .incoming_acknowledgments()
            .0
            // A player has just connected, and it's got only the initial empty update, so it's
            // fine.
            .unwrap_or(time.frame_number);

        // A client might be able to send several messages with the same unacknowledged
        // updates between runs of this system.
        dedup_by_key_unsorted(&mut player_updates, |update| update.frame_number);
        // We want to sort after deduping, to prevent users from re-ordering inputs.
        player_updates.sort_by_key(|update| update.frame_number);

        let player_update = player_updates
            .first()
            .expect("Expected at least one update for a player hash map entry")
            .clone();
        let frames_off_lag_compensation_limit = if player_update.frame_number > min_frame_number {
            FrameNumber::new(0)
        } else {
            player_update.frame_number.diff_abs(min_frame_number)
        };
        let updates = updates.get_direction_mut(
            player_net_id,
            player_update.frame_number,
            SERVER_UPDATES_LIMIT,
        );

        let mut updates_iter = player_updates.iter().peekable();
        while let Some(player_update) = updates_iter.next() {
            let next_player_update = updates_iter.peek();

            let duplicate_updates_from =
                player_update.frame_number + frames_off_lag_compensation_limit;
            let duplicate_updates_to = next_player_update
                .map_or_else(|| player_frame_number, |update| update.frame_number)
                + frames_off_lag_compensation_limit;

            let update_to_insert = Some(PlayerDirectionUpdate {
                direction: player_update.direction,
                is_processed_client_input: None,
            });

            log::trace!(
                "Player ({}) update for frame {} (fill from {} up to {})",
                player_net_id.0,
                player_update.frame_number,
                duplicate_updates_from,
                duplicate_updates_to,
            );

            // We fill the buffer of player direction commands with the updates that come
            // from clients. We populate each frame until a command changes or
            // we've reached the last acknowledged client's frame
            // (`PlayerUpdate::frame_number`).
            for frame_number in duplicate_updates_from..=duplicate_updates_to {
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
                    log::trace!(
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

pub fn process_switch_role_requests_system(
    time: Res<GameTime>,
    mut switch_role_requests: ResMut<DeferredPlayerQueues<PlayerRole>>,
    mut switch_role_commands: ResMut<DeferredQueue<SwitchPlayerRole>>,
) {
    #[cfg(feature = "profiler")]
    puffin::profile_function!();
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

pub fn process_spawn_level_object_requests_system(
    time: Res<GameTime>,
    players: Res<Players>,
    level_state: Res<LevelState>,
    mut spawn_level_object_requests: ResMut<
        DeferredPlayerQueues<messages::SpawnLevelObjectRequest>,
    >,
    mut entity_net_id_counter: ResMut<EntityNetIdCounter>,
    mut update_level_object_commands: ResMut<DeferredQueue<UpdateLevelObject>>,
    mut spawn_level_object_messages: ResMut<DeferredMessagesQueue<messages::SpawnLevelObject>>,
) {
    #[cfg(feature = "profiler")]
    puffin::profile_function!();
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
            let desc = match spawn_level_object_request.body {
                messages::SpawnLevelObjectRequestBody::New(desc) => desc,
                messages::SpawnLevelObjectRequestBody::Copy(entity_net_id) => {
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
            let net_id = entity_net_id_counter.increment();
            let spawn_level_object = UpdateLevelObject {
                object: LevelObject {
                    net_id,
                    label: format!("{} {}", desc.label(), net_id.0),
                    desc,
                    route: None,
                    collision_logic: CollisionLogic::None,
                },
                frame_number: time.frame_number,
            };
            update_level_object_commands.push(spawn_level_object.clone());
            spawn_level_object_messages.push(messages::SpawnLevelObject {
                correlation_id: spawn_level_object_request.correlation_id,
                command: spawn_level_object,
            });
        }
    }
}

pub fn process_update_level_object_requests_system(
    time: Res<GameTime>,
    players: Res<Players>,
    level_state: Res<LevelState>,
    mut update_level_object_requests: ResMut<DeferredPlayerQueues<LevelObject>>,
    mut spawn_level_object_commands: ResMut<DeferredQueue<UpdateLevelObject>>,
    mut update_level_object_messages: ResMut<DeferredMessagesQueue<UpdateLevelObject>>,
) {
    #[cfg(feature = "profiler")]
    puffin::profile_function!();
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
            let spawn_level_object = UpdateLevelObject {
                object: update_level_object_request,
                frame_number: time.frame_number,
            };
            spawn_level_object_commands.push(spawn_level_object.clone());
            update_level_object_messages.push(spawn_level_object);
        }
    }
}

pub fn process_despawn_level_object_requests_system(
    time: Res<GameTime>,
    players: Res<Players>,
    level_state: Res<LevelState>,
    mut despawn_level_object_requests: ResMut<DeferredPlayerQueues<EntityNetId>>,
    mut despawn_level_object_commands: ResMut<DeferredQueue<DespawnLevelObject>>,
    mut despawn_level_object_messages: ResMut<DeferredMessagesQueue<DespawnLevelObject>>,
) {
    #[cfg(feature = "profiler")]
    puffin::profile_function!();
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
