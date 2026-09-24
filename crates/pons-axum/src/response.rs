use std::marker::PhantomData;

use axum::response::{IntoResponse, Response};
use pons_core::Schema;
use pons_ir::ContentKind;

use crate::describe::{DescribeResponse, OperationCx, StatusResponse};

pub trait ResponseSpec {
    fn status() -> u16;
    fn content() -> ContentKind;
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
}

impl<T, S> IntoResponse for WithResponse<T, S>
where
    S: ResponseSpec,
{
    fn into_response(self) -> Response {
        let mut response = self.response;
        *response.status_mut() = axum::http::StatusCode::from_u16(valid_status::<S>())
            .expect("WithResponse requires a valid HTTP status");
        if !response
            .headers()
            .contains_key(axum::http::header::CONTENT_TYPE)
            && let Some(content_type) = media_type(S::content())
        {
            response.headers_mut().insert(
                axum::http::header::CONTENT_TYPE,
                axum::http::HeaderValue::from_static(content_type),
            );
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
        cx.response::<T>(valid_status::<S>(), S::content());
    }
}

impl<T, S> StatusResponse for WithResponse<T, S>
where
    T: Schema,
    S: ResponseSpec,
{
}

fn media_type(content: ContentKind) -> Option<&'static str> {
    match content {
        ContentKind::Json => Some("application/json"),
        ContentKind::ProblemJson => Some("application/problem+json"),
        ContentKind::Text => Some("text/plain"),
        ContentKind::Bytes => Some("application/octet-stream"),
        ContentKind::Form | ContentKind::Multipart | ContentKind::Stream | ContentKind::Empty => {
            None
        }
    }
}

fn valid_status<S: ResponseSpec>() -> u16 {
    let status = S::status();
    assert!(
        (100..=599).contains(&status),
        "WithResponse requires an HTTP status representable by Pons IR"
    );
    status
}
