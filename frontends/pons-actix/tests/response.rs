use std::collections::BTreeMap;

use actix_web::http::StatusCode;
use actix_web::test;
use actix_web::web::Json;
use pons_actix::{
    Info, JsonResponse, ProblemJsonResponse, ResponseSpec, Router, WithResponse, WithUnknownStatus,
    routing::post,
};
use pons_core::Schema;
use pons_ir::{ContentKind, ResponseStatus};
use serde::Serialize;

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
    WithResponse::new(actix_web::HttpResponse::Ok().json(User { id: 1 }))
}

async fn vendor_response() -> WithResponse<User, VendorJson> {
    WithResponse::new(actix_web::HttpResponse::Ok().json(User { id: 1 }))
}

async fn problem_response() -> WithResponse<User, ProblemJsonResponse<422>> {
    WithResponse::new(actix_web::HttpResponse::Ok().json(User { id: 1 }))
}

async fn unknown_response() -> WithUnknownStatus<Json<User>> {
    WithUnknownStatus::new(Json(User { id: 7 }))
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

#[actix_web::test]
async fn explicit_response_metadata_preserves_the_runtime_status() {
    let router = Router::<()>::new(info()).route("/created", post(created_response));
    let document = router.document();
    assert_eq!(
        document.operations[0].responses[0].status,
        ResponseStatus::Exact(201)
    );

    let app = test::init_service(router.into_actix().unwrap()).await;
    let response =
        test::call_service(&app, test::TestRequest::post().uri("/created").to_request()).await;
    assert_eq!(response.status(), StatusCode::CREATED);
    assert_eq!(
        response.headers().get("content-type").unwrap(),
        "application/json"
    );
}

#[actix_web::test]
async fn a_declared_media_type_replaces_the_inner_content_type() {
    let router = Router::<()>::new(info()).route("/vendor", post(vendor_response));
    let document = router.document();
    assert_eq!(
        document.operations[0].responses[0]
            .media
            .as_ref()
            .map(|media| media.to_string())
            .as_deref(),
        Some("application/vnd.acme+json")
    );

    let app = test::init_service(router.into_actix().unwrap()).await;
    let response =
        test::call_service(&app, test::TestRequest::post().uri("/vendor").to_request()).await;
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    assert_eq!(
        response.headers().get("content-type").unwrap(),
        "application/vnd.acme+json"
    );
}

#[actix_web::test]
async fn problem_json_keeps_its_declared_media_type() {
    let router = Router::<()>::new(info()).route("/problem", post(problem_response));
    let app = test::init_service(router.into_actix().unwrap()).await;
    let response =
        test::call_service(&app, test::TestRequest::post().uri("/problem").to_request()).await;
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        response.headers().get("content-type").unwrap(),
        "application/problem+json"
    );
}

#[actix_web::test]
async fn an_unknown_status_is_published_and_left_to_the_handler() {
    let router = Router::<()>::new(info()).route("/unknown", post(unknown_response));
    let document = router.document();
    assert_eq!(
        document.operations[0].responses[0].status,
        ResponseStatus::Unknown
    );

    let app = test::init_service(router.into_actix().unwrap()).await;
    let response =
        test::call_service(&app, test::TestRequest::post().uri("/unknown").to_request()).await;
    assert_eq!(response.status(), StatusCode::OK);
}
