use actix_web::body::BoxBody;
use actix_web::http::StatusCode;
use actix_web::{HttpRequest, HttpResponse, Responder, ResponseError};
use pons_core::{ErrorSpec, Schema, TypeCx};
use pons_ir::{
    Direction, Doc, ErrorContent, ErrorResponse, ErrorStatus, Presence, ResponseHeader, TypeRef,
};

use crate::describe::{
    HeaderSpec, header_schema, insert_response_header, invalid_response_header,
    is_invalid_response_header,
};

pub(crate) const REJECTED: u16 = 422;
pub(crate) const REJECTED_NAME: [&str; 2] = ["request", "rejected"];
pub(crate) const DEGRADED: u16 = 500;
pub(crate) const DEGRADED_NAME: [&str; 3] = ["response", "header", "invalid"];

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
        vec![Self::error(Some(<Self as Schema>::schema(
            cx,
            Direction::Output,
        )))]
    }
}

impl ViolationReport {
    pub(crate) fn error(body: Option<TypeRef>) -> ErrorResponse {
        ErrorResponse {
            status: ErrorStatus::Exact(REJECTED),
            name: REJECTED_NAME
                .iter()
                .map(|word| (*word).to_owned())
                .collect(),
            content: match &body {
                Some(_) => ErrorContent::Json,
                None => ErrorContent::Empty,
            },
            media_type: None,
            body,
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
        }
    }
}

impl std::fmt::Display for ViolationReport {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("the request does not satisfy the published request contract")
    }
}

impl ResponseError for ViolationReport {
    fn status_code(&self) -> StatusCode {
        StatusCode::UNPROCESSABLE_ENTITY
    }

    fn error_response(&self) -> HttpResponse<BoxBody> {
        HttpResponse::UnprocessableEntity().json(self)
    }
}

pub(crate) fn degraded_error() -> ErrorResponse {
    ErrorResponse {
        status: ErrorStatus::Exact(DEGRADED),
        name: DEGRADED_NAME.iter().map(|word| (*word).to_owned()).collect(),
        content: ErrorContent::Empty,
        media_type: None,
        body: None,
        headers: Vec::new(),
        retryable: Some(false),
        doc: Some(Doc {
            summary: Some(
                "A response header the handler builds from the request is not a header value, so the framework answers 500 with a plain text body this contract does not describe."
                    .to_owned(),
            ),
            description: None,
            deprecated: None,
            tags: Vec::new(),
        }),
        ext: Default::default(),
    }
}

const ENFORCING_WRAPPERS: [&str; 2] = ["WithErrorHeader", "WithOptionalErrorHeader"];

pub(crate) fn enforces_declared_status<E>() -> bool {
    enforces_declared_status_named(std::any::type_name::<E>())
}

fn enforces_declared_status_named(name: &str) -> bool {
    let path = name.split('<').next().unwrap_or(name);
    let wrapper = path.rsplit("::").next().unwrap_or(path);
    ENFORCING_WRAPPERS.contains(&wrapper)
}

pub struct WithErrorHeader<S: HeaderSpec, E>(pub E, pub <S as HeaderSpec>::Value);

impl<S: HeaderSpec, E> WithErrorHeader<S, E> {
    pub fn new(error: E, value: S::Value) -> Self {
        Self(error, value)
    }
}

impl<S: HeaderSpec, E: std::fmt::Debug> std::fmt::Debug for WithErrorHeader<S, E> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_tuple("WithErrorHeader")
            .field(&self.0)
            .finish()
    }
}

impl<S: HeaderSpec, E: std::fmt::Display> std::fmt::Display for WithErrorHeader<S, E> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(formatter)
    }
}

impl<S, E> ResponseError for WithErrorHeader<S, E>
where
    S: HeaderSpec,
    S::Value: ToString,
    E: ErrorSpec + ResponseError,
{
    fn status_code(&self) -> StatusCode {
        served_status::<E>()
    }

    fn error_response(&self) -> HttpResponse<BoxBody> {
        error_response::<S, E>(&self.0, Some(&self.1))
    }
}

impl<S, E> Responder for WithErrorHeader<S, E>
where
    S: HeaderSpec,
    S::Value: ToString,
    E: ErrorSpec + ResponseError,
{
    type Body = BoxBody;

    fn respond_to(self, _: &HttpRequest) -> HttpResponse<BoxBody> {
        self.error_response()
    }
}

impl<S, E> ErrorSpec for WithErrorHeader<S, E>
where
    S: HeaderSpec,
    E: ErrorSpec,
{
    fn errors(cx: &mut TypeCx) -> Vec<ErrorResponse> {
        let mut errors = E::errors(cx);
        let header = error_header::<S>(cx, Presence::Required);
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

impl<S: HeaderSpec, E: std::fmt::Debug> std::fmt::Debug for WithOptionalErrorHeader<S, E> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_tuple("WithOptionalErrorHeader")
            .field(&self.0)
            .finish()
    }
}

