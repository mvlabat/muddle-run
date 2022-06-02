use crate::{
    framebuffer::FrameNumber,
    game::level::LevelObject,
    messages::{EntityNetId, PlayerNetId},
    player::PlayerRole,
};
use bevy::{math::Vec2, utils::HashMap};
use serde::{Deserialize, Serialize};

pub struct DeferredQueue<T> {
    commands: Vec<T>,
}

impl<T> Default for DeferredQueue<T> {
    fn default() -> Self {
        Self {
            commands: Vec::new(),
        }
    }
}

impl<T> DeferredQueue<T> {
    pub fn push(&mut self, command: T) {
        self.commands.push(command);
    }

    pub fn drain(&mut self) -> Vec<T> {
        std::mem::take(&mut self.commands)
    }
}

// NOTE: after adding a new command, remember to clean them up in the
// `restart_game` system.

pub struct RestartGame;

pub struct SwitchPlayerRole {
    pub net_id: PlayerNetId,
    pub role: PlayerRole,
    pub frame_number: FrameNumber,
    pub is_player_frame_simulated: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct SpawnPlayer {
    pub net_id: PlayerNetId,
    pub start_position: Vec2,
    pub is_player_frame_simulated: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct DespawnPlayer {
    pub net_id: PlayerNetId,
    pub frame_number: FrameNumber,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct UpdateLevelObject {
    pub object: LevelObject,
    pub frame_number: FrameNumber,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct DespawnLevelObject {
    pub net_id: EntityNetId,
    pub frame_number: FrameNumber,
}

pub struct DeferredPlayerQueues<T> {
    updates: HashMap<PlayerNetId, Vec<T>>,
}

impl<T> Default for DeferredPlayerQueues<T> {
    fn default() -> Self {
        Self {
            updates: HashMap::default(),
        }
    }
}

impl<T> DeferredPlayerQueues<T> {
    pub fn push(&mut self, player_net_id: PlayerNetId, update: T) {
        let player_updates = self.updates.entry(player_net_id).or_default();
        player_updates.push(update);
    }

    pub fn drain(&mut self) -> HashMap<PlayerNetId, Vec<T>> {
        std::mem::take(&mut self.updates)
    }
}
