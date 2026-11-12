use std::marker::PhantomData;

use http::{StatusCode, header};
use pons_ir::{ContentKind, MediaDescriptor};
use tako_rs_core::responder::Responder;
use tako_rs_core::types::Response;

use crate::describe::{DescribeResponse, OperationCx};

pub trait MediaType {
    const VALUE: &'static str;
}

pub struct WithMedia<R, M> {
    responder: R,
    media: PhantomData<fn() -> M>,
}

impl<R, M> WithMedia<R, M> {
    pub fn new(responder: R) -> Self {
        Self {
            responder,
            media: PhantomData,
        }
    }
}

impl<R, M> Responder for WithMedia<R, M>
where
    R: Responder,
    M: MediaType,
{
    fn into_response(self) -> Response {
        let mut response = self.responder.into_response();
        let Ok(value) = http::HeaderValue::from_bytes(M::VALUE.as_bytes()) else {
            return invalid_response_header();
        };
        response.headers_mut().insert(header::CONTENT_TYPE, value);
        response
    }
}

impl<R, M> DescribeResponse for WithMedia<R, M>
where
    R: DescribeResponse,
    M: MediaType,
{
    fn describe(cx: &mut OperationCx) {
        let start = cx.responses.len();
        R::describe(cx);
        let checked = cx.responses[start..]
            .iter()
            .map(|response| checked_media(response.content, M::VALUE))
            .collect::<Vec<_>>();
        let mut errors = Vec::new();
        for (response, media) in cx.responses[start..].iter_mut().zip(checked) {
            match media {
                Ok(media) => response.media = Some(media),
                Err(error) => errors.push(error),
            }
        }
        for error in errors {
            cx.diagnostic("PONS-E102", error);
        }
    }
}

fn invalid_response_header() -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        "invalid response header value",
    )
        .into_response()
}

fn checked_media(content: ContentKind, declared: &'static str) -> Result<MediaDescriptor, String> {
    let header_value = http::HeaderValue::from_bytes(declared.as_bytes()).is_ok();
    let Some(descriptor) = MediaDescriptor::parse(declared).filter(|_| header_value) else {
        return Err(format!(
            "the response media type {declared:?} is not a valid media type; declare a type and subtype such as application/vnd.acme.report.v1+json"
        ));
    };
    match (content, descriptor.base() == "application/problem+json") {
        (ContentKind::ProblemJson, false) => Err(format!(
            "a problem JSON response declares {declared:?}, but it must declare application/problem+json"
        )),
        (content, true) if content != ContentKind::ProblemJson => Err(format!(
            "the response declares {declared:?}, which is problem JSON, but its content is {content:?}; declare it on a response whose content is problem JSON"
        )),
        _ => Ok(descriptor),
    }
}
