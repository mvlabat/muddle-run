use crate::JwtAuthClaims;
#[cfg(feature = "bevy_logging")]
use bevy::log;
use headers::Header;
use jwt_compact::{
    alg::{Rsa, RsaPublicKey},
    jwk::JsonWebKey,
    AlgorithmExt, ParseError, Token, UntrustedToken, ValidationError,
};
use reqwest::Url;
use serde::{Deserialize, Serialize};
use std::{convert::TryFrom, time::Duration};

pub const DEFAULT_JWK_CACHE_TTL: u64 = 15;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JwkSet<'a> {
    pub keys: Vec<Jwk<'a>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Jwk<'a> {
    #[serde(flatten)]
    base: JsonWebKey<'a>,
    #[serde(rename = "kid")]
    key_id: String,
    #[serde(rename = "use")]
    key_use: KeyUse,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum KeyUse {
    #[serde(rename = "sig")]
    Signature,
    #[serde(rename = "enc")]
    Encryption,
}

#[derive(Clone)]
pub struct Jwks {
    keys: std::sync::Arc<tokio::sync::RwLock<Vec<Key>>>,
}

pub struct Key {
    pub certs_url: Url,
    pub kid: String,
    pub key: RsaPublicKey,
}

impl Default for Jwks {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug)]
pub enum InvalidTokenError {
    Malformed(ParseError),
    KeyIdMissing,
    Invalid(ValidationError),
    InvalidAudience,
    UnknownSigner,
}

impl Jwks {
    pub fn new() -> Self {
        Self {
            keys: std::sync::Arc::new(tokio::sync::RwLock::new(Vec::new())),
        }
    }

    pub async fn update(&self, issuer: &Url, new_keys: Vec<(String, RsaPublicKey)>) {
        let mut keys = self.keys.write().await;
        keys.retain(|key| key.certs_url != *issuer);
        keys.extend(new_keys.into_iter().map(|(kid, key)| Key {
            certs_url: issuer.clone(),
            kid,
            key,
        }));
    }

    pub async fn get(&self, kid: &str) -> Option<RsaPublicKey> {
        let keys = self.keys.read().await;
        keys.iter()
            .find(|key| key.kid == kid)
            .map(|Key { ref key, .. }| key.clone())
    }

    pub async fn decode(
        &self,
        token: &str,
        audience: &[&str],
    ) -> Result<Token<JwtAuthClaims>, InvalidTokenError> {
        let token = UntrustedToken::new(token).map_err(InvalidTokenError::Malformed)?;
        let Some(kid) = token.header().key_id.as_ref() else {
            return Err(InvalidTokenError::KeyIdMissing);
        };

        let Some(key) = self.get(kid).await else {
            return Err(InvalidTokenError::UnknownSigner)
        };

        let verified_token: Token<JwtAuthClaims> = Rsa::rs256()
            .validate_integrity(&token, &key)
            .map_err(InvalidTokenError::Invalid)?;
        verified_token
            .claims()
            .validate_expiration(&Default::default())
            .map_err(InvalidTokenError::Invalid)?;

        let is_valid_aud = audience.contains(&verified_token.claims().custom.aud.as_str());
        if !is_valid_aud {
            return Err(InvalidTokenError::InvalidAudience);
        }

        Ok(verified_token)
    }
}

pub async fn poll_jwks(client: reqwest::Client, certs_url: Url, jwks: Jwks) {
    loop {
        let jwk_ttl = match client.get(certs_url.clone()).send().await {
            Ok(response) => {
                let max_age: Result<Duration, anyhow::Error> = try {
                    let cache_control = response.headers().get_all(reqwest::header::CACHE_CONTROL);
                    let cache_control = headers::CacheControl::decode(&mut cache_control.iter())
                        .map_err(anyhow::Error::msg)?;
                    cache_control
                        .max_age()
                        .ok_or_else(|| anyhow::Error::msg("No max-age directive"))?
                };
                let jwk_ttl = max_age.unwrap_or_else(|err| {
                    log::error!(
                        "Unable to determine cache TTL (will invalidate in {} seconds): {:?}",
                        DEFAULT_JWK_CACHE_TTL,
                        err
                    );
                    Duration::from_secs(DEFAULT_JWK_CACHE_TTL)
                });

                match response.json::<JwkSet>().await {
                    Ok(jwk_set) => {
                        log::info!("Updating a JWK set: {}", certs_url);
                        let keys = jwk_set
                            .keys
                            .into_iter()
                            .map(|key| {
                                (
                                    key.key_id,
                                    RsaPublicKey::try_from(&key.base).expect("Expected RSA keys"),
                                )
                            })
                            .collect();
                        jwks.update(&certs_url, keys).await;
                    }
                    Err(err) => {
                        log::error!("Failed to parse JWK set: {:?}", err);
                    }
                }

                jwk_ttl
            }
            Err(err) => {
                log::error!(
                    "Failed to fetch JwtSet (will re-try in {} seconds): {:?}",
                    DEFAULT_JWK_CACHE_TTL,
                    err
                );
                Duration::from_secs(DEFAULT_JWK_CACHE_TTL)
            }
        };

        tokio::time::sleep(jwk_ttl).await
    }
}
