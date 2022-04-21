#![feature(int_roundings)]
#![feature(async_closure)]

mod game_server_allocation;
mod jwks;
mod persistence;

use crate::{
    game_server_allocation::{post_game_server_allocation, PostGameServerAllocationParams},
    jwks::poll_jwks,
    persistence::get_registered_user,
};
use future::FutureExt;
use futures::{future, pin_mut, stream::BoxStream, SinkExt, StreamExt, TryFutureExt, TryStreamExt};
use kube::{
    api::{Api, ListParams, WatchEvent},
    Client, CustomResource,
};
use mr_messages_lib::{
    deserialize_binary, serialize_binary, GameServerState, GetRegisteredUserQuery, InitLevel,
    MatchmakerMessage, MatchmakerRequest, Server,
};
use mr_utils_lib::{jwks::Jwks, kube_discovery, try_parse_from_env};
use reqwest::Url;
use schemars::JsonSchema;
use serde::Deserializer;
use serde_derive::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    io::Read,
    net::{IpAddr, SocketAddr},
};
use tokio::{
    net::{TcpListener, TcpStream},
    sync::{
        broadcast::{Receiver, Sender},
        Mutex, MutexGuard,
    },
};
use tokio_tungstenite::{tungstenite, tungstenite::Message};

#[derive(Clone)]
pub struct Config {
    private_persistence_url: Url,
    google_certs_url: Url,
    auth0_certs_url: Url,
    google_web_client_id: String,
    google_desktop_client_id: String,
    auth0_client_id: String,
}

#[derive(Clone, Default)]
pub struct Servers {
    servers: std::sync::Arc<Mutex<HashMap<String, Server>>>,
}

#[derive(Clone, Default)]
pub struct CreateServerRequests {
    requests:
        std::sync::Arc<Mutex<HashMap<SocketAddr, (uuid::Uuid, PostGameServerAllocationParams)>>>,
}

impl CreateServerRequests {
    pub async fn lock(
        &self,
    ) -> MutexGuard<'_, HashMap<SocketAddr, (uuid::Uuid, PostGameServerAllocationParams)>> {
        self.requests.lock().await
    }
}

#[derive(CustomResource, Debug, Serialize, Deserialize, Default, Clone, JsonSchema)]
#[kube(group = "agones.dev", version = "v1", kind = "GameServer", namespaced)]
#[kube(status = "GameServerStatus")]
pub struct GameServerSpec {
    container: String,
}

#[derive(Debug, Serialize, Deserialize, Default, Clone, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GameServerStatus {
    pub state: GameServerState,
    #[serde(deserialize_with = "deserialize_null_default")]
    pub ports: Vec<GameServerPort>,
    pub address: String,
    pub node_name: String,
    pub players: GameServerPlayerStatus,
}

#[derive(Debug, Serialize, Deserialize, Default, Clone, JsonSchema)]
pub struct GameServerPort {
    name: String,
    port: u16,
}

#[derive(Debug, Serialize, Deserialize, Default, Clone, JsonSchema)]
pub struct GameServerPlayerStatus {
    count: u64,
    capacity: u64,
}

impl Servers {
    pub async fn init(&self, initial_list: Vec<Server>) {
        let mut servers = self.servers.lock().await;
        servers.clear();
        for server in initial_list {
            servers.insert(server.name.clone(), server);
        }
    }

    pub async fn add(&self, server: Server) {
        let mut servers = self.servers.lock().await;
        servers.insert(server.name.clone(), server);
    }

    pub async fn get(&self, name: &str) -> Option<Server> {
        let servers = self.servers.lock().await;
        servers.get(name).cloned()
    }

    pub async fn remove(&self, name: &str) -> Option<Server> {
        let mut servers = self.servers.lock().await;
        servers.remove(name)
    }

    pub async fn all(&self) -> Vec<Server> {
        let servers = self.servers.lock().await;
        servers.values().cloned().collect()
    }

    pub async fn allocated_count(&self) -> usize {
        let servers = self.servers.lock().await;
        servers
            .values()
            .filter(|server| server.state == GameServerState::Allocated)
            .count()
    }
}

