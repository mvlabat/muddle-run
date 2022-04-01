mod matchmaker;
mod persistence;

pub use matchmaker::*;
pub use persistence::*;

use serde::{de::DeserializeOwned, Deserialize, Deserializer, Serialize, Serializer};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PaginationParams {
    pub offset: i64,
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
