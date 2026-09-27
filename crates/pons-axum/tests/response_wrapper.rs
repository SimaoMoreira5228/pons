use axum::response::IntoResponse;
use pons_axum::http::StatusCode;
use pons_axum::routing::post;
use pons_axum::{
    Info, JsonResponse, ProblemJsonResponse, ResponseSpec, Router, WithResponse, WithUnknownStatus,
};
use pons_core::Schema;
use pons_ir::{ContentKind, ResponseStatus, TypeRef};
use serde::Serialize;
use std::collections::BTreeMap;

#[derive(Serialize, Schema)]
struct User {
    id: u64,
}

struct VendorJson;

impl ResponseSpec for VendorJson {
    fn status() -> u16 {
        202
    }

    fn content() -> ContentKind {
        ContentKind::Json
    }

    fn media_type() -> Option<&'static str> {
        Some("application/vnd.acme+json")
    }
}

async fn created_response() -> WithResponse<User, JsonResponse<201>> {
    WithResponse::new(StatusCode::OK.into_response())
}

async fn vendor_response() -> WithResponse<User, VendorJson> {
    WithResponse::new(StatusCode::OK.into_response())
}

async fn unknown_response() -> WithUnknownStatus<axum::Json<User>> {
    WithUnknownStatus::new(axum::Json(User { id: 7 }))
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
fn explicit_response_metadata_preserves_runtime_status() {
    let response = WithResponse::<User, JsonResponse<201>>::new(StatusCode::OK.into_response())
        .into_response();
    assert_eq!(response.status(), StatusCode::CREATED);
    assert_eq!(response.headers()["content-type"], "application/json");
    let problem =
        WithResponse::<User, ProblemJsonResponse<422>>::new(StatusCode::OK.into_response())
            .into_response();
    assert_eq!(problem.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        problem.headers()["content-type"],
        "application/problem+json"
    );
    let document = Router::<()>::new(info())
        .route("/created", post(created_response))
        .into_document()
        .unwrap();
    let operation = &document.operations[0];
    assert_eq!(operation.responses[0].status, ResponseStatus::Exact(201));
    assert_eq!(operation.responses[0].content, ContentKind::Json);
    assert!(matches!(
        operation.responses[0].body,
        Some(TypeRef::Named(_))
    ));
    let vendor =
        WithResponse::<User, VendorJson>::new(StatusCode::OK.into_response()).into_response();
    assert_eq!(
        vendor.headers()["content-type"],
        "application/vnd.acme+json"
    );
    let document = Router::<()>::new(info())
        .route("/vendor", post(vendor_response))
        .into_document()
        .unwrap();
    assert_eq!(
        document.operations[0].responses[0].media_type.as_deref(),
        Some("application/vnd.acme+json")
    );
    let document = Router::<()>::new(info())
        .route("/unknown", post(unknown_response))
        .into_document()
        .unwrap();
    assert_eq!(
        document.operations[0].responses[0].status,
        ResponseStatus::Unknown
    );
    let response = WithUnknownStatus::new(StatusCode::CREATED.into_response()).into_response();
    assert_eq!(response.status(), StatusCode::CREATED);
}
