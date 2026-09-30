use axum::response::IntoResponse;
use pons_axum::http::StatusCode;
use pons_axum::routing::post;
use pons_axum::{HeaderSpec, Info, Router, WithErrorHeader, WithOptionalErrorHeader};
use pons_core::Schema;
use pons_ir::{ContentKind, ErrorContent, Presence};
use serde::Serialize;
use std::collections::BTreeMap;

#[derive(Debug, Serialize, Schema)]
struct User {
    id: u64,
}

struct RetryAfter;

impl HeaderSpec for RetryAfter {
    type Value = u16;
    const NAME: &'static str = "Retry-After";
}

#[derive(Debug, Serialize, Schema, pons_core::ApiError)]
enum UserError {
    #[pons(status = 429)]
    Limited,
}

impl IntoResponse for UserError {
    fn into_response(self) -> axum::response::Response {
        StatusCode::TOO_MANY_REQUESTS.into_response()
    }
}

#[derive(Debug, Serialize, Schema, pons_core::ApiError)]
enum SlotError {
    #[pons(status = 404)]
    Missing,
    #[pons(status = 409)]
    Conflict,
    #[pons(status = "4xx")]
    Throttled,
    #[pons(status = "default")]
    Unexpected,
}

impl IntoResponse for SlotError {
    fn into_response(self) -> axum::response::Response {
        match self {
            Self::Missing => StatusCode::CONFLICT.into_response(),
            Self::Conflict => StatusCode::OK.into_response(),
            Self::Throttled => StatusCode::TOO_MANY_REQUESTS.into_response(),
            Self::Unexpected => StatusCode::SERVICE_UNAVAILABLE.into_response(),
        }
    }
}

#[derive(Debug, Serialize, Schema, pons_core::ApiError)]
enum OverloadedError {
    #[pons(status = 429)]
    Limited,
}

impl IntoResponse for OverloadedError {
    fn into_response(self) -> axum::response::Response {
        StatusCode::SERVICE_UNAVAILABLE.into_response()
    }
}

fn required_error() -> WithErrorHeader<RetryAfter, UserError> {
    WithErrorHeader::new(UserError::Limited, 3)
}

async fn required_header()
-> Result<pons_axum::extract::Json<User>, WithErrorHeader<RetryAfter, UserError>> {
    Err(required_error())
}

fn optional_error() -> WithOptionalErrorHeader<RetryAfter, UserError> {
    WithOptionalErrorHeader::new(UserError::Limited, Some(3))
}

async fn optional_header()
-> Result<pons_axum::extract::Json<User>, WithOptionalErrorHeader<RetryAfter, UserError>> {
    Err(optional_error())
}

fn info() -> Info {
    Info {
        title: "Users".into(),
        version: "1".into(),
        description: None,
        servers: Vec::new(),
        ext: BTreeMap::new(),
    }
}

#[test]
fn error_header_wrapper_preserves_runtime_and_contract_metadata() {
    let required = required_error().into_response();
    assert_eq!(required.status(), StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(required.headers()["Retry-After"], "3");

    let optional = optional_error().into_response();
    assert_eq!(optional.headers()["Retry-After"], "3");

    let document = Router::<()>::new(info())
        .route("/required", post(required_header))
        .route("/optional", post(optional_header))
        .into_document()
        .unwrap();
    let required_operation = document
        .operations
        .iter()
        .find(|operation| operation.path.iter().any(|segment| matches!(segment, pons_ir::PathSegment::Literal { lit } if lit == "required")))
        .unwrap();
    assert_eq!(required_operation.responses[0].content, ContentKind::Json);
    assert_eq!(required_operation.errors[0].content, ErrorContent::Json);
    assert_eq!(required_operation.errors[0].headers[0].wire, "Retry-After");
    assert_eq!(
        required_operation.errors[0].headers[0].presence,
        Presence::Required
    );

    let optional_operation = document
        .operations
        .iter()
        .find(|operation| operation.path.iter().any(|segment| matches!(segment, pons_ir::PathSegment::Literal { lit } if lit == "optional")))
        .unwrap();
    assert_eq!(
        optional_operation.errors[0].headers[0].presence,
        Presence::Optional
    );
}

#[test]
fn the_declared_error_status_owns_the_served_status() {
    let declared = WithErrorHeader::<RetryAfter, _>::new(SlotError::Missing, 1).into_response();
    assert_eq!(declared.status(), StatusCode::CONFLICT);
    assert!(!declared.headers().contains_key("content-type"));

    let undeclared = WithErrorHeader::<RetryAfter, _>::new(SlotError::Conflict, 1).into_response();
    assert_eq!(undeclared.status(), StatusCode::INTERNAL_SERVER_ERROR);

    let declared = WithErrorHeader::<RetryAfter, _>::new(SlotError::Throttled, 1).into_response();
    assert_eq!(declared.status(), StatusCode::TOO_MANY_REQUESTS);

    let declared = WithErrorHeader::<RetryAfter, _>::new(SlotError::Unexpected, 1).into_response();
    assert_eq!(declared.status(), StatusCode::SERVICE_UNAVAILABLE);

    let single = WithErrorHeader::<RetryAfter, _>::new(OverloadedError::Limited, 1).into_response();
    assert_eq!(single.status(), StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(single.headers()["Retry-After"], "1");
    assert!(!single.headers().contains_key("content-type"));
}