impl<S: HeaderSpec, E: std::fmt::Display> std::fmt::Display for WithOptionalErrorHeader<S, E> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(formatter)
    }
}

impl<S, E> ResponseError for WithOptionalErrorHeader<S, E>
where
    S: HeaderSpec,
    S::Value: ToString,
    E: ErrorSpec + ResponseError,
{
    fn status_code(&self) -> StatusCode {
        served_status::<E>()
    }

    fn error_response(&self) -> HttpResponse<BoxBody> {
        error_response::<S, E>(&self.0, self.1.as_ref())
    }
}

impl<S, E> Responder for WithOptionalErrorHeader<S, E>
where
    S: HeaderSpec,
    S::Value: ToString,
    E: ErrorSpec + ResponseError,
{
    type Body = BoxBody;

    fn respond_to(self, _: &HttpRequest) -> HttpResponse<BoxBody> {
        self.error_response()
    }
}

impl<S, E> ErrorSpec for WithOptionalErrorHeader<S, E>
where
    S: HeaderSpec,
    E: ErrorSpec,
{
    fn errors(cx: &mut TypeCx) -> Vec<ErrorResponse> {
        let mut errors = E::errors(cx);
        let header = error_header::<S>(cx, Presence::Optional);
        errors
            .iter_mut()
            .for_each(|error| error.headers.push(header.clone()));
        errors
    }
}

fn error_response<S, E>(error: &E, value: Option<&S::Value>) -> HttpResponse<BoxBody>
where
    S: HeaderSpec,
    S::Value: ToString,
    E: ErrorSpec + ResponseError,
{
    let mut response = error.error_response();
    if is_invalid_response_header(&response) {
        return response;
    }
    if !serves_declared_status::<E>(&mut response) {
        return invalid_response_header();
    }
    if let Some(value) = value
        && insert_response_header::<BoxBody, S>(&mut response, value).is_err()
    {
        return invalid_response_header();
    }
    response
}

fn served_status<E: ErrorSpec>() -> StatusCode {
    let mut cx = TypeCx::new();
    match E::errors(&mut cx).as_slice() {
        [error] => match error.status {
            ErrorStatus::Exact(status) => {
                StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR)
            }
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        },
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

fn serves_declared_status<E: ErrorSpec>(response: &mut HttpResponse<BoxBody>) -> bool {
    let mut cx = TypeCx::new();
    let declared: Vec<ErrorStatus> = E::errors(&mut cx)
        .iter()
        .map(|error| error.status.clone())
        .collect();
    if let [ErrorStatus::Exact(status)] = declared.as_slice()
        && let Ok(status) = StatusCode::from_u16(*status)
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

fn declares(status: &ErrorStatus, produced: StatusCode) -> bool {
    match status {
        ErrorStatus::Exact(status) => *status == produced.as_u16(),
        ErrorStatus::Class(class) => produced.as_u16() / 100 == u16::from(*class),
        ErrorStatus::Default => true,
    }
}

pub fn error_header<S>(cx: &mut TypeCx, presence: Presence) -> ResponseHeader
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

#[cfg(test)]
mod tests {
    use super::*;

    struct TraceId;

    impl HeaderSpec for TraceId {
        type Value = String;
        const NAME: &'static str = "X-Trace-ID";
    }

    #[derive(Debug, serde::Serialize, Schema, pons_core::ApiError)]
    enum Conflict {
        #[allow(dead_code)]
        #[pons(status = 409)]
        Conflict,
    }

    #[test]
    fn only_the_wrappers_are_named_as_enforcing_their_error_status() {
        assert!(enforces_declared_status::<WithErrorHeader<TraceId, Conflict>>());
        assert!(enforces_declared_status::<
            WithOptionalErrorHeader<TraceId, Conflict>,
        >());
        assert!(
            !enforces_declared_status::<Conflict>(),
            "a bare error type writes whatever status its ResponseError writes"
        );
    }

    #[test]
    fn a_renamed_dependency_still_names_the_wrappers_that_enforce() {
        for wrapper in ["WithErrorHeader", "WithOptionalErrorHeader"] {
            for crate_name in ["pons_actix", "web_actix", "actix_pons"] {
                assert!(
                    enforces_declared_status_named(&format!(
                        "{crate_name}::error::{wrapper}<app::TraceId, app::Conflict>"
                    )),
                    "a dependency renamed to {crate_name} still wraps the status in {wrapper}"
                );
            }
        }
        for bare in ["app::errors::Conflict", "Conflict", "app::WithErrors"] {
            assert!(!enforces_declared_status_named(bare));
        }
    }
}
