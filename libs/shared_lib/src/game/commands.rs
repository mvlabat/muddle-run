use crate::{
    framebuffer::FrameNumber,
    game::level::LevelObject,
    messages::{EntityNetId, PlayerNetId},
    player::PlayerRole,
    SimulationTime,
};
use bevy::{ecs::system::Resource, math::Vec2, utils::HashMap};
use serde::{Deserialize, Serialize};

pub trait DeferredCommand {
    fn is_player_frame_simulated(&self) -> bool {
        true
    }

    fn frame_number(&self) -> Option<FrameNumber> {
        None
    }
}

#[derive(Resource)]
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

impl<T: DeferredCommand> DeferredQueue<T> {
    pub fn push(&mut self, command: T) {
        self.commands.push(command);
    }

    pub fn drain(&mut self, time: &SimulationTime) -> Vec<T> {
        self.commands
            .drain_filter(|command| {
                let current_frame_number = if command.is_player_frame_simulated() {
                    time.player_frame
                } else {
                    time.server_frame
                };
                command
                    .frame_number()
                    .map_or(true, |frame_number| frame_number <= current_frame_number)
            })
            .collect()
    }
}

// NOTE: after adding a new command, remember to clean them up in the
// `reset_game_world_system` system.

pub struct SwitchPlayerRole {
    pub net_id: PlayerNetId,
    pub role: PlayerRole,
    pub frame_number: FrameNumber,
    pub is_player_frame_simulated: bool,
}

impl DeferredCommand for SwitchPlayerRole {
    fn is_player_frame_simulated(&self) -> bool {
        self.is_player_frame_simulated
    }

    fn frame_number(&self) -> Option<FrameNumber> {
        Some(self.frame_number)
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct SpawnPlayer {
    pub net_id: PlayerNetId,
    pub start_position: Vec2,
    pub is_player_frame_simulated: bool,
}

impl DeferredCommand for SpawnPlayer {
    fn is_player_frame_simulated(&self) -> bool {
        self.is_player_frame_simulated
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct DespawnPlayer {
    pub net_id: PlayerNetId,
    pub frame_number: FrameNumber,
    pub reason: DespawnReason,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub enum DespawnReason {
    DeathOrFinish,
    SwitchRole,
    Disconnect,
    NetworkUpdate,
}

impl DeferredCommand for DespawnPlayer {
    fn frame_number(&self) -> Option<FrameNumber> {
        Some(self.frame_number)
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct UpdateLevelObject {
    pub object: LevelObject,
    pub frame_number: FrameNumber,
}

impl DeferredCommand for UpdateLevelObject {
    fn frame_number(&self) -> Option<FrameNumber> {
        Some(self.frame_number)
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct DespawnLevelObject {
    pub net_id: EntityNetId,
    pub frame_number: FrameNumber,
}

impl DeferredCommand for DespawnLevelObject {
    fn frame_number(&self) -> Option<FrameNumber> {
        Some(self.frame_number)
    }
}

#[derive(Resource)]
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
