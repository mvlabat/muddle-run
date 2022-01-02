use serde::{de::DeserializeOwned, Deserialize, Deserializer, Serialize, Serializer};
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
pub struct RegisteredUser {
    pub id: i64,
    pub email: Option<String>,
    pub display_name: Option<String>,
    pub created_at: chrono::NaiveDateTime,
    pub updated_at: chrono::NaiveDateTime,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "code", content = "data", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum RegisterAccountError {
    UserWithEmailAlreadyExists(LinkAccount),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LinkAccountRequest {
    pub existing_account_jwt: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LinkAccount {
    pub user: RegisteredUser,
    pub login_methods: Vec<LinkAccountLoginMethod>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LinkAccountLoginMethod {
    pub issuer: String,
    pub login_hint: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum LinkAccountError {
    ClaimsMismatch,
    AlreadyLinked,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PatchUserRequest {
    pub display_name: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GetUserRequest {
    pub subject: String,
    pub issuer: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ErrorResponse<T: Clone + Serialize + DeserializeOwned = ()> {
    pub message: String,
    #[serde(
        deserialize_with = "ErrorKind::<T>::deserialize_error_kind",
        serialize_with = "ErrorKind::<T>::serialize_error_kind"
    )]
    pub error_kind: ErrorKind<T>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum ErrorKind<T = ()> {
    BadRequest,
    Unauthorized,
    Forbidden,
    NotFound,
    #[serde(skip)]
    RouteSpecific(T),
    #[serde(skip)]
    Unknown,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(untagged)]
enum ErrorWrapper<T> {
    Route(T),
    Common(ErrorKind<T>),
}

impl<T: Clone + Serialize + DeserializeOwned> ErrorKind<T> {
    fn deserialize_error_kind<'de, D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Ok(ErrorWrapper::deserialize(deserializer)
            .map(|w| match w {
                ErrorWrapper::Route(err) => Self::RouteSpecific(err),
                ErrorWrapper::Common(err) => err,
            })
            .unwrap_or(Self::Unknown))
    }

    fn serialize_error_kind<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        ErrorWrapper::serialize(
            &match self.clone() {
                Self::RouteSpecific(err) => ErrorWrapper::Route(err),
                other => ErrorWrapper::Common(other),
            },
            serializer,
        )
    }
}
