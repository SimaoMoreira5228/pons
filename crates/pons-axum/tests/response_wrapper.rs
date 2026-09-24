use axum::response::IntoResponse;
use pons_axum::http::StatusCode;
use pons_axum::routing::post;
use pons_axum::{Info, JsonResponse, ProblemJsonResponse, Router, WithResponse};
use pons_core::Schema;
use pons_ir::{ContentKind, TypeRef};
use serde::Serialize;
use std::collections::BTreeMap;

#[derive(Serialize, Schema)]
struct User {
    id: u64,
}

async fn created_response() -> WithResponse<User, JsonResponse<201>> {
    WithResponse::new(StatusCode::OK.into_response())
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
    assert_eq!(operation.responses[0].status, 201);
    assert_eq!(operation.responses[0].content, ContentKind::Json);
    assert!(matches!(
        operation.responses[0].body,
        Some(TypeRef::Named(_))
    ));
}
