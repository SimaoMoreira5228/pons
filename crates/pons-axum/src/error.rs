use axum::response::{IntoResponse, Response};
use pons_core::{ErrorSpec, Schema, TypeCx};
use pons_ir::{Direction, Doc, ErrorContent, ErrorResponse, ErrorStatus, Presence, ResponseHeader};

use crate::JsonResponse;
use crate::describe::{
    HeaderSpec, header_schema, insert_response_header, invalid_response_header,
    is_invalid_response_header,
};
use crate::response::WithResponse;

pub(crate) const REJECTED: u16 = 422;

#[derive(Debug, serde::Serialize, pons_core::Schema)]
#[serde(deny_unknown_fields)]
pub struct Violation {
    pub path: String,
    pub reason: String,
}

#[derive(Debug, serde::Serialize, pons_core::Schema)]
#[serde(deny_unknown_fields)]
pub struct ViolationReport {
    pub violations: Vec<Violation>,
}

impl ViolationReport {
    pub(crate) fn rejected(violations: Vec<pons_ir::Violation>) -> Self {
        Self {
            violations: violations
                .into_iter()
                .map(|violation| Violation {
                    path: violation.path().to_owned(),
                    reason: violation.reason(),
                })
                .collect(),
        }
    }
}

impl ErrorSpec for ViolationReport {
    fn errors(cx: &mut TypeCx) -> Vec<ErrorResponse> {
        vec![ErrorResponse {
            status: ErrorStatus::Exact(REJECTED),
            name: vec!["request".to_owned(), "rejected".to_owned()],
            content: ErrorContent::Json,
            media_type: None,
            body: Some(<Self as Schema>::schema(cx, Direction::Output)),
            headers: Vec::new(),
            retryable: Some(false),
            doc: Some(Doc {
                summary: Some(
                    "The request does not satisfy the published request contract.".to_owned(),
                ),
                description: None,
                deprecated: None,
                tags: Vec::new(),
            }),
            ext: Default::default(),
        }]
    }
}

impl IntoResponse for ViolationReport {
    fn into_response(self) -> Response {
        WithResponse::<Self, JsonResponse<REJECTED>>::new(axum::Json(self).into_response())
            .into_response()
    }
}

pub struct WithErrorHeader<S: HeaderSpec, E>(pub E, pub <S as HeaderSpec>::Value);

impl<S: HeaderSpec, E> WithErrorHeader<S, E> {
    pub fn new(error: E, value: S::Value) -> Self {
        Self(error, value)
    }
}

impl<S, E> IntoResponse for WithErrorHeader<S, E>
where
    S: HeaderSpec,
    S::Value: ToString,
    E: ErrorSpec + IntoResponse,
{
    fn into_response(self) -> Response {
        error_response::<S, E>(self.0, Some(&self.1))
    }
}

impl<S, E> ErrorSpec for WithErrorHeader<S, E>
where
    S: HeaderSpec,
    E: ErrorSpec,
{
    fn errors(cx: &mut TypeCx) -> Vec<ErrorResponse> {
        let mut errors = E::errors(cx);
        let header = response_header::<S>(cx, Presence::Required);
        errors
            .iter_mut()
            .for_each(|error| error.headers.push(header.clone()));
        errors
    }
}

pub struct WithOptionalErrorHeader<S: HeaderSpec, E>(pub E, pub Option<<S as HeaderSpec>::Value>);

impl<S: HeaderSpec, E> WithOptionalErrorHeader<S, E> {
    pub fn new(error: E, value: Option<S::Value>) -> Self {
        Self(error, value)
    }
}

impl<S, E> IntoResponse for WithOptionalErrorHeader<S, E>
where
    S: HeaderSpec,
    S::Value: ToString,
    E: ErrorSpec + IntoResponse,
{
    fn into_response(self) -> Response {
        error_response::<S, E>(self.0, self.1.as_ref())
    }
}

impl<S, E> ErrorSpec for WithOptionalErrorHeader<S, E>
where
    S: HeaderSpec,
    E: ErrorSpec,
{
    fn errors(cx: &mut TypeCx) -> Vec<ErrorResponse> {
        let mut errors = E::errors(cx);
        let header = response_header::<S>(cx, Presence::Optional);
        errors
            .iter_mut()
            .for_each(|error| error.headers.push(header.clone()));
        errors
    }
}

fn error_response<S, E>(error: E, value: Option<&S::Value>) -> Response
where
    S: HeaderSpec,
    S::Value: ToString,
    E: ErrorSpec + IntoResponse,
{
    let mut response = error.into_response();
    if is_invalid_response_header(&response) {
        return response;
    }
    if !serves_declared_status::<E>(&mut response) {
        return invalid_response_header();
    }
    if let Some(value) = value
        && insert_response_header::<S>(&mut response, value).is_err()
    {
        return invalid_response_header();
    }
    response
}

fn serves_declared_status<E: ErrorSpec>(response: &mut Response) -> bool {
    let mut cx = TypeCx::new();
    let declared: Vec<ErrorStatus> = E::errors(&mut cx)
        .iter()
        .map(|error| error.status.clone())
        .collect();
    if let [ErrorStatus::Exact(status)] = declared.as_slice()
        && let Ok(status) = axum::http::StatusCode::from_u16(*status)
    {
        *response.status_mut() = status;
        return true;
    }
    match declared.as_slice() {
        [] => true,
        _ => declared
            .iter()
            .any(|status| declares(status, response.status())),
    }
}

fn declares(status: &ErrorStatus, produced: axum::http::StatusCode) -> bool {
    match status {
        ErrorStatus::Exact(status) => *status == produced.as_u16(),
        ErrorStatus::Class(class) => produced.as_u16() / 100 == u16::from(*class),
        ErrorStatus::Default => true,
    }
}

fn response_header<S>(cx: &mut TypeCx, presence: Presence) -> ResponseHeader
where
    S: HeaderSpec,
{
    ResponseHeader {
        wire: S::NAME.to_owned(),
        ty: header_schema::<S::Value>(cx, Direction::Output, S::NAME),
        presence,
        ext: Default::default(),
    }
}
