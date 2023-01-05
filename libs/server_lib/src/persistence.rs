use crate::{
    net::FetchedLevelInfo, PersistenceMessageSender, PersistenceRequestReceiver,
    PersistenceRequestSender, TOKIO,
};
use bevy::{
    ecs::system::{Local, Res, ResMut, Resource},
    log,
    prelude::{Deref, DerefMut},
    utils::{HashMap, Instant},
};
use mr_messages_lib::{
    ErrorResponse, GetLevelResponse, GetRegisteredUserQuery, GetUserResponse, LevelData, LevelDto,
    PostLevelRequest, PostLevelResponse, RegisteredUser,
};
use mr_shared_lib::{
    game::level::{LevelObject, LevelState, ObjectRouteDesc},
    messages::EntityNetId,
    net::MessageId,
    registry::IncrementId,
};
use mr_utils_lib::jwks::poll_jwks;
use reqwest::{Client, Url};
use std::{ops::Deref, time::Duration};
use tokio::sync::mpsc::UnboundedSender;

const LEVEL_AUTOSAVE_PERIOD_SECS: u64 = 60;

#[derive(Resource, Clone)]
pub struct PersistenceConfig {
    pub public_url: Url,
    pub private_url: Url,
    pub google_web_client_id: String,
    pub google_desktop_client_id: String,
    pub auth0_client_id: String,
}

#[derive(Resource, Deref, DerefMut, Default)]
pub struct Jwks(pub mr_utils_lib::jwks::Jwks);

#[derive(Debug)]
pub enum PersistenceRequest {
    GetUser { id: MessageId, id_token: String },
    SaveLevel(PostLevelRequest),
}

#[derive(Debug)]
pub enum PersistenceMessage {
    UserInfoResponse {
        id: MessageId,
        user: Option<RegisteredUser>,
    },
    SaveLevelResponse(Result<PostLevelResponse, String>),
}

pub async fn get_user(persistence_url: Url, user_id: i64) -> anyhow::Result<GetUserResponse> {
    let client = reqwest::Client::new();

    let result = client
        .get(persistence_url.join(&format!("users/{user_id}")).unwrap())
        .send()
        .await?;

    let status = result.status();
    let data = result.bytes().await?;

    #[cfg(debug_assertions)]
    log::debug!(
        "Persistence server HTTP response: {}",
        String::from_utf8_lossy(&data)
    );

    if !status.is_success() {
        let error: ErrorResponse<()> = serde_json::from_slice(&data)?;
        return Err(anyhow::Error::msg(error.message));
    }

    let response: GetUserResponse = serde_json::from_slice(&data)?;
    Ok(response)
}

#[derive(Resource)]
pub struct InitLevelObjects(pub Vec<LevelObject>);

pub async fn load_level(
    persistence_url: Url,
    level_id: i64,
) -> anyhow::Result<(GetLevelResponse, InitLevelObjects)> {
    log::info!("Loading a level: {level_id}...");
    let client = reqwest::Client::new();

    let result = client
        .get(persistence_url.join(&format!("levels/{level_id}")).unwrap())
        .send()
        .await?;

    let status = result.status();
    let data = result.bytes().await?;

    #[cfg(debug_assertions)]
    log::debug!(
        "Persistence server HTTP response (status: {}): {}",
        status.as_u16(),
        String::from_utf8_lossy(&data)
    );

    if !status.is_success() {
        let error: ErrorResponse<()> = serde_json::from_slice(&data)?;
        return Err(anyhow::Error::msg(error.message));
    }

    let mut response: GetLevelResponse = serde_json::from_slice(&data)?;
    let level_objects: Vec<LevelObject> = serde_json::from_value(response.level.data.take())?;
    Ok((response, InitLevelObjects(level_objects)))
}

pub async fn create_level(
    persistence_url: Url,
    user_id: i64,
    user_name: Option<String>,
    title: String,
    level_data: LevelData,
) -> anyhow::Result<GetLevelResponse> {
    let response = post_level(
        persistence_url,
        &PostLevelRequest {
            title: title.clone(),
            user_id,
            data: level_data.clone(),
        },
    )
    .await?;
    Ok(GetLevelResponse {
        level: LevelDto {
            id: response.id,
            title,
            data: response.data,
            user_id,
            user_name,
            parent_id: match level_data {
                LevelData::Forked { parent_id, .. } => Some(parent_id),
                LevelData::Autosaved { .. } => unreachable!(),
                LevelData::Data { .. } => None,
            },
            created_at: response.created_at,
            updated_at: response.updated_at,
        },
        autosaved_versions: Vec::new(),
        level_permissions: Vec::new(),
    })
}

