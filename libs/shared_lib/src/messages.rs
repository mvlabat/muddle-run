use crate::{
    framebuffer::FrameNumber,
    game::{
        commands,
        commands::UpdateLevelObject,
        level::{LevelObject, LevelObjectDesc},
    },
    net::{MessageId, SessionId},
    player::{Player, PlayerRole},
    registry::IncrementId,
};
use bevy::{
    ecs::{component::Component, system::Resource},
    math::Vec2,
    prelude::{Deref, DerefMut},
};
use serde::{Deserialize, Serialize};

#[derive(Resource)]
pub struct DeferredMessagesQueue<T: Serialize> {
    messages: Vec<T>,
}

impl<T: Serialize> Default for DeferredMessagesQueue<T> {
    fn default() -> Self {
        Self {
            messages: Vec::new(),
        }
    }
}

impl<T: Serialize> DeferredMessagesQueue<T> {
    pub fn push(&mut self, message: T) {
        self.messages.push(message);
    }

    pub fn drain(&mut self) -> Vec<T> {
        std::mem::take(&mut self.messages)
    }
}

// TODO: refactor to be a part of entity registry, implement reclaiming ids of
// removed entities.
#[derive(Component, Serialize, Deserialize, Default, Debug, Copy, Clone, Eq, PartialEq, Hash)]
pub struct EntityNetId(pub u16);

#[derive(Resource, Deref, DerefMut, Default)]
pub struct EntityNetIdCounter(pub EntityNetId);

impl IncrementId for EntityNetId {
    fn increment(&mut self) -> Self {
        let old = *self;
        self.0 += 1;
        old
    }
}

// TODO: refactor to be a part of player registry, implement reclaiming ids of
// removed players.
#[derive(Serialize, Deserialize, Default, Debug, Copy, Clone, Eq, PartialEq, Hash)]
pub struct PlayerNetId(pub u16);

#[derive(Resource, Deref, DerefMut, Default)]
pub struct PlayerNetIdCounter(pub PlayerNetId);

impl IncrementId for PlayerNetId {
    fn increment(&mut self) -> Self {
        let old = *self;
        self.0 += 1;
        old
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
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
    /// A kludge message basically, to let our networking stack to initialize
    /// properly for webrtc.
    Initialize,
    /// Is sent as a response to server's `UnreliableServerMessage::Handshake`.
    Handshake {
        message_id: MessageId,
        id_token: Option<String>,
    },
    SwitchRole(PlayerRole),
    SpawnLevelObject(SpawnLevelObjectRequest),
    UpdateLevelObject(LevelObject),
    DespawnLevelObject(EntityNetId),
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct SpawnLevelObjectRequest {
    pub correlation_id: MessageId,
    pub body: SpawnLevelObjectRequestBody,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum SpawnLevelObjectRequestBody {
    New(LevelObjectDesc),
    Copy(EntityNetId),
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum ReliableServerMessage {
    /// A kludge message basically, to let our networking stack to initialize
    /// properly for webrtc.
    Initialize,
    /// Is sent if a server is still in the loading state when a client joins
    /// (as a response to client's `ReliableClientMessage::Handshake`).
    Loading,
    /// Is sent as a response to client's `ReliableClientMessage::Handshake` or
    /// when the game is started if a client is already joined.
    StartGame(StartGame),
    ConnectedPlayer((PlayerNetId, Player)),
    DisconnectedPlayer(DisconnectedPlayer),
    SpawnLevelObject(SpawnLevelObject),
    UpdateLevelObject(commands::UpdateLevelObject),
    DespawnLevelObject(commands::DespawnLevelObject),
    SwitchRole(SwitchRole),
    RespawnPlayer(RespawnPlayer),
    Disconnect(DisconnectReason),
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum DisconnectReason {
    InvalidJwt,
    InvalidUpdate,
    Timeout,
    Closed,
    Aborted,
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
    pub uuid: String,
    pub nickname: String,
    pub objects: Vec<commands::UpdateLevelObject>,
    pub players: Vec<(PlayerNetId, Player)>,
    pub level_id: Option<i64>,
    pub generation: u64,
    /// Full game state encoded as a DeltaUpdate.
    pub game_state: DeltaUpdate,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct DisconnectedPlayer {
    pub net_id: PlayerNetId,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct DeltaUpdate {
    pub frame_number: FrameNumber,
    /// Frame number is `None` if a player hasn't sent any input yet.
    pub acknowledgments: (Option<FrameNumber>, u64),
    pub players: Vec<PlayerState>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct PlayerState {
    pub net_id: PlayerNetId,
    /// Contains the initial position, so that applying all inputs renders a
    /// player in its actual position on server.
    pub position: Vec2,
    pub direction: Vec2,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct RunnerInput {
    pub frame_number: FrameNumber,
    pub direction: Vec2,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct SpawnLevelObject {
    pub correlation_id: MessageId,
    pub command: UpdateLevelObject,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct SwitchRole {
    pub net_id: PlayerNetId,
    pub role: PlayerRole,
    pub frame_number: FrameNumber,
}

/// This message isn't supposed to trigger the spawn command though. We spawn a
/// player as soon as it appears in a DeltaUpdate message, as usual.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct RespawnPlayer {
    pub net_id: PlayerNetId,
    pub reason: RespawnPlayerReason,
    pub frame_number: FrameNumber,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum RespawnPlayerReason {
    Finish,
    Death,
}
