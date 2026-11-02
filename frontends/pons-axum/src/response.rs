use std::marker::PhantomData;

use axum::http::StatusCode;
use axum::http::header::CONTENT_TYPE;
use axum::response::{IntoResponse, Response};
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

pub trait MediaType {
    const VALUE: &'static str;
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
    response: Response,
    body: PhantomData<fn() -> (T, S)>,
}

impl<T, S> WithResponse<T, S> {
    pub fn new(response: Response) -> Self {
        Self {
            response,
            body: PhantomData,
        }
    }

    pub fn media<M: MediaType>(self) -> WithMedia<T, S, M> {
        WithMedia {
            response: self.response,
            body: PhantomData,
            media: PhantomData,
        }
    }
}

impl<T, S> IntoResponse for WithResponse<T, S>
where
    S: ResponseSpec,
{
    fn into_response(self) -> Response {
        respond(
            self.response,
            spec_status::<S>(),
            media_type(S::content(), S::media_type()),
        )
    }
}

impl<T, S> DescribeResponse for WithResponse<T, S>
where
    T: Schema,
    S: ResponseSpec,
{
    fn describe(cx: &mut OperationCx) {
        let media = S::media_type().and_then(|declared| checked_media(S::content(), declared, cx));
        cx.response_with_media::<T>(spec_status::<S>(), S::content(), media);
    }
}

impl<T, S> StatusResponse for WithResponse<T, S>
where
    T: Schema,
    S: ResponseSpec,
{
}

pub struct WithMedia<T, S, M> {
    response: Response,
    body: PhantomData<fn() -> (T, S)>,
    media: PhantomData<fn() -> M>,
}

impl<T, S, M> IntoResponse for WithMedia<T, S, M>
where
    S: ResponseSpec,
    M: MediaType,
{
    fn into_response(self) -> Response {
        respond(self.response, spec_status::<S>(), Some(M::VALUE))
    }
}

impl<T, S, M> DescribeResponse for WithMedia<T, S, M>
where
    T: Schema,
    S: ResponseSpec,
    M: MediaType,
{
    fn describe(cx: &mut OperationCx) {
        let media = checked_media(S::content(), M::VALUE, cx);
        cx.response_with_media::<T>(spec_status::<S>(), S::content(), media);
    }
}

impl<T, S, M> StatusResponse for WithMedia<T, S, M>
where
    T: Schema,
    S: ResponseSpec,
    M: MediaType,
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

impl<T> IntoResponse for WithUnknownStatus<T>
where
    T: IntoResponse,
{
    fn into_response(self) -> Response {
        self.0.into_response()
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

fn respond(mut response: Response, status: u16, content_type: Option<&'static str>) -> Response {
    if is_invalid_response_header(&response) {
        return response;
    }
    *response.status_mut() =
        StatusCode::from_u16(status).expect("the response wrapper requires a valid HTTP status");
    if let Some(content_type) = content_type {
        let Ok(value) = axum::http::HeaderValue::from_bytes(content_type.as_bytes()) else {
            return invalid_response_header();
        };
        response.headers_mut().insert(CONTENT_TYPE, value);
    }
    response
}

fn checked_media(
    content: ContentKind,
    declared: &'static str,
    cx: &mut OperationCx,
) -> Option<MediaDescriptor> {
    let header_value = axum::http::HeaderValue::from_bytes(declared.as_bytes()).is_ok();
    let Some(descriptor) = MediaDescriptor::parse(declared).filter(|_| header_value) else {
        cx.diagnostic(
            "PONS-E102",
            format!(
                "the response media type {declared:?} is not a valid media type; declare a type and subtype such as application/vnd.acme.report.v1+json"
            ),
        );
        return None;
    };
    let problem = descriptor.base() == "application/problem+json";
    match (content, problem) {
        (ContentKind::ProblemJson, false) => {
            cx.diagnostic(
                "PONS-E102",
                format!(
                    "a problem JSON response declares {declared:?}, but it must declare application/problem+json"
                ),
            );
            None
        }
        (content, true) if content != ContentKind::ProblemJson => {
            cx.diagnostic(
                "PONS-E102",
                format!(
                    "the response declares {declared:?}, which is problem JSON, but its content is {content:?}; serve it with ProblemJsonResponse"
                ),
            );
            None
        }
        _ => Some(descriptor),
    }
}

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
