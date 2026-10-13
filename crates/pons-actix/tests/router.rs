use std::collections::BTreeMap;

use actix_web::ResponseError;
use actix_web::http::StatusCode;
use actix_web::test;
use actix_web::web;
use pons_actix::{
    HeaderSpec, Info, Json, NoContent, Router, WithHeader, WithStatus,
    routing::{get, post},
};
use pons_core::{ApiError, Schema};
use serde::{Deserialize, Serialize};

#[derive(Deserialize, Schema)]
struct UserPath {
    id: u64,
}

#[derive(Serialize, Schema)]
struct User {
    id: u64,
    name: String,
}

#[derive(Deserialize, Schema)]
#[serde(deny_unknown_fields)]
struct CreateUser {
    name: String,
}

#[derive(Deserialize, Schema)]
#[serde(deny_unknown_fields)]
struct UserQuery {
    limit: Option<u16>,
}

struct TraceId;

impl HeaderSpec for TraceId {
    type Value = String;
    const NAME: &'static str = "X-Trace-ID";
}

async fn path_user(pons_actix::Path(path): pons_actix::Path<UserPath>) -> Json<User> {
    Json(User {
        id: path.id,
        name: "Ada".into(),
    })
}

async fn query_users(pons_actix::Query(query): pons_actix::Query<UserQuery>) -> Json<Vec<User>> {
    let _ = query.limit;
    Json(vec![User {
        id: 1,
        name: "Ada".into(),
    }])
}

async fn create_user(Json(input): Json<CreateUser>) -> WithStatus<201, Json<User>> {
    WithStatus::new(Json(User {
        id: 1,
        name: input.name,
    }))
}

async fn form_user(pons_actix::Form(input): pons_actix::Form<CreateUser>) -> Json<User> {
    Json(User {
        id: 1,
        name: input.name,
    })
}

async fn response_headers() -> WithStatus<201, WithHeader<TraceId, Json<User>>> {
    WithStatus::new(WithHeader::new(
        Json(User {
            id: 1,
            name: "Ada".into(),
        }),
        "trace-123".into(),
    ))
}

async fn delete_user() -> NoContent {
    NoContent
}

#[derive(Debug, Serialize, Schema, ApiError)]
#[serde(rename_all = "snake_case")]
enum UserError {
    #[pons(status = 404)]
    Missing,
}

impl std::fmt::Display for UserError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("missing")
    }
}

impl ResponseError for UserError {
    fn status_code(&self) -> StatusCode {
        StatusCode::NOT_FOUND
    }

    fn error_response(&self) -> actix_web::HttpResponse<actix_web::body::BoxBody> {
        actix_web::HttpResponse::NotFound().finish()
    }
}

