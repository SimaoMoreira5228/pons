use std::marker::PhantomData;

use futures_util::{Stream, StreamExt};
use http::{StatusCode, header};
use pons_core::Schema;
use serde::Serialize;
use tako::Bytes;
use tako::body::TakoBody;
use tako_rs_core::responder::Responder;
use tako_rs_core::types::Response;

use crate::describe::{DescribeResponse, OperationCx};

pub struct PonsNdjson<T, S> {
    stream: S,
    item: PhantomData<fn() -> T>,
}

impl<T, S> PonsNdjson<T, S> {
    pub fn new(stream: S) -> Self {
        Self {
            stream,
            item: PhantomData,
        }
    }
}

impl<T, S> Responder for PonsNdjson<T, S>
where
    T: Serialize + Send + 'static,
    S: Stream<Item = T> + Send + 'static,
{
    fn into_response(self) -> Response {
        let lines = self.stream.map(|item| {
            serde_json::to_vec(&item).map(|mut line| {
                line.push(b'\n');
                Bytes::from(line)
            })
        });
        http::Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "application/x-ndjson")
            .body(TakoBody::from_stream(lines))
            .expect("valid NDJSON response")
    }
}

impl<T, S> DescribeResponse for PonsNdjson<T, S>
where
    T: Schema + Send + 'static,
    S: Stream<Item = T> + Send + 'static,
{
    fn describe(cx: &mut OperationCx) {
        cx.ndjson::<T>();
    }
}
