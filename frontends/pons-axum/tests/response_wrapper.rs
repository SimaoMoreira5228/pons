mod support;

use axum::response::IntoResponse;
use pons_axum::http::StatusCode;
use pons_axum::routing::post;
use pons_axum::{
    HeaderSpec, Info, JsonResponse, MediaType, ProblemJsonResponse, ResponseSpec, Router,
    WithHeader, WithMedia, WithResponse, WithUnknownStatus,
};
use pons_core::Schema;
use pons_ir::{ContentKind, ResponseStatus, TypeRef};
use serde::Serialize;
use std::collections::BTreeMap;
use tower_service::Service;

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

struct TraceId;

impl HeaderSpec for TraceId {
    type Value = String;
    const NAME: &'static str = "X-Trace-ID";
}

struct UndeclaredMediaType;

impl ResponseSpec for UndeclaredMediaType {
    fn status() -> u16 {
        200
    }

    fn content() -> ContentKind {
        ContentKind::Json
    }

    fn media_type() -> Option<&'static str> {
        Some("application/json\n")
    }
}

struct VendorReport;

impl MediaType for VendorReport {
    const VALUE: &'static str = "application/vnd.acme.report.v1+json; version=1";
}

struct GenericProblemJson;

impl MediaType for GenericProblemJson {
    const VALUE: &'static str = "application/problem+json";
}

struct BrokenMediaType;

impl MediaType for BrokenMediaType {
    const VALUE: &'static str = "application/json\n";
}

async fn created_response() -> WithResponse<User, JsonResponse<201>> {
    WithResponse::new(StatusCode::OK.into_response())
}

async fn vendor_response() -> WithResponse<User, VendorJson> {
    WithResponse::new(StatusCode::OK.into_response())
}

async fn report_response() -> WithMedia<User, JsonResponse<200>, VendorReport> {
    WithResponse::new(axum::Json(User { id: 9 }).into_response()).media::<VendorReport>()
}

async fn generic_problem_response() -> WithMedia<User, JsonResponse<200>, GenericProblemJson> {
    WithResponse::new(StatusCode::OK.into_response()).media::<GenericProblemJson>()
}

async fn broken_media_response() -> WithMedia<User, JsonResponse<200>, BrokenMediaType> {
    WithResponse::new(StatusCode::OK.into_response()).media::<BrokenMediaType>()
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
        document.operations[0].responses[0]
            .media
            .as_ref()
            .map(|media| media.to_string())
            .as_deref(),
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

#[test]
fn declared_media_type_replaces_the_inner_content_type() {
    let problem = WithResponse::<User, ProblemJsonResponse<422>>::new(
        axum::Json(User { id: 3 }).into_response(),
    )
    .into_response();
    assert_eq!(problem.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        problem.headers()["content-type"],
        "application/problem+json"
    );
}

#[test]
fn invalid_header_responses_keep_their_status() {
    let response = WithResponse::<User, JsonResponse<201>>::new(
        WithHeader::<TraceId, _>::new(axum::Json(User { id: 4 }), "bad\nvalue".into())
            .into_response(),
    )
    .into_response();
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        response.headers()["content-type"],
        "text/plain; charset=utf-8"
    );
}

#[test]
fn an_invalid_declared_media_type_degrades_instead_of_panicking() {
    let response = WithResponse::<User, UndeclaredMediaType>::new(StatusCode::OK.into_response())
        .into_response();
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
}

#[test]
fn a_declared_media_type_is_served_and_published() {
    let document = Router::<()>::new(info())
        .route("/report", post(report_response))
        .into_document()
        .unwrap();
    let response = &document.operations[0].responses[0];
    assert_eq!(response.content, ContentKind::Json);
    assert!(matches!(response.body, Some(TypeRef::Named(_))));
    let media = response
        .media
        .as_ref()
        .expect("the media type is published");
    assert_eq!(media.media_type, "application");
    assert_eq!(media.subtype, "vnd.acme.report.v1+json");
    assert_eq!(
        media.parameters.get("version").map(String::as_str),
        Some("1")
    );

    let mut app = support::built(Router::<()>::new(info()).route("/report", post(report_response)));
    let request = pons_axum::http::Request::builder()
        .method("POST")
        .uri("/report")
        .body(axum::body::Body::empty())
        .unwrap();
    let response = support::block_on(app.call(request)).unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers()["content-type"],
        "application/vnd.acme.report.v1+json; version=1"
    );
}

#[test]
fn an_invalid_declared_media_type_fails_the_description() {
    let errors = Router::<()>::new(info())
        .route("/broken", post(broken_media_response))
        .into_document()
        .unwrap_err();
    let message = errors
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(message.contains("PONS-E102"), "{message}");
    assert!(message.contains("not a valid media type"), "{message}");
}

#[test]
fn problem_json_media_on_a_json_response_is_refused() {
    let errors = Router::<()>::new(info())
        .route("/problem", post(generic_problem_response))
        .into_document()
        .unwrap_err();
    let message = errors
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(message.contains("PONS-E102"), "{message}");
    assert!(message.contains("ProblemJsonResponse"), "{message}");
}
