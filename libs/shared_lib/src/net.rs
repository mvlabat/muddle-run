use crate::{framebuffer::FrameNumber, looped_counter::WrappedCounter};
use bevy::math::Vec2;
use serde::{Deserialize, Serialize};
use crate::registry::IncrementId;

#[derive(Serialize, Deserialize, Default, Debug, Copy, Clone, Eq, PartialEq, Hash)]
pub struct MessageId(pub u16);

impl IncrementId for MessageId {
    fn increment(&mut self) {
        self.0 += 1;
    }
}

#[derive(Serialize, Deserialize, Default, Debug, Copy, Clone, Eq, PartialEq, Hash)]
pub struct EntityNetId(pub u16);

impl IncrementId for EntityNetId {
    fn increment(&mut self) {
        self.0 += 1;
    }
}

#[derive(Serialize, Deserialize, Default, Debug, Copy, Clone, Eq, PartialEq, Hash)]
pub struct PlayerNetId(pub u16);

impl IncrementId for PlayerNetId {
    fn increment(&mut self) {
        self.0 += 1;
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum ClientMessage {
    PlayerInput(PlayerInput),
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum ServerMessage {
    DeltaUpdate(DeltaUpdate),
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct DeltaUpdate {
    pub frame_number: FrameNumber,
    pub players: Vec<PlayerState>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct PlayerState {
    pub player_net_id: PlayerNetId,
    pub position: Vec2,
    pub inputs: Vec<PlayerInput>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct PlayerInput {
    pub frame_number: FrameNumber,
    pub direction: Vec2,
}

pub fn serialize<T: ?Sized + serde::Serialize>(message: &T) -> bincode::Result<Vec<u8>> {
    bincode::serialize(message)
}

pub fn deserialize<'a, T: ?Sized + serde::Deserialize<'a>>(
    message: &'a [u8],
) -> bincode::Result<T> {
    bincode::deserialize(message)
}
