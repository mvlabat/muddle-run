use crate::{game::level::LevelObject, net::PlayerNetId};
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
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct SpawnLevelObject {
    pub object: LevelObject,
}
