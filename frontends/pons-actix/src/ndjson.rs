use std::marker::PhantomData;

use actix_web::body::BoxBody;
use actix_web::{HttpRequest, HttpResponse, Responder};
use futures_util::{Stream, StreamExt};
use pons_core::Schema;
use serde::Serialize;

use crate::describe::{DescribeResponse, OperationCx, StatusResponse};

pub struct Ndjson<T, S> {
    stream: S,
    item: PhantomData<fn() -> T>,
}

impl<T, S> Ndjson<T, S> {
    pub fn new(stream: S) -> Self {
        Self {
            stream,
            item: PhantomData,
        }
    }
}

impl<T, S> Responder for Ndjson<T, S>
where
    T: Serialize + 'static,
    S: Stream<Item = T> + 'static,
{
    type Body = BoxBody;

    fn respond_to(self, _: &HttpRequest) -> HttpResponse<Self::Body> {
        let lines = self.stream.map(|item| {
            let mut line = serde_json::to_vec(&item).map_err(std::io::Error::other)?;
            line.push(b'\n');
            Ok::<_, std::io::Error>(actix_web::web::Bytes::from(line))
        });
        HttpResponse::Ok()
            .content_type("application/x-ndjson")
            .streaming(lines)
    }
}

impl<T, S> DescribeResponse for Ndjson<T, S>
where
    T: Schema,
{
    fn describe(cx: &mut OperationCx) {
        cx.ndjson::<T>();
    }
}

impl<T, S> StatusResponse for Ndjson<T, S>
where
    T: Schema + Serialize + 'static,
    S: Stream<Item = T> + 'static,
{
}
