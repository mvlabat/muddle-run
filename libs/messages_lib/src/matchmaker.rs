#[cfg(feature = "schemars")]
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;

pub const PLAYER_CAPACITY: u16 = 5;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum MatchmakerMessage {
    /// Is sent when a client is connected, contains a list of active servers.
    Init { servers: Vec<Server> },
    /// Is sent when a server is either added or modified.
    ServerUpdated(Server),
    /// Is sent when a server is closed, contains a server name.
    ServerRemoved(String),
    /// Is sent when a user sends an invalid token id with a request (contains a request id).
    InvalidJwt(uuid::Uuid),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum InitLevel {
    Existing(i64),
    Create {
        title: String,
        parent_id: Option<i64>,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum MatchmakerRequest {
    CreateServer {
        init_level: InitLevel,
        request_id: uuid::Uuid,
        id_token: Option<String>,
    },
}

impl MatchmakerRequest {
    pub fn request_id(&self) -> uuid::Uuid {
        match self {
            Self::CreateServer { request_id, .. } => *request_id,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Server {
    pub name: String,
    pub state: GameServerState,
    pub addr: SocketAddr,
    pub player_capacity: u16,
    pub player_count: u16,
    // If a request id is empty, it means that a server isn't allocated yet.
    pub request_id: uuid::Uuid,
}

/// The list of all the possible states: https://github.com/googleforgames/agones/blob/7770aa67fa5a19b5fc37386d220ecedf1044c0c3/pkg/apis/agones/v1/gameserver.go#L35-L62.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(JsonSchema))]
pub enum GameServerState {
    PortAllocation,
    Creating,
    Starting,
    Scheduled,
    RequestReady,
    Ready,
    Reserved,
    Allocated,
    Unhealthy,
    Shutdown,
    Error,
}

impl Default for GameServerState {
    fn default() -> Self {
        Self::Scheduled
    }
}