async fn post_level(
    persistence_url: Url,
    post_level_request: &PostLevelRequest,
) -> anyhow::Result<PostLevelResponse> {
    let client = reqwest::Client::new();

    let result = client
        .post(persistence_url.join("levels").unwrap())
        .json(post_level_request)
        .send()
        .await?;

    let status = result.status();
    let data = result.bytes().await?;

    #[cfg(debug_assertions)]
    log::debug!(
        "Persistence server HTTP response (status: {}): {}",
        status.as_u16(),
        String::from_utf8_lossy(&data)
    );

    if !status.is_success() {
        let error: ErrorResponse<()> = serde_json::from_slice(&data)?;
        return Err(anyhow::Error::msg(error.message));
    }

    Ok(serde_json::from_slice(&data)?)
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

pub fn save_level(
    mut last_sent: Local<Option<Instant>>,
    request_tx: Res<PersistenceRequestSender>,
    fetched_level_info: Option<Res<FetchedLevelInfo>>,
    level_state: Res<LevelState>,
) {
    let request_tx = match &**request_tx {
        Some(request_tx) => request_tx,
        None => return,
    };

    if last_sent.is_none() {
        *last_sent = Some(Instant::now());
        return;
    }

    if Instant::now().duration_since(last_sent.unwrap())
        < Duration::from_secs(LEVEL_AUTOSAVE_PERIOD_SECS)
    {
        return;
    }

    log::info!("Autosaving the level...");
    *last_sent = Some(Instant::now());

    let level_objects = remap_net_ids(&level_state.objects);
    let fetched_level_info = fetched_level_info.unwrap().into_inner();
    let request = PostLevelRequest {
        title: fetched_level_info.level.title.clone(),
        user_id: fetched_level_info.level.user_id,
        data: LevelData::Autosaved {
            autosaved_level_id: fetched_level_info.level.id,
            data: serde_json::to_value(level_objects).unwrap(),
        },
    };

    if let Err(err) = request_tx.send(PersistenceRequest::SaveLevel(request)) {
        log::error!("Failed to send a persistence request: {:?}", err);
    }
}

fn remap_net_ids(level_objects_map: &HashMap<EntityNetId, LevelObject>) -> Vec<LevelObject> {
    let mut level_objects: Vec<LevelObject> = Vec::new();
    let mut ids_map: HashMap<EntityNetId, EntityNetId> = HashMap::default();
    let mut entity_net_id_counter = EntityNetId(0);
    let mut dependencies: HashMap<EntityNetId, Vec<usize>> = HashMap::default();

    for (i, mut object) in level_objects_map.values().cloned().enumerate() {
        let new_net_id = entity_net_id_counter.increment();
        ids_map.insert(object.net_id, new_net_id);

        if let Some(route) = &mut object.route {
            match &mut route.desc {
                ObjectRouteDesc::Attached(Some(id)) | ObjectRouteDesc::Radial(Some(id)) => {
                    if let Some(new_id) = ids_map.get(id) {
                        *id = *new_id;
                    } else {
                        dependencies.entry(*id).or_default().push(i);
                    }
                }
                ObjectRouteDesc::ForwardBackwardsCycle(ids)
                | ObjectRouteDesc::ForwardCycle(ids) => {
                    for id in ids {
                        if let Some(new_id) = ids_map.get(id) {
                            *id = *new_id;
                        } else {
                            dependencies.entry(*id).or_default().push(i);
                        }
                    }
                }
                ObjectRouteDesc::Attached(None) | ObjectRouteDesc::Radial(None) => {}
            }
        }

        if let Some(dependencies) = dependencies.remove(&object.net_id) {
            for i in dependencies {
                let route = &mut level_objects[i].route;
                match route.as_mut().map(|route| &mut route.desc) {
                    Some(
                        ObjectRouteDesc::Attached(Some(id)) | ObjectRouteDesc::Radial(Some(id)),
                    ) => {
                        if *id == object.net_id {
                            *id = new_net_id;
                        }
                    }
                    Some(
                        ObjectRouteDesc::ForwardBackwardsCycle(ids)
                        | ObjectRouteDesc::ForwardCycle(ids),
                    ) => {
                        for id in ids {
                            if *id == object.net_id {
                                *id = new_net_id;
                            }
                        }
                    }
                    Some(ObjectRouteDesc::Attached(None) | ObjectRouteDesc::Radial(None))
                    | None => {}
                }
            }
        }

        object.net_id = new_net_id;
        level_objects.push(object);
    }

    level_objects
}

pub fn handle_persistence_requests(
    config: Option<Res<PersistenceConfig>>,
    jwks: Res<Jwks>,
    mut request_rx: ResMut<PersistenceRequestReceiver>,
    response_tx: Res<PersistenceMessageSender>,
) {
    let Some(config) = config.map(|config| config.clone()) else {
        return;
    };
    let jwks = jwks.clone();
    let mut request_rx = request_rx.take().unwrap();
    let response_tx = response_tx
        .deref()
        .as_ref()
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

                    tokio::spawn(get_registered_user(
                        client.clone(),
                        config.clone(),
                        response_tx.clone(),
                        id,
                        GetRegisteredUserQuery {
                            subject: jwt.claims().custom.sub.clone(),
                            issuer: jwt.claims().custom.iss.clone(),
                        },
                    ));
                }
                Some(PersistenceRequest::SaveLevel(post_level_request)) => {
                    let persistence_url = config.private_url.clone();
                    let response_tx = response_tx.clone();
                    tokio::spawn(async move {
                        let result = post_level(persistence_url, &post_level_request)
                            .await
                            .map_err(|err| {
                                log::error!("Failed to autosave the level: {:?}", err);
                                "Failed to autosave the level".to_owned()
                            });
                        if let Err(err) =
                            response_tx.send(PersistenceMessage::SaveLevelResponse(result))
                        {
                            log::error!("Failed to send a persistence message: {:?}", err);
                        }
                    });
                }
                None => {
                    log::error!("Persistence channel closed");
                    return;
                }
            }
        }
    });
}

async fn get_registered_user(
    client: Client,
    config: PersistenceConfig,
    response_tx: UnboundedSender<PersistenceMessage>,
    request_id: MessageId,
    request: GetRegisteredUserQuery,
) {
    let result = client
        .get(config.private_url.join("user").unwrap())
        .query(&request)
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
