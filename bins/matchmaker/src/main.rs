use futures::{future, pin_mut, SinkExt, StreamExt, TryStreamExt};
use k8s_openapi::api::core::v1::Pod;
use kube::{
    api::{Api, ListParams, WatchEvent},
    Client,
};
use mr_messages_lib::{MatchmakerMessage, Server};
use std::{
    collections::HashMap,
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

    let mut builder = env_logger::Builder::from_default_env();
    builder.filter_level(log::LevelFilter::Info).init();

    log::info!("Starting the matchmaker server...");

    let (tx, rx) = tokio::sync::broadcast::channel(32);
    drop(rx);

    let servers = Servers::default();
    future::select(
        tokio::spawn(watch_pods(tx.clone(), servers.clone())),
        tokio::spawn(listen_websocket(tx, servers)),
    )
    .await;
}

async fn watch_pods(tx: Sender<MatchmakerMessage>, servers: Servers) {
    log::info!("Starting k8s client...");

    let client = Client::try_default().await.unwrap();
    let pods: Api<Pod> = Api::namespaced(client, "default");

    let lp = ListParams::default().labels("app=mr_server").timeout(0);
    let mut stream = pods
        .watch(&lp, "0")
        .await
        .expect("Failed to start watching pods")
        .boxed();

    log::info!("Watching pod updates...");

    let initial_list = pods
        .list(&lp)
        .await
        .expect("Failed to get a list of running pods")
        .items
        .into_iter()
        .filter_map(|pod| server_from_resource(&pod))
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
            WatchEvent::Added(resource) => {
                if let Some(server) = server_from_resource(&resource) {
                    log::info!("New server: {:?}", server);
                    servers.add(server.clone()).await;
                    Some(MatchmakerMessage::ServerUpdated(server))
                } else {
                    None
                }
            }
            WatchEvent::Modified(resource) => {
                if let Some(server) = server_from_resource(&resource) {
                    log::info!("Server updated: {:?}", server);
                    servers.add(server.clone()).await;
                    Some(MatchmakerMessage::ServerUpdated(server))
                } else {
                    None
                }
            }
            WatchEvent::Deleted(resource) => {
                if let Some(server) = server_from_resource(&resource) {
                    log::info!("Server removed: {:?}", server);
                    servers.remove(&server.name).await;
                    Some(MatchmakerMessage::ServerRemoved(server.name))
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

fn server_from_resource(resource: &Pod) -> Option<Server> {
    resource
        .spec
        .as_ref()
        .and_then(|spec| {
            spec.containers
                .iter()
                .find(|container| container.name == "mr-server")
                .map(|container| {
                    (
                        resource
                            .metadata
                            .name
                            .clone()
                            .expect("Expected a name for a server"),
                        container.clone(),
                    )
                })
        })
        .and_then(|(name, container)| {
            let status = resource.status.as_ref().expect("Expected pod status");
            if status.phase.as_deref() != Some("Running") {
                log::warn!("Pod {} is not yet in the running state", name);
                return None;
            }

            let host_ip = match &status.host_ip {
                Some(host_ip) => host_ip.parse::<IpAddr>().expect("Failed to parse host ip"),
                None => {
                    log::warn!("Host ip of {} pod is not yet allocated", name);
                    return None;
                }
            };
            let port = match container.ports.and_then(|ports| {
                ports
                    .iter()
                    .find(|port| port.protocol.as_deref() == Some("UDP"))
                    .cloned()
            }) {
                Some(port) => port.host_port.expect("Expected a host_port"),
                None => {
                    log::error!("Pod {} doesn't expose a UDP port", name);
                    return None;
                }
            };

            Some(Server {
                name,
                addr: SocketAddr::new(host_ip, port as u16),
            })
        })
}
