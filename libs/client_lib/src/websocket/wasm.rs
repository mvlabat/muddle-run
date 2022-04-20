use futures::{Sink, Stream};
use std::{
    pin::Pin,
    task::{Context, Poll},
};
use url::Url;
use ws_stream_wasm::{WsMessage as Message, WsMeta, WsStream};

impl From<Message> for super::Message {
    fn from(message: Message) -> Self {
        match message {
            Message::Text(payload) => super::Message::Text(payload),
            Message::Binary(payload) => super::Message::Binary(payload),
        }
    }
}

#[allow(clippy::from_over_into)]
impl Into<Message> for super::Message {
    fn into(self) -> Message {
        match self {
            super::Message::Text(payload) => Message::Text(payload),
            super::Message::Binary(payload) => Message::Binary(payload),
            _ => unimplemented!(),
        }
    }
}

pub struct WebSocketStream {
    ws_meta: WsMeta,
    ws_stream: WsStream,
}

impl WebSocketStream {
    pub async fn connect(url: &Url) -> anyhow::Result<Self> {
        let (ws_meta, ws_stream) = WsMeta::connect(url, None).await?;
        Ok(Self { ws_meta, ws_stream })
    }
}

impl Sink<super::Message> for WebSocketStream {
    type Error = anyhow::Error;

    fn poll_ready(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Pin::new(&mut self.ws_stream)
            .poll_ready(cx)
            .map_err(anyhow::Error::from)
    }

    fn start_send(mut self: Pin<&mut Self>, item: super::Message) -> Result<(), Self::Error> {
        Ok(Pin::new(&mut self.ws_stream).start_send(item.into())?)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Pin::new(&mut self.ws_stream)
            .poll_flush(cx)
            .map_err(anyhow::Error::from)
    }

    fn poll_close(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Pin::new(&mut self.ws_stream)
            .poll_close(cx)
            .map_err(anyhow::Error::from)
    }
}

impl Stream for WebSocketStream {
    type Item = anyhow::Result<super::Message>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let ws_stream = unsafe { self.map_unchecked_mut(|s| &mut s.ws_stream) };
        match ws_stream.poll_next(cx) {
            Poll::Ready(Some(item)) => Poll::Ready(Some(Ok(item.into()))),
            Poll::Ready(None) => Poll::Ready(None),
            Poll::Pending => Poll::Pending,
        }
    }
}
