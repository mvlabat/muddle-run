use futures::{Sink, Stream};
use std::{
    pin::Pin,
    task::{Context, Poll},
};
use tokio::net::TcpStream;
use tokio_tungstenite::{
    connect_async,
    tungstenite::{
        error::Error,
        protocol::{frame::coding::CloseCode, CloseFrame},
        Message,
    },
    MaybeTlsStream,
};
use url::Url;

impl From<Message> for super::Message {
    fn from(message: Message) -> Self {
        match message {
            Message::Text(payload) => super::Message::Text(payload),
            Message::Binary(payload) => super::Message::Binary(payload),
            Message::Ping(payload) => super::Message::Ping(payload),
            Message::Pong(payload) => super::Message::Pong(payload),
            Message::Close(payload) => super::Message::Close(payload.map(|payload| payload.into())),
            Message::Frame(_) => unreachable!(),
        }
    }
}

#[allow(clippy::from_over_into)]
impl Into<Message> for super::Message {
    fn into(self) -> Message {
        match self {
            super::Message::Text(payload) => Message::Text(payload),
            super::Message::Binary(payload) => Message::Binary(payload),
            super::Message::Ping(payload) => Message::Ping(payload),
            super::Message::Pong(payload) => Message::Pong(payload),
            super::Message::Close(payload) => Message::Close(payload.map(|payload| payload.into())),
        }
    }
}

impl<'t> From<CloseFrame<'t>> for super::CloseFrame<'t> {
    fn from(close_frame: CloseFrame<'t>) -> Self {
        super::CloseFrame {
            code: close_frame.code.into(),
            reason: close_frame.reason,
        }
    }
}

#[allow(clippy::from_over_into)]
impl<'t> Into<CloseFrame<'t>> for super::CloseFrame<'t> {
    fn into(self) -> CloseFrame<'t> {
        CloseFrame {
            code: self.code.into(),
            reason: self.reason,
        }
    }
}

impl From<CloseCode> for super::CloseCode {
    fn from(close_code: CloseCode) -> Self {
        match close_code {
            CloseCode::Normal => super::CloseCode::Normal,
            CloseCode::Away => super::CloseCode::Away,
            CloseCode::Protocol => super::CloseCode::Protocol,
            CloseCode::Unsupported => super::CloseCode::Unsupported,
            CloseCode::Status => super::CloseCode::Status,
            CloseCode::Abnormal => super::CloseCode::Abnormal,
            CloseCode::Invalid => super::CloseCode::Invalid,
            CloseCode::Policy => super::CloseCode::Policy,
            CloseCode::Size => super::CloseCode::Size,
            CloseCode::Extension => super::CloseCode::Extension,
            CloseCode::Error => super::CloseCode::Error,
            CloseCode::Restart => super::CloseCode::Restart,
            CloseCode::Again => super::CloseCode::Again,
            CloseCode::Tls => super::CloseCode::Tls,
            CloseCode::Reserved(code) => super::CloseCode::Reserved(code),
            CloseCode::Iana(code) => super::CloseCode::Iana(code),
            CloseCode::Library(code) => super::CloseCode::Library(code),
            CloseCode::Bad(code) => super::CloseCode::Bad(code),
        }
    }
}

#[allow(clippy::from_over_into)]
impl Into<CloseCode> for super::CloseCode {
    fn into(self) -> CloseCode {
        match self {
            super::CloseCode::Normal => CloseCode::Normal,
            super::CloseCode::Away => CloseCode::Away,
            super::CloseCode::Protocol => CloseCode::Protocol,
            super::CloseCode::Unsupported => CloseCode::Unsupported,
            super::CloseCode::Status => CloseCode::Status,
            super::CloseCode::Abnormal => CloseCode::Abnormal,
            super::CloseCode::Invalid => CloseCode::Invalid,
            super::CloseCode::Policy => CloseCode::Policy,
            super::CloseCode::Size => CloseCode::Size,
            super::CloseCode::Extension => CloseCode::Extension,
            super::CloseCode::Error => CloseCode::Error,
            super::CloseCode::Restart => CloseCode::Restart,
            super::CloseCode::Again => CloseCode::Again,
            super::CloseCode::Tls => CloseCode::Tls,
            super::CloseCode::Reserved(code) => CloseCode::Reserved(code),
            super::CloseCode::Iana(code) => CloseCode::Iana(code),
            super::CloseCode::Library(code) => CloseCode::Library(code),
            super::CloseCode::Bad(code) => CloseCode::Bad(code),
        }
    }
}

pub struct WebSocketStream {
    ws_stream: tokio_tungstenite::WebSocketStream<MaybeTlsStream<TcpStream>>,
}

impl WebSocketStream {
    pub async fn connect(url: &Url) -> anyhow::Result<Self> {
        let (ws_stream, _) = connect_async(url).await?;
        Ok(Self { ws_stream })
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

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        match Pin::new(&mut self.ws_stream).poll_next(cx) {
            Poll::Ready(Some(item)) => {
                Poll::Ready(Some(item.map(|item| item.into()).map_err(|err| err.into())))
            }
            Poll::Ready(None) => Poll::Ready(None),
            Poll::Pending => Poll::Pending,
        }
    }
}
