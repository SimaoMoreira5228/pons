use std::marker::PhantomData;

use axum::body::Bytes;
use axum::response::IntoResponse;
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

impl<T, S> IntoResponse for Ndjson<T, S>
where
    T: Serialize + Send + 'static,
    S: Stream<Item = T> + Send + 'static,
{
    fn into_response(self) -> axum::response::Response {
        let stream = self.stream.map(|item| {
            let mut line = serde_json::to_vec(&item).map_err(std::io::Error::other)?;
            line.push(b'\n');
            Ok::<_, std::io::Error>(Bytes::from(line))
        });
        let mut response = axum::body::Body::from_stream(stream).into_response();
        response.headers_mut().insert(
            axum::http::header::CONTENT_TYPE,
            axum::http::HeaderValue::from_static("application/x-ndjson"),
        );
        response
    }
}

impl<T, S> DescribeResponse for Ndjson<T, S>
where
    T: Schema + Send + 'static,
    S: Stream<Item = T> + Send + 'static,
{
    fn describe(cx: &mut OperationCx) {
        cx.ndjson::<T>();
    }
}

impl<T, S> StatusResponse for Ndjson<T, S>
where
    T: Schema + Serialize + Send + 'static,
    S: Stream<Item = T> + Send + 'static,
{
}