#[tokio::main]
async fn main() {
    mr_utils_lib::env::load_env();

    // TODO: add sentry support and move panic handler to the utils crate.
    let orig_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic_info| {
        orig_hook(panic_info);

        // A kludge to let sentry send events first and then shutdown.
        std::thread::spawn(|| {
            std::thread::sleep(std::time::Duration::new(1, 0));
            std::process::exit(1);
        });
    }));

    let _guard = sentry::init(sentry::ClientOptions {
        release: sentry::release_name!(),
        ..Default::default()
    });

    let mut builder = env_logger::Builder::from_default_env();
    builder.filter_level(log::LevelFilter::Info).init();

    log::info!("Starting the matchmaker server...");

    let client = Client::try_default()
        .await
        .expect("Unable to detect kubernetes environment");

    let private_persistence_url: Option<Url> =
        try_parse_from_env!("MUDDLE_PRIVATE_PERSISTENCE_URL");
    let public_persistence_url: Option<Url> = try_parse_from_env!("MUDDLE_PUBLIC_PERSISTENCE_URL");
    let cloned_client = client.clone();
    let (_public_persistence_url, private_persistence_url) = future::ready(
        public_persistence_url
            .zip(private_persistence_url)
            .ok_or(()),
    )
    .or_else(async move |_| {
        kube_discovery::discover_persistence(cloned_client)
            .await
            .ok_or(())
    })
    .await
    .expect("Failed to discover the persistence service");

    let config = Config {
        private_persistence_url,
        google_certs_url: "https://www.googleapis.com/oauth2/v3/certs"
            .parse()
            .unwrap(),
        auth0_certs_url: "https://muddle-run.eu.auth0.com/.well-known/jwks.json"
            .parse()
            .unwrap(),
        google_web_client_id: std::env::var("MUDDLE_GOOGLE_WEB_CLIENT_ID")
            .expect("Expected MUDDLE_GOOGLE_WEB_CLIENT_ID"),
        google_desktop_client_id: std::env::var("MUDDLE_GOOGLE_DESKTOP_CLIENT_ID")
            .expect("Expected MUDDLE_GOOGLE_DESKTOP_CLIENT_ID"),
        auth0_client_id: std::env::var("MUDDLE_AUTH0_CLIENT_ID")
            .expect("Expected MUDDLE_AUTH0_CLIENT_ID"),
    };

    let (tx, rx) = tokio::sync::broadcast::channel(32);
    drop(rx);

    let servers = Servers::default();
    let create_server_requests = CreateServerRequests::default();
    let jwks = Jwks::default();
    let mut watch_game_servers = tokio::spawn(watch_game_servers(
        client.clone(),
        tx.clone(),
        servers.clone(),
    ))
    .fuse();
    let mut serve_webhook_service =
        tokio::spawn(serve_webhook_service(tx.clone(), servers.clone())).fuse();
    let mut listen_websocket = tokio::spawn(listen_websocket(HandleConnectionParams {
        tx,
        kube_client: client,
        reqwest_client: Default::default(),
        servers,
        create_server_requests,
        jwks: jwks.clone(),
        config: config.clone(),
    }))
    .fuse();
    let mut poll_jwks = tokio::spawn(poll_jwks(config, jwks)).fuse();
    futures::select!(
        _ = watch_game_servers => {},
        _ = serve_webhook_service => {},
        _ = listen_websocket => {},
        _ = poll_jwks => {},
    );
}

