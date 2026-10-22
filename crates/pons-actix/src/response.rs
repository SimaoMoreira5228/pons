use std::marker::PhantomData;

use actix_web::body::{BoxBody, MessageBody};
use actix_web::http::StatusCode;
use actix_web::http::header::{CONTENT_TYPE, HeaderValue};
use actix_web::{HttpRequest, HttpResponse, Responder};
use pons_core::Schema;
use pons_ir::{ContentKind, MediaDescriptor, ResponseStatus};

use crate::describe::{
    DescribeResponse, OperationCx, StatusResponse, invalid_response_header,
    is_invalid_response_header, valid_status,
};

pub trait ResponseSpec {
    fn status() -> u16;
    fn content() -> ContentKind;
    fn media_type() -> Option<&'static str> {
        None
    }
}

pub struct JsonResponse<const STATUS: u16>;

impl<const STATUS: u16> ResponseSpec for JsonResponse<STATUS> {
    fn status() -> u16 {
        STATUS
    }

    fn content() -> ContentKind {
        ContentKind::Json
    }
}

pub struct ProblemJsonResponse<const STATUS: u16>;

impl<const STATUS: u16> ResponseSpec for ProblemJsonResponse<STATUS> {
    fn status() -> u16 {
        STATUS
    }

    fn content() -> ContentKind {
        ContentKind::ProblemJson
    }
}

pub struct TextResponse<const STATUS: u16>;

impl<const STATUS: u16> ResponseSpec for TextResponse<STATUS> {
    fn status() -> u16 {
        STATUS
    }

    fn content() -> ContentKind {
        ContentKind::Text
    }
}

pub struct BytesResponse<const STATUS: u16>;

impl<const STATUS: u16> ResponseSpec for BytesResponse<STATUS> {
    fn status() -> u16 {
        STATUS
    }

    fn content() -> ContentKind {
        ContentKind::Bytes
    }
}

pub struct WithResponse<T, S = JsonResponse<200>> {
    response: HttpResponse<BoxBody>,
    body: PhantomData<fn() -> (T, S)>,
}

impl<T, S> WithResponse<T, S> {
    pub fn new<B: MessageBody + 'static>(response: HttpResponse<B>) -> Self {
        Self {
            response: response.map_into_boxed_body(),
            body: PhantomData,
        }
    }
}

impl<T, S> Responder for WithResponse<T, S>
where
    S: ResponseSpec,
{
    type Body = BoxBody;

    fn respond_to(self, _: &HttpRequest) -> HttpResponse<Self::Body> {
        let mut response = self.response;
        if is_invalid_response_header(&response) {
            return response;
        }
        *response.status_mut() = StatusCode::from_u16(spec_status::<S>())
            .expect("WithResponse requires a valid HTTP status");
        if let Some(content_type) = media_type(S::content(), S::media_type()) {
            let Ok(value) = HeaderValue::from_bytes(content_type.as_bytes()) else {
                return invalid_response_header();
            };
            response.headers_mut().insert(CONTENT_TYPE, value);
        }
        response
    }
}

impl<T, S> DescribeResponse for WithResponse<T, S>
where
    T: Schema,
    S: ResponseSpec,
{
    fn describe(cx: &mut OperationCx) {
        cx.response_with_media::<T>(
            spec_status::<S>(),
            S::content(),
            S::media_type().map(MediaDescriptor::from_declared),
        );
    }
}

impl<T, S> StatusResponse for WithResponse<T, S>
where
    T: Schema,
    S: ResponseSpec,
{
}

pub struct WithUnknownStatus<T>(pub T);

impl<T> WithUnknownStatus<T> {
    pub fn new(response: T) -> Self {
        Self(response)
    }

    pub fn into_inner(self) -> T {
        self.0
    }
}

impl<T> Responder for WithUnknownStatus<T>
where
    T: Responder,
{
    type Body = T::Body;

    fn respond_to(self, request: &HttpRequest) -> HttpResponse<Self::Body> {
        self.0.respond_to(request)
    }
}

impl<T> DescribeResponse for WithUnknownStatus<T>
where
    T: DescribeResponse,
{
    fn describe(cx: &mut OperationCx) {
        let start = cx.responses.len();
        T::describe(cx);
        for response in &mut cx.responses[start..] {
            response.status = ResponseStatus::Unknown;
        }
    }
}

impl<T> StatusResponse for WithUnknownStatus<T> where T: StatusResponse {}

fn media_type(content: ContentKind, override_type: Option<&'static str>) -> Option<&'static str> {
    override_type.or(match content {
        ContentKind::Json => Some("application/json"),
        ContentKind::ProblemJson => Some("application/problem+json"),
        ContentKind::Text => Some("text/plain"),
        ContentKind::Bytes => Some("application/octet-stream"),
        ContentKind::Form | ContentKind::Multipart | ContentKind::Stream | ContentKind::Empty => {
            None
        }
    })
}

fn spec_status<S: ResponseSpec>() -> u16 {
    valid_status(S::status(), "WithResponse")
}
