use crate::websocket::CloseCode;
use futures::Stream;
use js_sys::Function;
use std::{
    convert::{TryFrom, TryInto},
    pin::Pin,
    task::{Context, Poll},
};
use tokio::sync::mpsc::UnboundedReceiver;
use url::Url;
use wasm_bindgen::{closure::Closure, JsCast, JsValue};
use wasm_bindgen_futures::{spawn_local, JsFuture};
use web_sys::{CloseEvent, Event, MessageEvent};

impl TryFrom<MessageEvent> for super::Message {
    type Error = anyhow::Error;

    fn try_from(event: MessageEvent) -> Result<Self, Self::Error> {
        let message = if let Ok(text) = event.data().dyn_into::<js_sys::JsString>() {
            super::Message::Text(text.into())
        } else if let Ok(binary) = event.data().dyn_into::<js_sys::ArrayBuffer>() {
            super::Message::Binary(js_sys::Uint8Array::new(&binary).to_vec())
        } else {
            return Err(anyhow::Error::msg("Unsupported message"));
        };
        Ok(message)
    }
}

/// Stream-based WebSocket.
#[derive(Debug)]
pub struct WebSocketStream {
    sender: WebSocketSender,
    receiver: WebSocketReceiver,
}

impl Drop for WebSocketStream {
    fn drop(&mut self) {
        self.sender.close(None);
    }
}

/// WebSocket sender. Also responsible for closing the connection.
#[derive(Debug, Clone)]
struct WebSocketSender {
    websocket: web_sys::WebSocket,
}

/// WebSocket receiver.
#[derive(Debug)]
struct WebSocketReceiver {
    receiver: UnboundedReceiver<anyhow::Result<super::Message>>,
    _on_message_callback: Closure<dyn FnMut(MessageEvent)>,
    _on_close_callback: Closure<dyn FnMut(CloseEvent)>,
}

impl WebSocketStream {
    /// Creates a new WebSocket and connects it to the specified `url`.
    /// Returns `ConnectionError` if it can't connect.
    pub async fn connect(url: &Url) -> anyhow::Result<Self> {
        let (sender, receiver) = tokio::sync::mpsc::unbounded_channel();

        let mut connection_callback = Box::new(|accept: Function, reject: Function| {
            // Connection
            let websocket =
                web_sys::WebSocket::new(url.as_ref()).expect("Couldn't create WebSocket.");
            {
                let js_value = websocket
                    .clone()
                    .dyn_into::<JsValue>()
                    .expect("Couldn't cast WebSocket to JsValue.");
                let onopen_callback = Closure::wrap(Box::new(move |_event| {
                    accept.call1(&JsValue::NULL, &js_value).ok();
                }) as Box<dyn FnMut(Event)>);
                websocket.set_onopen(Some(onopen_callback.as_ref().unchecked_ref()));
                onopen_callback.forget();
            }

            // Error handling.
            let onerror_callback = Closure::wrap(Box::new(move |_event| {
                reject.call0(&JsValue::NULL).ok();
            }) as Box<dyn FnMut(Event)>);
            websocket.set_onerror(Some(onerror_callback.as_ref().unchecked_ref()));
            onerror_callback.forget();
        }) as Box<dyn FnMut(Function, Function)>;
        let connection_promise = js_sys::Promise::new(&mut connection_callback);

        JsFuture::from(connection_promise)
            .await
            .map(move |websocket| {
                let websocket: web_sys::WebSocket = websocket
                    .dyn_into()
                    .expect("Couldn't cast JsValue to WebSocket.");

                // Message streaming.
                let _on_message_callback = {
                    let sender = sender.clone();
                    let _on_message_callback = Closure::wrap(Box::new(move |e: MessageEvent| {
                        sender.send(e.try_into()).ok();
                    })
                        as Box<dyn FnMut(MessageEvent)>);
                    websocket.set_onmessage(Some(_on_message_callback.as_ref().unchecked_ref()));
                    _on_message_callback
                };
                // Close event.
                let _on_close_callback = Closure::wrap(Box::new(move |_e: CloseEvent| {
                    sender
                        .send(Err(anyhow::Error::msg("Connection closed normally")))
                        .ok();
                })
                    as Box<dyn FnMut(CloseEvent)>);
                websocket.set_onclose(Some(_on_close_callback.as_ref().unchecked_ref()));

                websocket.set_binary_type(web_sys::BinaryType::Arraybuffer);
                WebSocketStream {
                    sender: WebSocketSender { websocket },
                    receiver: WebSocketReceiver {
                        receiver,
                        _on_message_callback,
                        _on_close_callback,
                    },
                }
            })
            .map_err(|_| anyhow::Error::msg("Failed to connect"))
    }
}

impl WebSocketSender {
    /// Attempts to close the connection and returns `SendError` if it fails.
    pub fn close(&mut self, message: Option<super::CloseFrame<'_>>) -> Result<(), ()> {
        self.send(&super::Message::Close(message.map(|msg| msg.into_owned())))
    }

    /// Attempts to send a message and returns `SendError` if it fails.
    pub fn send(&mut self, message: &super::Message) -> Result<(), ()> {
        if self.websocket.ready_state() == web_sys::WebSocket::OPEN {
            match message {
                super::Message::Text(text) => self.websocket.send_with_str(text).map_err(|_| ()),
                super::Message::Binary(binary) => {
                    self.websocket.send_with_u8_array(binary).map_err(|_| ())
                }
                super::Message::Close(close_frame) => if let Some(close_frame) = close_frame {
                    self.websocket.close_with_code_and_reason(
                        close_frame.code.into(),
                        close_frame.reason.as_ref(),
                    )
                } else {
                    self.websocket.close_with_code(CloseCode::Normal.into())
                }
                .map_err(|_| ()),
                _ => unimplemented!(),
            }
        } else {
            Err(())
        }
    }
}

impl WebSocketReceiver {
    /// Attempts to receive a message and returns `ReceiveError` if it fails.
    pub async fn next(&mut self) -> Option<anyhow::Result<super::Message>> {
        self.receiver.recv().await
    }
}

impl Stream for WebSocketStream {
    type Item = anyhow::Result<super::Message>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.receiver.receiver.poll_recv(cx)
    }
}