async fn typed_user() -> Result<Json<User>, UserError> {
    Err(UserError::Missing)
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
async fn router_description_records_paths_queries_and_bodies() {
    let router = Router::<()>::new(info())
        .route("/users/{id}", get(path_user))
        .route("/users", post(create_user))
        .route("/users", get(query_users));
    let document = router
        .into_document()
        .expect("the router describes a contract");
    assert_eq!(document.operations.len(), 3);
    let create = document
        .operations
        .iter()
        .find(|operation| operation.method == pons_ir::Method::Post)
        .unwrap();
    assert!(create.body.is_some());
    assert_eq!(
        create.responses[0].status,
        pons_ir::ResponseStatus::Exact(201)
    );
    let path = document
        .operations
        .iter()
        .find(|operation| {
            operation
                .path
                .iter()
                .any(|segment| matches!(segment, pons_ir::PathSegment::Param { param, .. } if param == "id"))
        })
        .unwrap();
    assert!(path.query.is_none());
}

#[actix_web::test]
async fn invalid_json_bodies_answer_the_declared_422() {
    let app = test::init_service(
        Router::<()>::new(info())
            .route("/users", post(create_user))
            .into_actix()
            .expect("the router validates"),
    )
    .await;
    let request = test::TestRequest::post()
        .uri("/users")
        .insert_header(("content-type", "application/json"))
        .set_payload(r#"{}"#)
        .to_request();
    let response = test::call_service(&app, request).await;
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let body = test::read_body(response).await;
    let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(body["violations"][0]["path"], "/name");
}

#[actix_web::test]
async fn invalid_queries_answer_the_declared_422() {
    let app = test::init_service(
        Router::<()>::new(info())
            .route("/users", get(query_users))
            .into_actix()
            .expect("the router validates"),
    )
    .await;
    let request = test::TestRequest::get()
        .uri("/users?limit=not-a-number")
        .to_request();
    let response = test::call_service(&app, request).await;
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

#[actix_web::test]
async fn invalid_paths_answer_the_declared_422() {
    let app = test::init_service(
        Router::<()>::new(info())
            .route("/users/{id}", get(path_user))
            .into_actix()
            .expect("the router validates"),
    )
    .await;
    let request = test::TestRequest::get()
        .uri("/users/not-a-number")
        .to_request();
    let response = test::call_service(&app, request).await;
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

#[actix_web::test]
async fn regex_path_parameters_keep_the_parameter_name() {
    let router = Router::<()>::new(info()).route("/users/{id:[0-9]+}", get(path_user));
    let document = router
        .into_document()
        .expect("the regex path parameter describes a contract");
    let parameter = document.operations[0]
        .path
        .iter()
        .find_map(|segment| match segment {
            pons_ir::PathSegment::Param { param, ty } => Some((param, ty)),
            _ => None,
        })
        .expect("the operation publishes its path parameter");
    assert_eq!(parameter.0, "id");
    assert!(matches!(
        parameter.1,
        pons_ir::TypeRef::Inline(pons_ir::InlineType::Int {
            bits: 64,
            signed: false,
            ..
        })
    ));
}

#[actix_web::test]
async fn nested_regex_quantifiers_keep_the_parameter_name() {
    let router = Router::<()>::new(info()).route("/users/{id:[0-9]{2}}", get(path_user));
    let document = router
        .into_document()
        .expect("the nested regex path parameter describes a contract");
    assert!(document.operations[0].path.iter().any(|segment| {
        matches!(segment, pons_ir::PathSegment::Param { param, .. } if param == "id")
    }));
}

#[actix_web::test]
async fn mixed_literal_parameter_segments_are_refused() {
    let router = Router::<()>::new(info()).route("/files/{id}.json", get(path_user));
    assert!(router.into_document().is_err());
}

#[actix_web::test]
async fn tail_path_parameters_are_refused() {
    let router = Router::<()>::new(info()).route("/files/{tail}*", get(path_user));
    assert!(router.into_document().is_err());
}

#[actix_web::test]
async fn invalid_forms_answer_the_declared_422() {
    let app = test::init_service(
        Router::<()>::new(info())
            .route("/users", post(form_user))
            .into_actix()
            .expect("the router validates"),
    )
    .await;
    let request = test::TestRequest::post()
        .uri("/users")
        .insert_header(("content-type", "application/x-www-form-urlencoded"))
        .set_payload("other=x")
        .to_request();
    let response = test::call_service(&app, request).await;
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

#[actix_web::test]
async fn status_and_response_headers_reach_the_wire() {
    let router = Router::<()>::new(info()).route("/users", post(response_headers));
    let document = router.document();
    let response = &document.operations[0].responses[0];
    assert_eq!(response.status, pons_ir::ResponseStatus::Exact(201));
    assert_eq!(response.headers[0].wire, "X-Trace-ID");

    let app = test::init_service(router.into_actix().unwrap()).await;
    let request = test::TestRequest::post().uri("/users").to_request();
    let response = test::call_service(&app, request).await;
    assert_eq!(response.status(), StatusCode::CREATED);
    assert_eq!(response.headers().get("x-trace-id").unwrap(), "trace-123");
}

#[actix_web::test]
async fn typed_errors_publish_their_status_and_answer_it() {
    let router = Router::<()>::new(info()).route("/users/me", get(typed_user));
    let document = router.document();
    assert_eq!(
        document.operations[0].errors[0].status,
        pons_ir::ErrorStatus::Exact(404)
    );

    let app = test::init_service(router.into_actix().unwrap()).await;
    let request = test::TestRequest::get().uri("/users/me").to_request();
    let response = test::call_service(&app, request).await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[actix_web::test]
async fn no_content_publishes_and_serves_204() {
    let router =
        Router::<()>::new(info()).route("/users/{id}", pons_actix::routing::delete(delete_user));
    let document = router.document();
    assert_eq!(
        document.operations[0].responses[0].status,
        pons_ir::ResponseStatus::Exact(204)
    );
    assert_eq!(
        document.operations[0].responses[0].content,
        pons_ir::ContentKind::Empty
    );

    let app = test::init_service(router.into_actix().unwrap()).await;
    let request = test::TestRequest::delete().uri("/users/1").to_request();
    let response = test::call_service(&app, request).await;
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
}

async fn raw_response() -> actix_web::HttpResponse {
    actix_web::HttpResponse::Ok().finish()
}

#[actix_web::test]
async fn raw_responses_are_refused_instead_of_described() {
    let router = Router::<()>::new(info()).route("/raw", get(raw_response));
    assert!(router.into_document().is_err());
}

async fn raw_json(_: web::Json<CreateUser>) -> Json<User> {
    Json(User {
        id: 1,
        name: "Ada".into(),
    })
}

#[actix_web::test]
async fn raw_actix_extractors_are_refused_instead_of_described() {
    let router = Router::<()>::new(info()).route("/raw", post(raw_json));
    assert!(router.into_document().is_err());
}

#[actix_web::test]
async fn routes_with_the_same_path_keep_each_method() {
    let app = test::init_service(
        Router::<()>::new(info())
            .route("/users", get(query_users))
            .route("/users", post(create_user))
            .into_actix()
            .unwrap(),
    )
    .await;
    let response = test::call_service(
        &app,
        test::TestRequest::get().uri("/users?limit=1").to_request(),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let response = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/users")
            .insert_header(("content-type", "application/json"))
            .set_payload(r#"{"name":"Ada"}"#)
            .to_request(),
    )
    .await;
    assert_eq!(response.status(), StatusCode::CREATED);
}

#[actix_web::test]
async fn try_lower_document_applies_the_lowering_and_checks_the_result() {
    let document = Router::<()>::new(info())
        .route("/users/{id}", pons_actix::routing::delete(delete_user))
        .try_lower_document(|mut document| {
            document
                .ext
                .insert("com.example.ticket/v1".into(), "PONS-1".into());
            Ok(document)
        })
        .expect("the lowered document validates");
    assert_eq!(document.ext["com.example.ticket/v1"], "PONS-1");
}

struct RequestId;

impl HeaderSpec for RequestId {
    type Value = String;
    const NAME: &'static str = "X-Request-ID";
}

async fn header_user(pons_actix::Header(request_id): pons_actix::Header<RequestId>) -> Json<User> {
    let _ = request_id;
    Json(User {
        id: 1,
        name: "Ada".into(),
    })
}

#[actix_web::test]
async fn headers_are_validated_against_the_request_contract() {
    let router = Router::<()>::new(info()).route("/users/me", get(header_user));
    let document = router.document();
    assert_eq!(document.operations[0].headers[0].wire, "X-Request-ID");
    assert_eq!(
        document.operations[0].headers[0].presence,
        pons_ir::Presence::Required
    );

    let app = test::init_service(router.into_actix().unwrap()).await;
    let missing =
        test::call_service(&app, test::TestRequest::get().uri("/users/me").to_request()).await;
    assert_eq!(missing.status(), StatusCode::UNPROCESSABLE_ENTITY);

    let present = test::call_service(
        &app,
        test::TestRequest::get()
            .uri("/users/me")
            .insert_header(("x-request-id", "req-1"))
            .to_request(),
    )
    .await;
    assert_eq!(present.status(), StatusCode::OK);
}

struct Retry;

impl HeaderSpec for Retry {
    type Value = u16;
    const NAME: &'static str = "X-Retry-After";
}

async fn enforced_error() -> Result<Json<User>, pons_actix::WithErrorHeader<Retry, UserError>> {
    Err(pons_actix::WithErrorHeader::new(UserError::Missing, 3))
}

#[actix_web::test]
async fn error_headers_are_published_and_set_on_the_error_response() {
    let router = Router::<()>::new(info()).route("/users/me", get(enforced_error));
    let document = router.document();
    let error = &document.operations[0].errors[0];
    assert_eq!(error.status, pons_ir::ErrorStatus::Exact(404));
    assert_eq!(error.headers[0].wire, "X-Retry-After");

    let app = test::init_service(router.into_actix().unwrap()).await;
    let response =
        test::call_service(&app, test::TestRequest::get().uri("/users/me").to_request()).await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    assert_eq!(response.headers().get("x-retry-after").unwrap(), "3");
}
