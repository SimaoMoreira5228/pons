use std::future::Future;
use std::marker::PhantomData;
use std::pin::Pin;

use serde::Serialize;
use serde::de::DeserializeOwned;
use tako::types::Request;
use tako::ws::{Message, WebSocket, WebSocketUpgrade};
use tako_rs_core::responder::Responder;
use tako_rs_core::types::Response;

use crate::describe::{DescribeResponse, OperationCx};

type Handler<Client, Server> = Box<
    dyn FnOnce(TypedSocket<Client, Server>) -> Pin<Box<dyn Future<Output = ()> + Send>>
        + Send
        + 'static,
>;

pub struct PonsWebSocket<Client, Server> {
    request: Request,
    handler: Handler<Client, Server>,
}

impl<Client, Server> PonsWebSocket<Client, Server> {
    pub fn new<H, Fut>(request: Request, handler: H) -> Self
    where
        Client: 'static,
        Server: 'static,
        H: FnOnce(TypedSocket<Client, Server>) -> Fut + Send + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        Self {
            request,
            handler: Box::new(move |socket| Box::pin(handler(socket))),
        }
    }
}

impl<Client, Server> Responder for PonsWebSocket<Client, Server>
where
    Client: 'static,
    Server: 'static,
{
    fn into_response(self) -> Response {
        let Self { request, handler } = self;
        WebSocketUpgrade::new(request, move |socket| handler(TypedSocket::new(socket)))
            .into_response()
    }
}

impl<Client: pons_core::Schema, Server: pons_core::Schema> DescribeResponse
    for PonsWebSocket<Client, Server>
{
    fn describe(cx: &mut OperationCx) {
        cx.websocket::<Client, Server>();
    }
}

pub struct TypedSocket<Client, Server> {
    inner: WebSocket,
    messages: PhantomData<fn() -> (Client, Server)>,
}

impl<Client, Server> TypedSocket<Client, Server> {
    fn new(inner: WebSocket) -> Self {
        Self {
            inner,
            messages: PhantomData,
        }
    }
}

impl<Client, Server> TypedSocket<Client, Server>
where
    Server: Serialize,
{
    pub async fn send(&mut self, message: Server) -> Result<(), TypedSocketError> {
        let frame = encode(message)?;
        self.inner
            .send(frame)
            .await
            .map_err(TypedSocketError::Transport)
    }
}

impl<Client, Server> TypedSocket<Client, Server>
where
    Client: DeserializeOwned,
{
    pub async fn recv(&mut self) -> Option<Result<Client, TypedSocketError>> {
        loop {
            match self.inner.recv().await? {
                Ok(Message::Text(text)) => return Some(decode(text.as_str())),
                Ok(Message::Binary(_)) => return Some(Err(TypedSocketError::NotText)),
                Ok(Message::Close(_)) => return None,
                Ok(Message::Ping(_) | Message::Pong(_)) => continue,
                Err(error) => return Some(Err(TypedSocketError::Transport(error))),
            }
        }
    }
}

fn encode<M: Serialize>(message: M) -> Result<Message, TypedSocketError> {
    serde_json::to_string(&message)
        .map(Message::text)
        .map_err(TypedSocketError::Encode)
}

fn decode<M: DeserializeOwned>(text: &str) -> Result<M, TypedSocketError> {
    serde_json::from_str(text).map_err(TypedSocketError::Decode)
}

#[derive(Debug)]
pub enum TypedSocketError {
    Encode(serde_json::Error),
    Decode(serde_json::Error),
    NotText,
    Transport(tako::ws::Error),
}

impl std::fmt::Display for TypedSocketError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Encode(error) => write!(formatter, "failed to encode a message as JSON: {error}"),
            Self::Decode(error) => write!(formatter, "failed to decode a JSON message: {error}"),
            Self::NotText => formatter.write_str("the incoming frame is not a text frame"),
            Self::Transport(error) => write!(formatter, "WebSocket transport error: {error}"),
        }
    }
}

impl std::error::Error for TypedSocketError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Encode(error) | Self::Decode(error) => Some(error),
            Self::NotText => None,
            Self::Transport(error) => Some(error),
        }
    }
}

#[cfg(test)]
mod tests {
    use serde::Deserialize;

    use super::*;

    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    struct Ping {
        seq: u64,
    }

    #[test]
    fn a_json_text_frame_round_trips() {
        let Message::Text(text) = encode(Ping { seq: 7 }).expect("the message encodes") else {
            panic!("a JSON message is a text frame")
        };
        let decoded = decode::<Ping>(text.as_str()).expect("the frame decodes");
        assert_eq!(decoded, Ping { seq: 7 });
    }

    #[test]
    fn malformed_json_is_a_typed_error() {
        let error = decode::<Ping>("{").expect_err("the JSON is incomplete");
        assert!(matches!(error, TypedSocketError::Decode(_)));
    }
}
