use crate::net::auth::AuthRequest;
use bevy::log;
use futures::{channel::mpsc::unbounded, StreamExt};
use tokio::sync::mpsc::UnboundedSender;
use wasm_bindgen::{closure::Closure, JsCast, JsValue};
use web_sys::StorageEvent;

pub async fn serve(auth_request_tx: UnboundedSender<AuthRequest>) {
    let (event_tx, mut event_rx) = unbounded();

    let callback = Closure::wrap(Box::new(move |event: JsValue| {
        let storage_event: StorageEvent = event.dyn_into().unwrap();
        let Some((key, new_value)) = storage_event.key().zip(storage_event.new_value()) else {
            return;
        };
        if key != "oauth_code" {
            return;
        }
        let Some((state_token, code)) = new_value.split_once(' ') else {
            log::warn!("Invalid oauth_code value: {}", new_value);
            return;
        };
        if let Err(err) = event_tx.unbounded_send((state_token.to_owned(), code.to_owned())) {
            log::error!("Failed to send local storage event: {:?}", err);
        }
    }) as Box<dyn Fn(JsValue)>);

    let window = web_sys::window().unwrap();
    window
        .add_event_listener_with_callback("storage", callback.as_ref().unchecked_ref())
        .expect("Failed to add LocalStorage event listener");

    auth_request_tx
        .send(AuthRequest::RedirectUrlServerPort(0))
        .expect("Failed to write to a channel (auth request)");

    loop {
        let Some((state, code)) = event_rx.next().await else {
            log::error!("The local storage event channel has exhausted");
            break;
        };
        auth_request_tx
            .send(AuthRequest::HandleOAuthResponse { state, code })
            .expect("Failed to write to a channel (auth request)");
    }
}
