use bevy::log;
use core::slice::SlicePattern;
use mr_messages_lib::{ErrorResponse, GetLevelResponse, GetLevelsRequest, LevelsListItem};
use mr_shared_lib::net::MessageId;
use reqwest::Client;
use serde::{de::DeserializeOwned, Serialize};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use url::Url;

#[derive(Clone)]
pub struct PersistenceClient {
    client: Client,
    public_persistence_url: Url,
}

impl PersistenceClient {
    pub fn new(client: Client, public_persistence_url: Url) -> Self {
        Self {
            client,
            public_persistence_url,
        }
    }

    pub async fn request<
        R: DeserializeOwned,
        E: Serialize + DeserializeOwned + Clone,
        B: Serialize,
        T: std::fmt::Display,
    >(
        &self,
        method: reqwest::Method,
        path: &str,
        id_token: Option<T>,
        body: &B,
    ) -> Option<Result<R, ErrorResponse<E>>> {
        let mut request = self
            .client
            .request(method, self.public_persistence_url.join(path).unwrap());
        if let Some(id_token) = id_token {
            request = request.bearer_auth(id_token);
        }
        let result = request.json(body).send().await;

        let (data, status) = match result {
            Ok(result) => {
                let status = result.status();
                (result.bytes().await, status)
            }
            Err(err) => {
                log::error!("Failed to send a request: {:?}", err);
                return None;
            }
        };

        #[cfg(debug_assertions)]
        if let Ok(data) = &data {
            log::debug!(
                "Persistence server HTTP response: {}",
                String::from_utf8_lossy(data.as_slice())
            );
        }

        if status.is_success() {
            match data
                .ok()
                .and_then(|data| serde_json::from_slice::<R>(data.as_slice()).ok())
            {
                Some(response) => Some(Ok(response)),
                None => {
                    log::error!(
                        "Failed to parse a body response from the persistence server ({:?}, status: {})",
                        path,
                        status.as_u16()
                    );
                    None
                }
            }
        } else {
            match data
                .ok()
                .and_then(|data| serde_json::from_slice::<ErrorResponse<E>>(data.as_slice()).ok())
            {
                Some(response) => Some(Err(response)),
                None => {
                    log::error!(
                        "Failed to parse a body response from the persistence server ({:?}, status: {})",
                        path,
                        status.as_u16()
                    );
                    None
                }
            }
        }
    }

    pub async fn get_levels(
        &self,
        query: &GetLevelsRequest,
    ) -> Option<Result<Vec<LevelsListItem>, ErrorResponse<()>>> {
        let query = serde_urlencoded::to_string(query).unwrap();
        self.request(
            reqwest::Method::GET,
            &format!("/levels?{query}"),
            Option::<&str>::None,
            &(),
        )
        .await
    }

    pub async fn get_level(
        &self,
        level_id: i64,
    ) -> Option<Result<GetLevelResponse, ErrorResponse<()>>> {
        self.request(
            reqwest::Method::GET,
            &format!("/levels/{level_id}"),
            Option::<&str>::None,
            &(),
        )
        .await
    }
}

#[derive(Debug)]
pub enum PersistenceRequest {
    GetLevels {
        request_id: MessageId,
        body: GetLevelsRequest,
    },
    GetLevel {
        request_id: MessageId,
        level_id: i64,
    },
}

#[derive(Debug)]
pub struct PersistenceMessage {
    pub request_id: MessageId,
    pub payload: PersistenceMessagePayload,
}

impl PersistenceMessage {
    pub fn new(request_id: MessageId, payload: PersistenceMessagePayload) -> Self {
        Self {
            request_id,
            payload,
        }
    }
}

#[derive(Debug)]
pub enum PersistenceMessagePayload {
    GetLevelsResponse(Vec<LevelsListItem>),
    GetLevelResponse(GetLevelResponse),
    RequestFailed(String),
}

pub struct PersistenceRequestsHandler {
    pub client: PersistenceClient,
    pub request_rx: UnboundedReceiver<PersistenceRequest>,
    pub message_tx: UnboundedSender<PersistenceMessage>,
}

impl PersistenceRequestsHandler {
    pub async fn serve(self) {
        let PersistenceRequestsHandler {
            client,
            mut request_rx,
            message_tx,
        } = self;
        while let Some(request) = request_rx.recv().await {
            let client = client.clone();
            let message_tx = message_tx.clone();
            let _ = match request {
                PersistenceRequest::GetLevels { request_id, body } => {
                    tokio::task::spawn_local(async move {
                        match client.get_levels(&body).await {
                            Some(Ok(response)) => message_tx.send(PersistenceMessage::new(
                                request_id,
                                PersistenceMessagePayload::GetLevelsResponse(response),
                            )),
                            _ => message_tx.send(PersistenceMessage::new(
                                request_id,
                                PersistenceMessagePayload::RequestFailed(
                                    "Failed to get the list of levels".to_owned(),
                                ),
                            )),
                        }
                        .expect("Failed to send a persistence message");
                    })
                }
                PersistenceRequest::GetLevel {
                    request_id,
                    level_id,
                } => tokio::task::spawn_local(async move {
                    match client.get_level(level_id).await {
                        Some(Ok(response)) => message_tx.send(PersistenceMessage::new(
                            request_id,
                            PersistenceMessagePayload::GetLevelResponse(response),
                        )),
                        _ => message_tx.send(PersistenceMessage::new(
                            request_id,
                            PersistenceMessagePayload::RequestFailed(
                                "Failed to get the level".to_owned(),
                            ),
                        )),
                    }
                    .expect("Failed to send a persistence message");
                }),
            };
        }
    }
}
