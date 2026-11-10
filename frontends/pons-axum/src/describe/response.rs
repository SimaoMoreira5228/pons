use axum::response::IntoResponse;
use pons_core::{ErrorSpec, Schema, TypeCx};
use pons_ir::{
    ContentKind, Direction, InlineType, Presence, ResponseHeader, ResponseStatus, TypeRef,
};

use super::header::{HeaderSpec, header_schema};
use super::{DescribeResponse, OperationCx, StatusResponse};

#[derive(Clone)]
pub(crate) struct InvalidResponseHeader;

pub struct WithHeader<S: HeaderSpec, R>(pub R, pub S::Value);

impl<S: HeaderSpec, R> WithHeader<S, R> {
    pub fn new(body: R, value: S::Value) -> Self {
        Self(body, value)
    }
}

pub struct WithOptionalHeader<S: HeaderSpec, R>(pub R, pub Option<S::Value>);

impl<S: HeaderSpec, R> WithOptionalHeader<S, R> {
    pub fn new(body: R, value: Option<S::Value>) -> Self {
        Self(body, value)
    }
}

pub(crate) fn insert_response_header<S>(
    response: &mut axum::response::Response,
    value: &S::Value,
) -> Result<(), &'static str>
where
    S: HeaderSpec,
    S::Value: ToString,
{
    let name = axum::http::HeaderName::from_bytes(S::NAME.as_bytes())
        .map_err(|_| "invalid response header name")?;
    let value = axum::http::HeaderValue::from_bytes(value.to_string().as_bytes())
        .map_err(|_| "invalid response header value")?;
    response.headers_mut().insert(name, value);
    Ok(())
}

pub(crate) fn invalid_response_header() -> axum::response::Response {
    let mut response = (
        axum::http::StatusCode::INTERNAL_SERVER_ERROR,
        "invalid response header value",
    )
        .into_response();
    response.extensions_mut().insert(InvalidResponseHeader);
    response
}

pub(crate) fn is_invalid_response_header(response: &axum::response::Response) -> bool {
    response
        .extensions()
        .get::<InvalidResponseHeader>()
        .is_some()
}

pub(crate) fn valid_status(status: u16, wrapper: &str) -> u16 {
    assert!(
        (100..=599).contains(&status),
        "{wrapper} requires an HTTP status representable by Pons IR"
    );
    status
}

fn describe_response_header<S: HeaderSpec>(cx: &mut OperationCx, start: usize, presence: Presence) {
    let header = ResponseHeader {
        wire: S::NAME.to_owned(),
        ty: header_schema::<S::Value>(&mut cx.types, Direction::Output, S::NAME),
        presence,
        ext: Default::default(),
    };
    cx.responses[start..]
        .iter_mut()
        .for_each(|response| response.headers.push(header.clone()));
}

impl<S, R> IntoResponse for WithHeader<S, R>
where
    S: HeaderSpec,
    S::Value: ToString,
    R: StatusResponse,
{
    fn into_response(self) -> axum::response::Response {
        let mut response = self.0.into_response();
        if insert_response_header::<S>(&mut response, &self.1).is_err() {
            return invalid_response_header();
        }
        response
    }
}

impl<S, R> DescribeResponse for WithHeader<S, R>
where
    S: HeaderSpec,
    R: DescribeResponse,
{
    fn describe(cx: &mut OperationCx) {
        let start = cx.responses.len();
        R::describe(cx);
        describe_response_header::<S>(cx, start, Presence::Required);
    }
}

impl<S, R> StatusResponse for WithHeader<S, R>
where
    S: HeaderSpec,
    S::Value: ToString,
    R: StatusResponse,
{
}

impl<S, R> IntoResponse for WithOptionalHeader<S, R>
where
    S: HeaderSpec,
    S::Value: ToString,
    R: StatusResponse,
{
    fn into_response(self) -> axum::response::Response {
        let mut response = self.0.into_response();
        if let Some(value) = self.1
            && insert_response_header::<S>(&mut response, &value).is_err()
        {
            return invalid_response_header();
        }
        response
    }
}

impl<S, R> DescribeResponse for WithOptionalHeader<S, R>
where
    S: HeaderSpec,
    R: DescribeResponse,
{
    fn describe(cx: &mut OperationCx) {
        let start = cx.responses.len();
        R::describe(cx);
        describe_response_header::<S>(cx, start, Presence::Optional);
    }
}

impl<S, R> StatusResponse for WithOptionalHeader<S, R>
where
    S: HeaderSpec,
    S::Value: ToString,
    R: StatusResponse,
{
}

pub struct WithStatus<const STATUS: u16, R>(pub R);

