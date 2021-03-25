use bevy::{
    ecs::{Res, ResMut},
    log,
};
use mr_shared_lib::{
    framebuffer::FrameNumber,
    messages::{PlayerInput, PlayerNetId},
    player::{PlayerDirectionUpdate, PlayerUpdates},
    GameTime, SimulationTime, SIMULATIONS_PER_SECOND,
};
use std::collections::HashMap;

pub const SERVER_UPDATES_LIMIT: u16 = 64;
pub const MAX_LAG_COMPENSATION_MSEC: u16 = 200;

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

pub fn process_player_input_updates(
    time: Res<GameTime>,
    mut simulation_time: ResMut<SimulationTime>,
    mut updates: ResMut<PlayerUpdates>,
    mut deferred_updates: ResMut<DeferredUpdates<PlayerInput>>,
) {
    let deferred_updates = deferred_updates.drain();

    for (player_net_id, player_updates) in deferred_updates {
        let player_update = player_updates
            .first()
            .expect("Expected at least one update for a player hash map entry");
        let updates = updates.get_direction_mut(
            player_net_id,
            player_update.frame_number,
            SERVER_UPDATES_LIMIT,
        );
        for player_update in player_updates {
            let lag_compensated_frames = (MAX_LAG_COMPENSATION_MSEC as f32
                / (1000.0 / SIMULATIONS_PER_SECOND as f32))
                as u16;
            let min_frame_number = time.frame_number - FrameNumber::new(lag_compensated_frames);
            let update_frame_number = std::cmp::max(min_frame_number, player_update.frame_number);

            // We don't want to allow re-writing updates.
            if updates.get(update_frame_number).is_none() && updates.can_insert(update_frame_number)
            {
                simulation_time.server_frame =
                    std::cmp::min(simulation_time.server_frame, update_frame_number);
                simulation_time.player_frame = simulation_time.server_frame;
                updates.insert(
                    update_frame_number,
                    Some(PlayerDirectionUpdate {
                        direction: player_update.direction,
                        is_processed_client_input: None,
                    }),
                );
            } else {
                // TODO: is just discarding old updates good enough?
                log::warn!(
                    "Ignoring player {:?} input for frame {} (current: {})",
                    player_net_id,
                    update_frame_number,
                    time.frame_number,
                );
            }
        }
    }
}
