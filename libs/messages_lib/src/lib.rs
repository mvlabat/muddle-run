mod matchmaker;
mod persistence;

pub use matchmaker::*;
pub use persistence::*;

use serde::{de::DeserializeOwned, Deserialize, Deserializer, Serialize, Serializer};
use serde_with::rust::display_fromstr::deserialize as deserialize_fromstr;

// See: https://docs.rs/serde_qs/0.9.1/serde_qs/index.html#flatten-workaround
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct PaginationParams {
    #[serde(deserialize_with = "deserialize_fromstr")]
    pub offset: i64,
    #[serde(deserialize_with = "deserialize_fromstr")]
    pub limit: i64,
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

pub fn serialize_binary<T: Serialize>(value: &T) -> bincode::Result<Vec<u8>> {
    bincode::serialize(value)
}

pub fn deserialize_binary<'a, T: Deserialize<'a>>(bytes: &'a [u8]) -> bincode::Result<T> {
    bincode::deserialize(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serialize_matchmaker_message() {
        let messages = vec![
            MatchmakerMessage::Init {
                servers: vec![Server {
                    name: "test".to_owned(),
                    state: Default::default(),
                    addr: "127.0.0.1:0".parse().unwrap(),
                    player_capacity: 0,
                    player_count: 0,
                    request_id: Default::default(),
                }],
            },
            MatchmakerMessage::ServerUpdated(Server {
                name: "test".to_owned(),
                state: Default::default(),
                addr: "127.0.0.1:0".parse().unwrap(),
                player_capacity: 0,
                player_count: 0,
                request_id: Default::default(),
            }),
            MatchmakerMessage::ServerRemoved("test".to_owned()),
            MatchmakerMessage::InvalidJwt(Default::default()),
        ];

        for message in messages {
            let serialized = serialize_binary(&message).unwrap();
            let serialized_hex = hex::encode(&serialized);
            let value: MatchmakerMessage = deserialize_binary(&serialized).unwrap_or_else(|err| {
                panic!("Failed to deserialize {message:?} (binary: {serialized_hex}): {err:?}");
            });
            match value {
                MatchmakerMessage::Init { .. }
                | MatchmakerMessage::ServerUpdated(_)
                | MatchmakerMessage::ServerRemoved(_)
                | MatchmakerMessage::InvalidJwt(_) => {}
            }
            assert_eq!(message, value);
        }
    }
}
