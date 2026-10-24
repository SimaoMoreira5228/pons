use std::convert::Infallible;

use actix_web::body::BoxBody;
use actix_web::{HttpRequest, HttpResponse, Responder};
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

impl<S> Responder for RawBytes<S>
where
    S: Stream<Item = actix_web::web::Bytes> + 'static,
{
    type Body = BoxBody;

    fn respond_to(self, _: &HttpRequest) -> HttpResponse<Self::Body> {
        let stream = self.stream.map(Ok::<_, Infallible>);
        HttpResponse::Ok()
            .content_type("application/octet-stream")
            .streaming(stream)
    }
}

impl<S> DescribeResponse for RawBytes<S> {
    fn describe(cx: &mut OperationCx) {
        cx.bytes_stream("application/octet-stream");
    }
}

impl<S> StatusResponse for RawBytes<S> where S: Stream<Item = actix_web::web::Bytes> + 'static {}
