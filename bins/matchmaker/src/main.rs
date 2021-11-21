#![feature(int_roundings)]
#![feature(async_closure)]

use future::FutureExt;
use futures::{future, pin_mut, SinkExt, StreamExt, TryStreamExt};
use kube::{
    api::{Api, ListParams, WatchEvent},
    Client,
};
use kube_derive::CustomResource;
use mr_messages_lib::{MatchmakerMessage, Server, PLAYER_CAPACITY};
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
        Mutex,
    },
};
use tokio_tungstenite::tungstenite::Message;

#[derive(Clone, Default)]
pub struct Servers {
    servers: std::sync::Arc<Mutex<HashMap<String, Server>>>,
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
    state: String,
    #[serde(deserialize_with = "deserialize_null_default")]
    ports: Vec<GameServerPort>,
    address: String,
    node_name: String,
    players: GameServerPlayerStatus,
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

    pub async fn remove(&self, name: &str) {
        let mut servers = self.servers.lock().await;
        servers.remove(name);
    }

    pub async fn all(&self) -> Vec<Server> {
        let servers = self.servers.lock().await;
        servers.values().cloned().collect()
    }
}

#[tokio::main]
async fn main() {
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

    let (tx, rx) = tokio::sync::broadcast::channel(32);
    drop(rx);

    let servers = Servers::default();
    let mut watch_game_servers =
        tokio::spawn(watch_game_servers(tx.clone(), servers.clone())).fuse();
    let mut serve_webhook_service =
        tokio::spawn(serve_webhook_service(tx.clone(), servers.clone())).fuse();
    let mut listen_websocket = tokio::spawn(listen_websocket(tx, servers)).fuse();
    futures::select!(
        _ = watch_game_servers => {},
        _ = serve_webhook_service => {},
        _ = listen_websocket => {},
    );
}

async fn watch_game_servers(tx: Sender<MatchmakerMessage>, servers: Servers) {
    log::info!("Starting k8s client...");

    let client = Client::try_default().await.unwrap();
    let game_servers: Api<GameServer> = Api::namespaced(client, "default");

    let lp = ListParams::default().labels("app=mr_server").timeout(0);
    let mut stream = game_servers
        .watch(&lp, "0")
        .await
        .expect("Failed to start watching game servers")
        .boxed();

    log::info!("Watching GameServer updates...");

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

    log::info!("Initialized the server list ({} servers)", list_len);

    while let Some(status) = stream
        .try_next()
        .await
        .expect("Failed to read from the k8s stream")
    {
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
                            servers.remove(&server_name).await;
                            Some(MatchmakerMessage::ServerRemoved(server_name))
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
                            servers.remove(&server_name).await;
                            Some(MatchmakerMessage::ServerRemoved(server_name))
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

                let servers = servers.all().await;
                let active_players = tx.receiver_count() as u32
                    + servers.iter().map(|s| s.player_count).sum::<u16>() as u32;
                let desired_replicas_count =
                    active_players.unstable_div_ceil(PLAYER_CAPACITY as u32);

                fleet_autoscale_review.response = Some(FleetAutoscaleResponse {
                    uid: fleet_autoscale_review.request.uid.clone(),
                    scale: desired_replicas_count != fleet_autoscale_review.request.status.replicas,
                    replicas: desired_replicas_count,
                });

                log::info!(
                    "Webhook response: {:?}",
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

async fn listen_websocket(tx: Sender<MatchmakerMessage>, servers: Servers) {
    let addr = "0.0.0.0:8080";
    let listener = TcpListener::bind(addr).await.expect("Failed to bind");
    log::info!("Listening on: {}", addr);

    while let Ok((stream, addr)) = listener.accept().await {
        tokio::spawn(handle_connection(
            addr,
            stream,
            tx.subscribe(),
            servers.clone(),
        ));
    }
}

async fn handle_connection(
    addr: SocketAddr,
    stream: TcpStream,
    mut rx: Receiver<MatchmakerMessage>,
    servers: Servers,
) {
    log::info!("Incoming TCP connection from: {}", addr);

    let ws_stream = match tokio_tungstenite::accept_async(stream).await {
        Ok(ws_stream) => ws_stream,
        Err(err) => {
            log::error!("Error during the websocket handshake occurred: {:?}", err);
            return;
        }
    };
    log::info!("WebSocket connection established: {}", addr);

    let (mut outgoing, incoming) = ws_stream.split();
    let drain_incoming = incoming.map(|_| Ok(())).forward(futures::sink::drain());

    let current_servers = servers.all().await;
    if let Err(err) = outgoing
        .send(Message::Binary(
            bincode::serialize(&MatchmakerMessage::Init(current_servers))
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
                bincode::serialize(&message).expect("Failed to serialize a broadcasted message"),
            );
            if let Err(err) = outgoing.send(message).await {
                log::warn!("Failed to send a message to {}: {:?}", addr, err);
                break;
            }
        }
    };

    pin_mut!(drain_incoming, broadcast);
    future::select(drain_incoming, broadcast).await;

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

            if status.state != "Ready" && status.state != "Allocated" && status.state != "Reserved" {
                log::info!(
                    "GameServer {} is not in Ready, Allocated or Reserved state (current: {}), deleting",
                    name,
                    status.state
                );
                return Some(ServerCommand::Delete(name));
            }

            let ip_addr = match status.address.parse::<IpAddr>() {
                Ok(ip) => ip,
                Err(err) => {
                    log::error!(
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
                    log::error!("GameServer {} doesn't expose a UDP port, skipping", name);
                    return None;
                }
            };
            Some(ServerCommand::Update(Server {
                name,
                addr: SocketAddr::new(ip_addr, port),
                player_capacity: PLAYER_CAPACITY,
                player_count: 0,
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