impl<const STATUS: u16, R> WithStatus<STATUS, R> {
    pub fn new(body: R) -> Self {
        Self(body)
    }
}

impl<const STATUS: u16, R> IntoResponse for WithStatus<STATUS, R>
where
    R: StatusResponse,
{
    fn into_response(self) -> axum::response::Response {
        let status = valid_status(STATUS, "WithStatus");
        let mut response = self.0.into_response();
        if !is_invalid_response_header(&response) {
            *response.status_mut() = axum::http::StatusCode::from_u16(status)
                .expect("WithStatus requires a valid HTTP status");
        }
        response
    }
}

impl<const STATUS: u16, R> DescribeResponse for WithStatus<STATUS, R>
where
    R: StatusResponse,
{
    fn describe(cx: &mut OperationCx) {
        let status = valid_status(STATUS, "WithStatus");
        let first_response = cx.responses.len();
        R::describe(cx);
        for response in &mut cx.responses[first_response..] {
            response.status = ResponseStatus::Exact(status);
        }
    }
}

pub struct NoContent;

impl IntoResponse for NoContent {
    fn into_response(self) -> axum::response::Response {
        axum::http::StatusCode::NO_CONTENT.into_response()
    }
}

impl DescribeResponse for NoContent {
    fn describe(cx: &mut OperationCx) {
        cx.empty_response(204);
    }
}

impl StatusResponse for NoContent {}

impl DescribeResponse for axum::response::Response {
    fn describe(cx: &mut OperationCx) {
        cx.diagnostic(
            "PONS-E102",
            "raw Axum responses expose neither status nor body; use a typed response wrapper",
        );
    }
}

impl<R> DescribeResponse for (axum::http::StatusCode, R)
where
    R: DescribeResponse,
{
    fn describe(cx: &mut OperationCx) {
        let start = cx.responses.len();
        R::describe(cx);
        for response in &mut cx.responses[start..] {
            response.status = ResponseStatus::Unknown;
        }
        cx.diagnostic(
            "PONS-W101",
            "a (StatusCode, R) response publishes the unknown status, a catch-all the status the handler writes is never checked against; wrap the body in WithResponse to publish a status the framework writes itself",
        );
    }
}

impl DescribeResponse for () {
    fn describe(cx: &mut OperationCx) {
        cx.empty_response(200);
    }
}

impl<T: Schema> DescribeResponse for axum::Json<T> {
    fn describe(cx: &mut OperationCx) {
        cx.response::<T>(200, ContentKind::Json);
    }
}

impl DescribeResponse for String {
    fn describe(cx: &mut OperationCx) {
        cx.response::<String>(200, ContentKind::Text);
    }
}

impl<R, E> DescribeResponse for Result<R, E>
where
    R: DescribeResponse,
    E: axum::response::IntoResponse + ErrorSpec,
{
    fn describe(cx: &mut OperationCx) {
        R::describe(cx);
        let errors = E::errors(&mut cx.types);
        if !crate::error::enforces_declared_status::<E>() {
            let statuses = errors
                .iter()
                .map(|error| format!("{:?}", error.status))
                .collect::<Vec<_>>()
                .join(", ");
            cx.diagnostic(
                "PONS-W101",
                format!(
                    "a bare Result declares the error statuses it names ({statuses}) but nothing enforces them at runtime; wrap the error in WithErrorHeader to serve only a status its error type declares"
                ),
            );
        }
        cx.errors.extend(errors);
    }
}

impl<T> StatusResponse for axum::Json<T> where T: Schema + serde::Serialize {}

impl StatusResponse for String {}

impl StatusResponse for () {}

impl StatusResponse for axum::body::Bytes {}

impl<T, S> StatusResponse for crate::Sse<T, S>
where
    T: crate::SseEvent + Send + 'static,
    S: futures_util::Stream<Item = T> + Send + 'static,
{
}

impl DescribeResponse for axum::body::Bytes {
    fn describe(cx: &mut OperationCx) {
        cx.response::<ByteArray>(200, ContentKind::Bytes);
    }
}

impl DescribeResponse for ByteArray {
    fn describe(cx: &mut OperationCx) {
        cx.response::<Self>(200, ContentKind::Bytes);
    }
}

impl<T: crate::SseEvent, S> DescribeResponse for crate::Sse<T, S> {
    fn describe(cx: &mut OperationCx) {
        cx.sse::<T>(T::NAME);
    }
}

pub struct ByteArray;

impl Schema for ByteArray {
    fn schema(_: &mut TypeCx, _: Direction) -> TypeRef {
        TypeRef::Inline(InlineType::Bytes {
            encoding: pons_ir::ByteEncoding::Base64,
        })
    }
}
