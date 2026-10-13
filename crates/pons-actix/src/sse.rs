use std::marker::PhantomData;

use actix_web::body::BoxBody;
use actix_web::{HttpRequest, HttpResponse, Responder};
use futures_util::{Stream, StreamExt};
use pons_core::Schema;
use serde::Serialize;

use crate::describe::{DescribeResponse, OperationCx, StatusResponse};

pub trait SseEvent: Schema + Serialize {
    const NAME: &'static str;
}

pub struct Sse<T, S> {
    stream: S,
    event: PhantomData<fn() -> T>,
}

impl<T, S> Sse<T, S> {
    pub fn new(stream: S) -> Self {
        Self {
            stream,
            event: PhantomData,
        }
    }
}

impl<T, S> Responder for Sse<T, S>
where
    T: SseEvent + 'static,
    S: Stream<Item = T> + 'static,
{
    type Body = BoxBody;

    fn respond_to(self, _: &HttpRequest) -> HttpResponse<Self::Body> {
        let events = self.stream.map(|data| {
            let mut frame = format!("event: {}\ndata: ", T::NAME).into_bytes();
            frame.extend(serde_json::to_vec(&data).map_err(std::io::Error::other)?);
            frame.extend_from_slice(b"\n\n");
            Ok::<_, std::io::Error>(actix_web::web::Bytes::from(frame))
        });
        HttpResponse::Ok()
            .content_type("text/event-stream")
            .streaming(events)
    }
}

impl<T, S> DescribeResponse for Sse<T, S>
where
    T: SseEvent,
{
    fn describe(cx: &mut OperationCx) {
        cx.sse::<T>(T::NAME);
    }
}

impl<T, S> StatusResponse for Sse<T, S>
where
    T: SseEvent + 'static,
    S: Stream<Item = T> + 'static,
{
}
