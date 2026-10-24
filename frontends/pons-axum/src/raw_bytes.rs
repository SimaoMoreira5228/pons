use std::convert::Infallible;

use axum::body::Bytes;
use axum::response::IntoResponse;
use futures_util::{Stream, StreamExt};

use crate::describe::{DescribeResponse, OperationCx, StatusResponse};

pub struct RawBytes<S> {
    stream: S,
}

impl<S> RawBytes<S> {
    pub fn new(stream: S) -> Self {
        Self { stream }
    }
}

impl<S> IntoResponse for RawBytes<S>
where
    S: Stream<Item = Bytes> + Send + 'static,
{
    fn into_response(self) -> axum::response::Response {
        let stream = self.stream.map(Ok::<_, Infallible>);
        let mut response = axum::body::Body::from_stream(stream).into_response();
        response.headers_mut().insert(
            axum::http::header::CONTENT_TYPE,
            axum::http::HeaderValue::from_static("application/octet-stream"),
        );
        response
    }
}

impl<S> DescribeResponse for RawBytes<S>
where
    S: Stream<Item = Bytes> + Send + 'static,
{
    fn describe(cx: &mut OperationCx) {
        cx.bytes_stream("application/octet-stream");
    }
}

impl<S> StatusResponse for RawBytes<S> where S: Stream<Item = Bytes> + Send + 'static {}
