use crate::net::PlayerConnections;
use bevy::{log, prelude::*};
use mr_shared_lib::{
    framebuffer::{FrameNumber, Framebuffer},
    messages::{DeltaUpdate, PlayerInput, PlayerNetId, PlayerState},
    player::PlayerUpdates,
    GameTime,
};
use std::collections::HashMap;

pub const SERVER_UPDATES_LIMIT: u16 = 32;

pub struct DeferredUpdates<T> {
    updates: HashMap<PlayerNetId, Vec<T>>,
}

impl<T> Default for DeferredUpdates<T> {
    fn default() -> Self {
        Self {
            updates: HashMap::new(),
        }
    }
}

impl<T> DeferredUpdates<T> {
    pub fn push(&mut self, player_net_id: PlayerNetId, update: T) {
        let player_updates = self.updates.entry(player_net_id).or_default();
        player_updates.push(update);
    }

    pub fn drain(&mut self) -> HashMap<PlayerNetId, Vec<T>> {
        std::mem::take(&mut self.updates)
    }
}

#[derive(Default)]
pub struct AcknowledgedInputs {
    /// Stores server frame number for each client update.
    pub inputs: HashMap<PlayerNetId, Framebuffer<Option<FrameNumber>>>,
}

pub fn process_player_input_updates(
    time: Res<GameTime>,
    mut updates: ResMut<PlayerUpdates>,
    mut deferred_updates: ResMut<DeferredUpdates<PlayerInput>>,
    mut acknowledged_inputs: ResMut<AcknowledgedInputs>,
) {
    let deferred_updates = deferred_updates.drain();

    for (player_net_id, player_updates) in deferred_updates {
        let player_update = player_updates
            .first()
            .expect("Expected at least one update for a player hash map entry");
        let inputs = acknowledged_inputs
            .inputs
            .entry(player_net_id)
            .or_insert_with(|| Framebuffer::new(player_update.frame_number, SERVER_UPDATES_LIMIT));
        let updates = updates.get_mut(
            player_net_id,
            player_update.frame_number,
            SERVER_UPDATES_LIMIT,
        );
        for player_update in player_updates {
            if inputs.get(time.game_frame).is_none()
                && inputs.can_insert(player_update.frame_number)
            {
                inputs.insert(player_update.frame_number, Some(time.game_frame));
            }

            if updates.get(time.game_frame).is_none()
                && inputs.can_insert(player_update.frame_number)
            {
                // TODO: input correction (allow 200ms latency max).
                updates.insert(player_update.frame_number, Some(player_update.direction));
            } else {
                log::warn!(
                    "Ignoring player {:?} input for frame {}",
                    player_net_id,
                    player_update.frame_number
                );
            }
        }
    }
}

pub fn prepare_client_updates(
    time: Res<GameTime>,
    player_connections: Res<PlayerConnections>,
    updates: Res<PlayerUpdates>,
    mut deferred_server_updates: ResMut<DeferredUpdates<DeltaUpdate>>,
) {
    // TODO: actual delta updates.
    for (&connection_player_net_id, _) in player_connections.iter() {
        let players = updates
            .updates
            .iter()
            .map(|(&player_net_id, updates_buffer)| {
                let mut inputs = Vec::new();
                if let Some((frame_number, player_input)) =
                    updates_buffer.get_with_interpolation(time.game_frame)
                {
                    inputs.push(PlayerInput {
                        frame_number,
                        direction: *player_input,
                    });
                }
                PlayerState {
                    net_id: player_net_id,
                    position: Default::default(), // TODO: position
                    inputs,
                }
            })
            .collect();
        deferred_server_updates.push(
            connection_player_net_id,
            DeltaUpdate {
                frame_number: time.game_frame,
                players,
                confirmed_actions: Vec::new(),
            },
        );
    }
}
