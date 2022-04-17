use crate::{
    net::TcpConnectionStatus,
    websocket::{Message, WebSocketStream},
};
use bevy::log;
use futures::{select, FutureExt, SinkExt, StreamExt, TryStreamExt};
use mr_messages_lib::{deserialize_binary, serialize_binary, MatchmakerMessage, MatchmakerRequest};
use std::time::Duration;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use url::Url;

pub struct MatchmakerRequestsHandler {
    pub url: Url,
    pub connection_request_rx: UnboundedReceiver<bool>,
    pub status_tx: UnboundedSender<TcpConnectionStatus>,
    pub matchmaker_message_tx: UnboundedSender<MatchmakerMessage>,
}

impl MatchmakerRequestsHandler {
    pub async fn serve(mut self, matchmaker_request_rx: UnboundedReceiver<MatchmakerRequest>) {
        let mut matchmaker_request_rx = Some(matchmaker_request_rx);
        let mut current_state = false;

        let mut disconnect_request_tx = None;
        let (disconnect_tx, mut disconnect_rx) = tokio::sync::mpsc::unbounded_channel();
        loop {
            let connect = select! {
                connect = self.connection_request_rx.recv().fuse() => connect,
                disconnected_matchmaker_request_rx = disconnect_rx.recv().fuse() => {
                    matchmaker_request_rx = Some(disconnected_matchmaker_request_rx.expect("Disconnect channel closed unexpectedly"));
                    Some(false)
                },
            };
            let connect = match connect {
                Some(connect) => connect,
                None => break,
            };

            if current_state == connect {
                continue;
            }

            if connect {
                log::info!("Connecting to the matchmaker service...");
                let _ = self.status_tx.send(TcpConnectionStatus::Connecting);
                let message_tx = self.matchmaker_message_tx.clone();
                let url = self.url.clone();
                let ws_status_tx = self.status_tx.clone();
                let disconnect_request_channel = tokio::sync::oneshot::channel();
                disconnect_request_tx = Some(disconnect_request_channel.0);
                tokio::task::spawn_local(handle_matchmaker_connection(
                    message_tx,
                    matchmaker_request_rx.take().unwrap(),
                    url,
                    ws_status_tx,
                    disconnect_request_channel.1,
                    disconnect_tx.clone(),
                ));
            } else {
                // If this fails, the connection is already closed.
                if disconnect_request_tx.take().unwrap().send(()).is_ok() {
                    log::info!("Dropping the connection with the matchmaker service...");
                    let _ = self.status_tx.send(TcpConnectionStatus::Disconnected);
                }
                tokio::time::sleep(Duration::from_millis(500)).await;
            }

            current_state = connect;
        }
        panic!("Failed to read from a channel (matchmaker connection request)");
    }
}

async fn handle_matchmaker_connection(
    message_tx: UnboundedSender<MatchmakerMessage>,
    mut matchmaker_request_rx: UnboundedReceiver<MatchmakerRequest>,
    url: Url,
    ws_status_tx: UnboundedSender<TcpConnectionStatus>,
    disconnect_request_rx: tokio::sync::oneshot::Receiver<()>,
    disconnect_tx: UnboundedSender<UnboundedReceiver<MatchmakerRequest>>,
) {
    let (mut ws_sink, mut ws_stream) = match WebSocketStream::connect(&url).await {
        Ok(ws_stream) => ws_stream.split(),
        Err(err) => {
            log::error!("Failed to connect to matchmaker: {:?}", err);
            let _ = ws_status_tx.send(TcpConnectionStatus::Disconnected);
            disconnect_tx.send(matchmaker_request_rx).unwrap();
            return;
        }
    };
    let mut disconnect_rx = disconnect_request_rx.fuse();

    let _ = ws_status_tx.send(TcpConnectionStatus::Connected);
    log::info!("Successfully connected to the matchmacker");

    loop {
        let message = select! {
            message = ws_stream.try_next().fuse() => message,
            message = matchmaker_request_rx.recv().fuse() => {
                let message = message.expect("Unexpected end of the requests stream");
                let message = Message::Binary(serialize_binary(&message).expect("Failed to serialize MatchmakerRequest"));
                if let Err(err) = ws_sink.send(message).await {
                    log::error!("Matchmaker connection error: {:?}", err);
                    break;
                }
                continue;
            }
            _ = disconnect_rx => break,
        };

        let message: crate::websocket::Message = match message {
            Ok(Some(message)) => message,
            Ok(None) => {
                // In practice, the stream is unlikely to get exhausted before receiving an error.
                log::warn!("Matchmaker stream has exhausted, disconnecting");
                break;
            }
            Err(err) => {
                log::error!("Matchmaker connection error: {:?}", err);
                break;
            }
        };

        let matchmaker_message = match message {
            crate::websocket::Message::Binary(data) => {
                match deserialize_binary::<MatchmakerMessage>(&data) {
                    Ok(server_update) => server_update,
                    Err(err) => {
                        log::error!(
                            "Failed to deserialize matchmaker message, disconnecting: {:?}",
                            err
                        );
                        break;
                    }
                }
            }
            _ => continue,
        };

        if let Err(err) = message_tx.send(matchmaker_message) {
            log::error!("Failed to send a matchmaker update: {:?}", err);
        }
    }

    let _ = ws_status_tx.send(TcpConnectionStatus::Disconnected);
    disconnect_tx.send(matchmaker_request_rx).unwrap();
}
