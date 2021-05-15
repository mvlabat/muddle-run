use crate::{
    framebuffer::FrameNumber,
    game::commands::{DespawnLevelObject, SpawnLevelObject},
    net::{MessageId, SessionId},
    player::PlayerRole,
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
pub struct Message<T> {
    pub session_id: SessionId,
    pub message: T,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum UnreliableClientMessage {
    Connect(MessageId),
    PlayerUpdate(PlayerUpdate),
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum ReliableClientMessage {
    /// A kludge message basically, to let our networking stack to initialize properly for webrtc.
    Initialize,
    /// Is sent as a response to server's `UnreliableServerMessage::Handshake`.
    Handshake(MessageId),
    SwitchRole(PlayerRole),
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum ReliableServerMessage {
    /// A kludge message basically, to let our networking stack to initialize properly for webrtc.
    Initialize,
    /// Is sent as a response to client's `ReliableClientMessage::Handshake`.
    StartGame(StartGame),
    ConnectedPlayer(ConnectedPlayer),
    DisconnectedPlayer(DisconnectedPlayer),
    SpawnLevelObject(SpawnLevelObject),
    DespawnLevelObject(DespawnLevelObject),
    SwitchRole(SwitchRole),
    Disconnect,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct PlayerUpdate {
    pub frame_number: FrameNumber,
    pub acknowledgments: (Option<FrameNumber>, u64),
    pub inputs: PlayerInputs,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum PlayerInputs {
    Runner { inputs: Vec<RunnerInput> },
    Builder,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum UnreliableServerMessage {
    /// Is sent as a response to client's `UnreliableClientMessage::Connect`.
    Handshake(MessageId),
    DeltaUpdate(DeltaUpdate),
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct StartGame {
    /// Correlates to a handshake id of a client's request.
    pub handshake_id: MessageId,
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
    pub nickname: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct DisconnectedPlayer {
    pub net_id: PlayerNetId,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct DeltaUpdate {
    pub frame_number: FrameNumber,
    pub acknowledgments: (Option<FrameNumber>, u64),
    pub players: Vec<PlayerState>,
    pub confirmed_actions: Vec<ConfirmedAction>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct PlayerState {
    pub net_id: PlayerNetId,
    /// Contains the initial position, so that applying all inputs renders a player in its actual position on server.
    pub position: Vec2,
    pub inputs: Vec<RunnerInput>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct RunnerInput {
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

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct SwitchRole {
    pub net_id: PlayerNetId,
    pub role: PlayerRole,
    pub frame_number: FrameNumber,
}
