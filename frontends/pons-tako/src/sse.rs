use std::marker::PhantomData;

use futures_util::{Stream, StreamExt};
use http::{StatusCode, header};
use serde::Serialize;
use tako::Bytes;
use tako::body::TakoBody;
use tako_rs_core::responder::Responder;
use tako_rs_core::types::Response;

pub struct PonsSse<T, S> {
    stream: S,
    event: PhantomData<fn() -> T>,
}

impl<T, S> PonsSse<T, S> {
    pub fn new(stream: S) -> Self {
        Self {
            stream,
            event: PhantomData,
        }
    }
}

impl<T, S> Responder for PonsSse<T, S>
where
    T: Serialize + Send + 'static,
    S: Stream<Item = T> + Send + 'static,
{
    fn into_response(self) -> Response {
        let events = self.stream.map(|item| {
            serde_json::to_vec(&item).map(|json| {
                let mut frame = Vec::with_capacity(json.len() + 8);
                frame.extend_from_slice(b"data: ");
                frame.extend_from_slice(&json);
                frame.extend_from_slice(b"\n\n");
                Bytes::from(frame)
            })
        });
        http::Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "text/event-stream")
            .header(header::CACHE_CONTROL, "no-cache, no-store, must-revalidate")
            .header("X-Accel-Buffering", "no")
            .body(TakoBody::from_stream(events))
            .expect("valid SSE response")
    }
}
