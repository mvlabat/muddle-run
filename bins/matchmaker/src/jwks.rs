use crate::Config;
use futures::FutureExt;
use mr_utils_lib::jwks::Jwks;

pub async fn poll_jwks(config: Config, jwks: Jwks) {
    log::info!("Start JWKs polling");
    let client = reqwest::Client::new();

    let jwks = jwks.clone();
    let mut poll_google_jwks = tokio::spawn(mr_utils_lib::jwks::poll_jwks(
        client.clone(),
        config.google_certs_url.clone(),
        jwks.clone(),
    ))
    .fuse();
    let mut poll_auth0_jwks = tokio::spawn(mr_utils_lib::jwks::poll_jwks(
        client,
        config.auth0_certs_url.clone(),
        jwks,
    ))
    .fuse();
    futures::select! {
        _ = poll_google_jwks => {},
        _ = poll_auth0_jwks => {},
    }
}
