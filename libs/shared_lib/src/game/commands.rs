use crate::{game::level_objects::*, net::PlayerNetId};
use crate::net::EntityNetId;

pub struct GameCommands<T> {
    commands: Vec<T>,
}

impl<T> GameCommands<T> {
    pub fn push(&mut self, command: T) {
        self.commands.push(command);
    }

    pub fn drain(&mut self) -> Vec<T> {
        std::mem::take(&mut self.commands)
    }
}

pub struct SpawnPlayer {
    pub net_id: PlayerNetId,
}

pub struct SpawnLevelObject {
    pub net_id: EntityNetId,
    pub desc: SpawnLevelObjectDesc,
}

pub enum SpawnLevelObjectDesc {
    Plane(PlaneDesc),
}
