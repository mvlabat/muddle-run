use serde::{Deserialize, Serialize};
use std::net::SocketAddr;

pub const PLAYER_CAPACITY: u16 = 5;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum MatchmakerMessage {
    /// Is sent when a client is connected, contains a list of active servers.
    Init(Vec<Server>),
    /// Is sent when a server is either added or modified.
    ServerUpdated(Server),
    /// Is sent when a server is closed, contains a server name.
    ServerRemoved(String),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Server {
    pub name: String,
    pub addr: SocketAddr,
    pub player_capacity: u16,
    pub player_count: u16,
}

#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
pub struct JwtAuthClaims {
    pub iss: String,
    pub sub: String,
    pub email: Option<String>,
    pub exp: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RegisterRequest {
    pub jwt: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RegisterResponse {
    #[serde(flatten)]
    pub user: RegisteredUser,
    pub is_new: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RegisteredUser {
    pub id: i64,
    pub email: Option<String>,
    pub username: Option<String>,
    pub created_at: chrono::NaiveDateTime,
    pub updated_at: chrono::NaiveDateTime,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ErrorResponse {
    pub message: String,
}

impl ErrorResponse {
    pub fn new(message: String) -> Self {
        Self { message }
    }
}
