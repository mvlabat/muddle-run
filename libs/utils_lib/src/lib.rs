#![feature(try_blocks)]

use serde::{Deserialize, Serialize};

pub mod env;
#[cfg(feature = "jwks")]
pub mod jwks;
#[cfg(feature = "kube_discovery")]
pub mod kube_discovery;

#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
pub struct JwtAuthClaims {
    pub iss: String,
    pub sub: String,
    pub email: Option<String>,
    pub aud: String,
}