async fn watch_game_servers(client: Client, tx: Sender<MatchmakerMessage>, servers: Servers) {
    let game_servers: Api<GameServer> = Api::namespaced(client, "default");
    log::info!("Watching GameServer updates...");
    let mut stream = init_stream_and_watch(game_servers.clone(), servers.clone()).await;

    loop {
        let status = match stream
            .try_next()
            .await
            .expect("Failed to read from the k8s stream")
        {
            Some(status) => status,
            None => {
                log::info!("The k8s stream has ended, re-subscribing");
                stream = init_stream_and_watch(game_servers.clone(), servers.clone()).await;
                continue;
            }
        };

        let message = match status {
            WatchEvent::Added(resource) | WatchEvent::Modified(resource) => {
                if let Some(server_command) = server_command_from_resource(&resource) {
                    log::info!("Resource updated: {:?}", resource.status);
                    match server_command {
                        ServerCommand::Update(server) => {
                            servers.add(server.clone()).await;
                            Some(MatchmakerMessage::ServerUpdated(server))
                        }
                        ServerCommand::Delete(server_name) => {
                            if servers.remove(&server_name).await.is_some() {
                                Some(MatchmakerMessage::ServerRemoved(server_name))
                            } else {
                                None
                            }
                        }
                    }
                } else {
                    None
                }
            }
            WatchEvent::Deleted(resource) => {
                if let Some(server_command) = server_command_from_resource(&resource) {
                    log::info!("Resource deleted: {:?}", server_command);
                    match server_command {
                        ServerCommand::Update(server) => {
                            servers.remove(&server.name).await;
                            Some(MatchmakerMessage::ServerRemoved(server.name))
                        }
                        ServerCommand::Delete(server_name) => {
                            if servers.remove(&server_name).await.is_some() {
                                Some(MatchmakerMessage::ServerRemoved(server_name))
                            } else {
                                None
                            }
                        }
                    }
                } else {
                    None
                }
            }
            WatchEvent::Error(err) => {
                log::error!("Error event: {:?}", err);
                None
            }
            WatchEvent::Bookmark(_) => None,
        };

        if let Some(message) = message {
            let _ = tx.send(message);
        }
    }
}

