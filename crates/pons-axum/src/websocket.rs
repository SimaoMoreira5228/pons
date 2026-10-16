use std::future::Future;
use std::marker::PhantomData;

use axum::extract::FromRequestParts;
use axum::extract::ws::WebSocketUpgrade as AxumWebSocketUpgrade;
use axum::http::request::Parts;
use axum::response::{IntoResponse, Response};
use pons_core::Schema;

use crate::describe::{DescribeRequest, DescribeResponse, OperationCx, StatusResponse};

pub struct WebSocketUpgrade<Client, Server> {
    inner: AxumWebSocketUpgrade,
    messages: PhantomData<fn() -> (Client, Server)>,
}

impl<Client, Server, S> FromRequestParts<S> for WebSocketUpgrade<Client, Server>
where
    S: Send + Sync,
{
    type Rejection = <AxumWebSocketUpgrade as FromRequestParts<S>>::Rejection;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let inner = AxumWebSocketUpgrade::from_request_parts(parts, state).await?;
        Ok(Self {
            inner,
            messages: PhantomData,
        })
    }
}

impl<Client, Server> DescribeRequest for WebSocketUpgrade<Client, Server> {
    fn describe(_: &mut OperationCx) {}
}

impl<Client, Server> WebSocketUpgrade<Client, Server> {
    pub fn on_upgrade<F, Fut>(self, callback: F) -> WebSocketResponse<Client, Server, F>
    where
        F: FnOnce(axum::extract::ws::WebSocket) -> Fut + Send + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        WebSocketResponse {
            upgrade: self.inner,
            callback,
            messages: PhantomData,
        }
    }
}

pub struct WebSocketResponse<Client, Server, F> {
    upgrade: AxumWebSocketUpgrade,
    callback: F,
    messages: PhantomData<fn() -> (Client, Server)>,
}

impl<Client, Server, F, Fut> IntoResponse for WebSocketResponse<Client, Server, F>
where
    F: FnOnce(axum::extract::ws::WebSocket) -> Fut + Send + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    fn into_response(self) -> Response {
        self.upgrade.on_upgrade(self.callback)
    }
}

impl<Client, Server, F> DescribeResponse for WebSocketResponse<Client, Server, F>
where
    Client: Schema,
    Server: Schema,
{
    fn describe(cx: &mut OperationCx) {
        cx.websocket::<Client, Server>();
    }
}

impl<Client, Server, F, Fut> StatusResponse for WebSocketResponse<Client, Server, F>
where
    Client: Schema,
    Server: Schema,
    F: FnOnce(axum::extract::ws::WebSocket) -> Fut + Send + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
}
