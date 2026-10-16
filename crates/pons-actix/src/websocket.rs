use std::future::{Future, Ready, ready};
use std::marker::PhantomData;

use actix_web::body::BoxBody;
use actix_web::{FromRequest, HttpRequest, HttpResponse, Responder, web};
use actix_ws::{MessageStream, Session};
use futures_util::future::LocalBoxFuture;
use pons_core::Schema;

use crate::describe::{DescribeRequest, DescribeResponse, OperationCx, StatusResponse};

type Handler = Box<dyn FnOnce(Session, MessageStream) -> LocalBoxFuture<'static, ()>>;

pub struct WebSocketUpgrade<Client, Server> {
    request: HttpRequest,
    payload: web::Payload,
    messages: PhantomData<fn() -> (Client, Server)>,
}

impl<Client, Server> WebSocketUpgrade<Client, Server> {
    pub fn on_upgrade<F, Fut>(self, handler: F) -> WebSocket<Client, Server>
    where
        F: FnOnce(Session, MessageStream) -> Fut + 'static,
        Fut: Future<Output = ()> + 'static,
    {
        WebSocket {
            request: self.request,
            payload: self.payload,
            handler: Box::new(move |session, messages| Box::pin(handler(session, messages))),
            messages: PhantomData,
        }
    }
}

impl<Client, Server> FromRequest for WebSocketUpgrade<Client, Server> {
    type Error = std::convert::Infallible;
    type Future = Ready<Result<Self, Self::Error>>;

    fn from_request(req: &HttpRequest, payload: &mut actix_web::dev::Payload) -> Self::Future {
        let payload = web::Payload::from_request(req, payload)
            .into_inner()
            .expect("the payload extractor is infallible");
        ready(Ok(Self {
            request: req.clone(),
            payload,
            messages: PhantomData,
        }))
    }
}

impl<Client, Server> DescribeRequest for WebSocketUpgrade<Client, Server> {
    fn describe(_: &mut OperationCx) {}
}

pub struct WebSocket<Client, Server> {
    request: HttpRequest,
    payload: web::Payload,
    handler: Handler,
    messages: PhantomData<fn() -> (Client, Server)>,
}

impl<Client, Server> Responder for WebSocket<Client, Server> {
    type Body = BoxBody;

    fn respond_to(self, _: &HttpRequest) -> HttpResponse<Self::Body> {
        match actix_ws::handle(&self.request, self.payload) {
            Ok((response, session, messages)) => {
                actix_web::rt::spawn(async move {
                    (self.handler)(session, messages).await;
                });
                response
            }
            Err(_) => HttpResponse::BadRequest().finish(),
        }
    }
}

impl<Client: Schema, Server: Schema> DescribeResponse for WebSocket<Client, Server> {
    fn describe(cx: &mut OperationCx) {
        cx.websocket::<Client, Server>();
    }
}

impl<Client: Schema, Server: Schema> StatusResponse for WebSocket<Client, Server> {}