async fn init_stream_and_watch<'a>(
    game_servers: Api<GameServer>,
    servers: Servers,
) -> BoxStream<'a, kube::Result<WatchEvent<GameServer>>> {
    let lp = ListParams::default().labels("app=mr_server").timeout(0);
    let stream = game_servers
        .watch(&lp, "0")
        .await
        .expect("Failed to start watching game servers")
        .boxed();

    let initial_list = game_servers
        .list(&lp)
        .await
        .expect("Failed to get a list of running game servers")
        .items
        .into_iter()
        .filter_map(|gs| {
            if let Some(ServerCommand::Update(server)) = server_command_from_resource(&gs) {
                Some(server)
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    let list_len = initial_list.len();
    servers.init(initial_list).await;

    log::info!("Initialized the server list ({list_len} servers)");

    stream
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct FleetAutoscaleReview {
    request: FleetAutoscaleRequest,
    response: Option<FleetAutoscaleResponse>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct FleetAutoscaleRequest {
    uid: String,
    name: String,
    namespace: String,
    status: FleetStatus,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct FleetAutoscaleResponse {
    uid: String,
    scale: bool,
    replicas: u32,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
struct FleetStatus {
    replicas: u32,
    ready_replicas: u32,
    reserved_replicas: u32,
    allocated_replicas: u32,
}

async fn serve_webhook_service(tx: Sender<MatchmakerMessage>, servers: Servers) {
    let make_svc = hyper::service::make_service_fn(move |_conn| {
        fn bad_request() -> hyper::Response<hyper::Body> {
            hyper::Response::builder()
                .status(400)
                .body(hyper::Body::empty())
                .unwrap()
        }

        let tx = tx.clone();
        let servers = servers.clone();

        let serve = move |req: hyper::Request<hyper::Body>| {
            let tx = tx.clone();
            let servers = servers.clone();
            async move {
                let json_string = match hyper::body::aggregate(req)
                    .await
                    .map_err(anyhow::Error::msg)
                    .and_then(|body| {
                        use hyper::body::Buf;
                        let mut json_string = String::new();
                        body.reader().read_to_string(&mut json_string)?;
                        Ok(json_string)
                    }) {
                    Ok(body) => body,
                    Err(err) => {
                        log::error!("Failed to read body: {:?}", err);
                        return Ok::<_, std::convert::Infallible>(bad_request());
                    }
                };

                log::info!("Incoming request: {}", json_string);

                let mut fleet_autoscale_review: FleetAutoscaleReview =
                    match serde_json::from_str(&json_string) {
                        Ok(request) => request,
                        Err(err) => {
                            log::error!("Failed to parse body: {:?}", err);
                            return Ok(bad_request());
                        }
                    };

                let active_players = tx.receiver_count() as u32;
                let allocated_servers = servers.allocated_count().await as u32;
                let desired_replicas_count = active_players.max(1) + allocated_servers;
                fleet_autoscale_review.response = Some(FleetAutoscaleResponse {
                    uid: fleet_autoscale_review.request.uid.clone(),
                    scale: desired_replicas_count != fleet_autoscale_review.request.status.replicas,
                    replicas: desired_replicas_count,
                });

                log::info!(
                    "Webhook response (active players: {}, allocated servers: {}): {:?}",
                    active_players,
                    allocated_servers,
                    fleet_autoscale_review.response.as_ref().unwrap()
                );

                let body = serde_json::to_vec(&fleet_autoscale_review).unwrap();
                Ok(hyper::Response::new(
                    hyper::body::Bytes::copy_from_slice(&body).into(),
                ))
            }
        };

        async { Ok::<_, std::convert::Infallible>(hyper::service::service_fn(serve)) }
    });

    let addr = ([0, 0, 0, 0], 8081).into();

    let server = hyper::Server::bind(&addr).serve(make_svc);

    log::info!("Webhook is listening on http://{}", addr);

    if let Err(err) = server.await {
        log::error!("An error occurred while serving the webhook: {:?}", err);
    }
}

async fn listen_websocket(params: HandleConnectionParams) {
    let addr = "0.0.0.0:8080";
    let listener = TcpListener::bind(addr).await.expect("Failed to bind");
    log::info!("Listening on: {}", addr);

    loop {
        match listener.accept().await {
            Ok((stream, addr)) => {
                tokio::spawn(handle_connection(
                    addr,
                    stream,
                    params.tx.subscribe(),
                    params.clone(),
                ));
            }
            Err(err) => {
                log::error!("Failed to accept a websocket connection {err:?}");
            }
        }
    }
}

#[derive(Clone)]
struct HandleConnectionParams {
    tx: Sender<MatchmakerMessage>,
    kube_client: Client,
    reqwest_client: reqwest::Client,
    servers: Servers,
    create_server_requests: CreateServerRequests,
    jwks: Jwks,
    config: Config,
}

async fn handle_connection(
    addr: SocketAddr,
    stream: TcpStream,
    mut rx: Receiver<MatchmakerMessage>,
    params: HandleConnectionParams,
) {
    log::debug!("Incoming TCP connection from: {}", addr);

    let ws_stream = match tokio_tungstenite::accept_async(stream).await {
        Ok(ws_stream) => ws_stream,
        Err(err) => {
            log::debug!("Error during the websocket handshake occurred: {:?}", err);
            return;
        }
    };
    log::info!("WebSocket connection established: {}", addr);

    let create_server_requests = params.create_server_requests.clone();

    let (mut outgoing, mut incoming) = ws_stream.split();
    let drain_incoming = async move {
        while let Some(message) = incoming.next().await {
            let message = match message {
                Ok(message) => message,
                Err(err) => {
                    log::warn!("Connection error: {:?}", err);
                    break;
                }
            };

            let matchmaker_request = match message {
                tungstenite::Message::Binary(data) => {
                    match deserialize_binary::<MatchmakerRequest>(&data) {
                        Ok(matchmaker_request) => matchmaker_request,
                        Err(err) => {
                            log::error!(
                                "Failed to deserialize matchmaker request, disconnecting: {:?}",
                                err
                            );
                            break;
                        }
                    }
                }
                _ => continue,
            };

            match matchmaker_request {
                MatchmakerRequest::CreateServer {
                    init_level,
                    request_id,
                    id_token,
                } => {
                    log::info!("Received a request to create a server: {request_id}");
                    let user_id = if let Some(id_token) = id_token {
                        let jwt = match params
                            .jwks
                            .decode(
                                &id_token,
                                &[
                                    &params.config.google_web_client_id,
                                    &params.config.google_desktop_client_id,
                                    &params.config.auth0_client_id,
                                ],
                            )
                            .await
                        {
                            Ok(jwt) => jwt,
                            Err(err) => {
                                log::warn!("Invalid JWT: {:?}", err);
                                params
                                    .tx
                                    .send(MatchmakerMessage::InvalidJwt(request_id))
                                    .expect("Failed to send a persistence message");
                                continue;
                            }
                        };

                        let registered_user = get_registered_user(
                            &params.reqwest_client,
                            &params.config,
                            GetRegisteredUserQuery {
                                subject: jwt.claims().custom.sub.clone(),
                                issuer: jwt.claims().custom.iss.clone(),
                            },
                        )
                        .await
                        .expect("Failed to get a registered user");
                        let registered_user = match registered_user {
                            Some(registered_user) => registered_user,
                            None => {
                                log::warn!("Invalid JWT: no user found with the id_token");
                                params
                                    .tx
                                    .send(MatchmakerMessage::InvalidJwt(request_id))
                                    .expect("Failed to send a persistence message");
                                continue;
                            }
                        };
                        Some(registered_user.id)
                    } else {
                        None
                    };

                    let post_game_server_allocation_params = match init_level {
                        InitLevel::Create { title, parent_id } => PostGameServerAllocationParams {
                            request_id,
                            user_id,
                            level_title: Some(title),
                            level_parent_id: parent_id,
                            level_id: None,
                        },
                        InitLevel::Existing(level_id) => PostGameServerAllocationParams {
                            request_id,
                            user_id,
                            level_title: None,
                            level_parent_id: None,
                            level_id: Some(level_id),
                        },
                    };
                    post_game_server_allocation(
                        params.kube_client.clone(),
                        post_game_server_allocation_params.clone(),
                    )
                    .await
                    .expect("Failed to post a game server allocation");

                    let mut create_server_requests =
                        params.create_server_requests.requests.lock().await;
                    create_server_requests
                        .insert(addr, (request_id, post_game_server_allocation_params));
                }
            }
        }
    };

    let current_servers = params.servers.all().await;
    if let Err(err) = outgoing
        .send(Message::Binary(
            serialize_binary(&MatchmakerMessage::Init {
                servers: current_servers,
            })
            .expect("Failed to serialize an init message"),
        ))
        .await
    {
        log::warn!(
            "Failed to send an init message to {}, disconnecting: {:?}",
            addr,
            err
        );
        return;
    }

    let broadcast = async move {
        while let Ok(message) = rx.recv().await {
            let message = Message::binary(
                serialize_binary(&message).expect("Failed to serialize a broadcasted message"),
            );
            if let Err(err) = outgoing.send(message).await {
                log::warn!("Failed to send a message to {}: {:?}", addr, err);
                break;
            }
        }
    };

    pin_mut!(drain_incoming, broadcast);
    future::select(drain_incoming, broadcast).await;
    let mut create_server_requests = create_server_requests.lock().await;
    create_server_requests.remove(&addr);

    log::info!("{} disconnected", addr);
}

#[derive(Debug)]
enum ServerCommand {
    Update(Server),
    Delete(String),
}

fn server_command_from_resource(resource: &GameServer) -> Option<ServerCommand> {
    resource
        .status
        .as_ref()
        .and_then(|status: &GameServerStatus| {
            let name = match &resource.metadata.name {
                Some(name) => name.clone(),
                None => {
                    log::error!("GameServer doesn't have a name, skipping");
                    return None;
                }
            };

            if !matches!(
                status.state,
                GameServerState::Ready | GameServerState::Allocated
            ) {
                log::info!(
                    "GameServer {} is not in the Ready or Allocated state (current: {:?}), deleting",
                    name,
                    status.state
                );
                return Some(ServerCommand::Delete(name));
            }

            let ip_addr = match status.address.parse::<IpAddr>() {
                Ok(ip) => ip,
                Err(err) => {
                    log::warn!(
                        "Skipping GameServer {} (failed to parse ip address '{}': {:?})",
                        name,
                        status.address,
                        err
                    );
                    return None;
                }
            };
            let GameServerPort { port, .. } = match status
                .ports
                .iter()
                .find(|port| port.name == "MUDDLE_LISTEN_PORT-udp")
                .cloned()
            {
                Some(port) => port,
                None => {
                    log::warn!("GameServer {} doesn't expose a UDP port, skipping", name);
                    return None;
                }
            };

            let request_id = resource
                .metadata
                .annotations
                .as_ref()
                .and_then(|annotations| annotations.get("request_id"))
                .and_then(|id| id.parse().ok())
                .unwrap_or_default();

            Some(ServerCommand::Update(Server {
                name,
                state: status.state,
                addr: SocketAddr::new(ip_addr, port),
                player_capacity: status.players.capacity as u16,
                player_count: status.players.count as u16,
                request_id,
            }))
        })
}

fn deserialize_null_default<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    T: Default + serde::Deserialize<'de>,
    D: Deserializer<'de>,
{
    let opt = <Option<_> as serde::Deserialize>::deserialize(deserializer)?;
    Ok(opt.unwrap_or_default())
}
