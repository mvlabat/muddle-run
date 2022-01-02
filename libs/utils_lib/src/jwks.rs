use headers::Header;
use jsonwebtoken::jwk::{Jwk, JwkSet};
use reqwest::Url;
use std::time::Duration;

pub const DEFAULT_JWK_CACHE_TTL: u64 = 15;

#[derive(Clone)]
pub struct Jwks {
    // A pair of a certs url and Jwk.
    keys: std::sync::Arc<tokio::sync::RwLock<Vec<(Url, Jwk)>>>,
}

impl Default for Jwks {
    fn default() -> Self {
        Self::new()
    }
}

impl Jwks {
    pub fn new() -> Self {
        Self {
            keys: std::sync::Arc::new(tokio::sync::RwLock::new(Vec::new())),
        }
    }

    pub async fn update(&self, issuer: &Url, new_keys: Vec<Jwk>) {
        let mut keys = self.keys.write().await;
        keys.retain(|(key_issuer, _)| key_issuer != issuer);
        keys.extend(new_keys.into_iter().map(|key| (issuer.clone(), key)));
    }

    pub async fn get(&self, kid: &str) -> Option<Jwk> {
        let keys = self.keys.read().await;
        keys.iter()
            .find(|(_, key)| key.common.key_id.as_deref() == Some(kid))
            .map(|(_, key)| key.clone())
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

                if let Ok(jwk_set) = response.json::<JwkSet>().await {
                    log::info!("Updating a JWK set: {}", certs_url);
                    jwks.update(&certs_url, jwk_set.keys).await;
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
