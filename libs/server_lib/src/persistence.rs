use crate::TOKIO;
use bevy::{
    ecs::system::{Res, ResMut},
    log,
};
use mr_messages_lib::{GetUserRequest, RegisteredUser};
use mr_shared_lib::net::MessageId;
use mr_utils_lib::jwks::{poll_jwks, Jwks};
use reqwest::{Client, Url};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};

#[derive(Clone)]
pub struct PersistenceConfig {
    pub url: Url,
    pub google_web_client_id: String,
    pub google_desktop_client_id: String,
    pub auth0_client_id: String,
}

#[derive(Debug)]
pub enum PersistenceRequest {
    GetUser { id: MessageId, id_token: String },
}

#[derive(Debug)]
pub enum PersistenceMessage {
    UserInfoResponse {
        id: MessageId,
        user: Option<RegisteredUser>,
    },
}

pub fn init_jwks_polling(config: Option<Res<PersistenceConfig>>, jwks: Res<Jwks>) {
    if config.is_none() {
        return;
    }
    log::info!("Start JWKs polling");
    let client = reqwest::Client::new();

    let google_certs_url: Url = "https://www.googleapis.com/oauth2/v3/certs"
        .parse()
        .unwrap();
    let auth0_certs_url: Url = "https://muddle-run.eu.auth0.com/.well-known/jwks.json"
        .parse()
        .unwrap();

    let jwks = jwks.clone();
    TOKIO.spawn(poll_jwks(client.clone(), google_certs_url, jwks.clone()));
    TOKIO.spawn(poll_jwks(client, auth0_certs_url, jwks));
}

pub fn handle_persistence_requests(
    config: Option<Res<PersistenceConfig>>,
    jwks: Res<Jwks>,
    mut request_rx: ResMut<Option<UnboundedReceiver<PersistenceRequest>>>,
    response_tx: Option<Res<UnboundedSender<PersistenceMessage>>>,
) {
    let Some(config) = config.map(|config| config.clone()) else {
        return;
    };
    let jwks = jwks.clone();
    let mut request_rx = request_rx.take().unwrap();
    let response_tx = response_tx
        .expect("Expected PersistenceMessage sender when persistence config is available")
        .clone();

    let client = reqwest::Client::new();

    TOKIO.spawn(async move {
        loop {
            match request_rx.recv().await {
                Some(PersistenceRequest::GetUser { id, id_token }) => {
                    let jwt = match jwks
                        .decode(
                            &id_token,
                            &[
                                &config.google_web_client_id,
                                &config.google_desktop_client_id,
                                &config.auth0_client_id,
                            ],
                        )
                        .await
                    {
                        Ok(jwt) => jwt,
                        Err(err) => {
                            log::warn!("Invalid JWT: {:?}", err);
                            response_tx
                                .send(PersistenceMessage::UserInfoResponse { id, user: None })
                                .expect("Failed to send a persistence message");
                            continue;
                        }
                    };

                    tokio::spawn(get_user(
                        client.clone(),
                        config.clone(),
                        response_tx.clone(),
                        id,
                        GetUserRequest {
                            subject: jwt.claims().custom.sub.clone(),
                            issuer: jwt.claims().custom.iss.clone(),
                        },
                    ));
                }
                None => {
                    log::error!("Persistence channel closed");
                    return;
                }
            }
        }
    });
}

async fn get_user(
    client: Client,
    config: PersistenceConfig,
    response_tx: UnboundedSender<PersistenceMessage>,
    request_id: MessageId,
    request: GetUserRequest,
) {
    let result = client
        .get(config.url.join("users").unwrap())
        .json(&request)
        .send()
        .await;

    let response = match result {
        Ok(response) => response,
        Err(err) => {
            log::error!("Failed to get a user: {:?}", err);
            response_tx
                .send(PersistenceMessage::UserInfoResponse {
                    id: request_id,
                    user: None,
                })
                .expect("Failed to send a persistence message");
            return;
        }
    };

    let registered_user: RegisteredUser = match response.json().await {
        Ok(user) => user,
        Err(err) => {
            log::error!("Failed to get a user: {:?}", err);
            response_tx
                .send(PersistenceMessage::UserInfoResponse {
                    id: request_id,
                    user: None,
                })
                .expect("Failed to send a persistence message");
            return;
        }
    };

    response_tx
        .send(PersistenceMessage::UserInfoResponse {
            id: request_id,
            user: Some(registered_user),
        })
        .expect("Failed to send a persistence message");
}
