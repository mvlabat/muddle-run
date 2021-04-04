use crate::{
    framebuffer::FrameNumber,
    game::level::LevelObject,
    messages::{EntityNetId, PlayerNetId},
};
use bevy::math::Vec2;
use serde::{Deserialize, Serialize};

pub struct GameCommands<T> {
    commands: Vec<T>,
}

impl<T> Default for GameCommands<T> {
    fn default() -> Self {
        Self {
            commands: Vec::new(),
        }
    }
}

impl<T> GameCommands<T> {
    pub fn push(&mut self, command: T) {
        self.commands.push(command);
    }

    pub fn drain(&mut self) -> Vec<T> {
        std::mem::take(&mut self.commands)
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct SpawnPlayer {
    pub net_id: PlayerNetId,
    pub start_position: Vec2,
    pub is_player_frame_simulated: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct DespawnPlayer {
    pub net_id: PlayerNetId,
    pub frame_number: FrameNumber,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct SpawnLevelObject {
    pub object: LevelObject,
    pub frame_number: FrameNumber,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct DespawnLevelObject {
    pub net_id: EntityNetId,
    pub frame_number: FrameNumber,
}
