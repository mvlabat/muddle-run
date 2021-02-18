use crate::{
    framebuffer::FrameNumber,
    game::commands::{DespawnLevelObject, SpawnLevelObject},
    registry::IncrementId,
};
use bevy::math::Vec2;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Default, Debug, Copy, Clone, Eq, PartialEq, Hash)]
pub struct EntityNetId(pub u16);

impl IncrementId for EntityNetId {
    fn increment(&mut self) -> Self {
        let old = *self;
        self.0 += 1;
        old
    }
}

#[derive(Serialize, Deserialize, Default, Debug, Copy, Clone, Eq, PartialEq, Hash)]
pub struct PlayerNetId(pub u16);

impl IncrementId for PlayerNetId {
    fn increment(&mut self) -> Self {
        let old = *self;
        self.0 += 1;
        old
    }
}

#[derive(Serialize, Deserialize, Default, Debug, Copy, Clone, Eq, PartialEq, Hash)]
pub struct ActionNetId(pub u16);

impl IncrementId for ActionNetId {
    fn increment(&mut self) -> Self {
        let old = *self;
        self.0 += 1;
        old
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum ClientMessage {
    PlayerInput(PlayerInput),
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum ReliableServerMessage {
    StartGame(StartGame),
    ConnectedPlayer(ConnectedPlayer),
    DisconnectedPlayer(DisconnectedPlayer),
    SpawnLevelObject(SpawnLevelObject),
    DespawnLevelObject(DespawnLevelObject),
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum UnreliableServerMessage {
    DeltaUpdate(DeltaUpdate),
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct StartGame {
    pub net_id: PlayerNetId,
    pub nickname: String,
    pub objects: Vec<SpawnLevelObject>,
    pub players: Vec<ConnectedPlayer>,
    /// Full game state encoded as a DeltaUpdate.
    pub game_state: DeltaUpdate,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ConnectedPlayer {
    pub net_id: PlayerNetId,
    pub connected_at: FrameNumber,
    pub nickname: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct DisconnectedPlayer {
    pub net_id: PlayerNetId,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct DeltaUpdate {
    pub players: Vec<PlayerState>,
    pub confirmed_actions: Vec<ConfirmedAction>,
    pub frame_number: FrameNumber,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct PlayerState {
    pub net_id: PlayerNetId,
    pub position: Vec2,
    pub inputs: Vec<PlayerInput>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct PlayerInput {
    pub frame_number: FrameNumber,
    pub direction: Vec2,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ConfirmedAction {
    /// ID of an action from a user's request.
    pub id: ActionNetId,
    /// Indicates which frame will contain the action.
    /// If the value is set to `None`, the action was discarded by the server.
    pub confirmed_frame: Option<FrameNumber>,
}
