use serde::{Deserialize, Serialize};
use std::net::SocketAddr;

pub const PLAYER_CAPACITY: u16 = 10;

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
