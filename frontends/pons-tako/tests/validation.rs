use std::collections::BTreeMap;

use http::header::CONTENT_TYPE;
use http::{Method, StatusCode};
use http_body_util::BodyExt;
use pons_core::{Schema, TypeCx};
use pons_ir::{Direction, ErrorContent, ErrorStatus, InlineType, IntegerWire, TypeKind, TypeRef};
use pons_tako::{Form, Info, Json, Path, Query, Router};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tako::body::TakoBody;
use tako::extractors::json::Json as TakoJson;
use tako::types::Request;

#[derive(Deserialize)]
#[serde(transparent)]
struct Slug(pub String);

impl Schema for Slug {
    fn schema(_: &mut TypeCx, _: Direction) -> TypeRef {
        TypeRef::Inline(InlineType::String {
            format: None,
            min_length: Some(1),
            max_length: Some(16),
            pattern: Some("^[a-z][a-z0-9-]*$".into()),
        })
    }
}

#[derive(Deserialize)]
#[serde(transparent)]
struct Retries(pub u8);

impl Schema for Retries {
    fn schema(_: &mut TypeCx, _: Direction) -> TypeRef {
        TypeRef::Inline(InlineType::Int {
            bits: 8,
            signed: false,
            wire: IntegerWire::Number,
            minimum: Some(json!(1)),
            maximum: Some(json!(5)),
            multiple_of: Some(json!(1)),
        })
    }
}

#[derive(Deserialize, Schema)]
#[serde(deny_unknown_fields)]
struct CreateUser {
    slug: Slug,
}

#[derive(Deserialize, Schema)]
#[serde(deny_unknown_fields)]
struct Search {
    slug: Slug,
    #[serde(default)]
    limit: Option<Retries>,
}

#[derive(Deserialize, Schema)]
#[serde(deny_unknown_fields)]
struct UserPath {
    slug: Slug,
}

#[derive(Deserialize, Schema)]
#[serde(deny_unknown_fields)]
struct SlugForm {
    slug: Slug,
}

#[derive(Serialize, Schema)]
struct User {
    id: u64,
    name: String,
}

async fn create_user(Json(input): Json<CreateUser>) -> TakoJson<User> {
    TakoJson(User {
        id: input.slug.0.len() as u64,
        name: "Ada".into(),
    })
}

async fn search(Query(query): Query<Search>) -> TakoJson<User> {
    TakoJson(User {
        id: u64::from(query.limit.map_or(0, |limit| limit.0)),
        name: query.slug.0,
    })
}

async fn get_user(Path(path): Path<UserPath>) -> TakoJson<User> {
    TakoJson(User {
        id: path.slug.0.len() as u64,
        name: "Ada".into(),
    })
}

async fn create_form(Form(form): Form<SlugForm>) -> TakoJson<User> {
    TakoJson(User {
        id: form.slug.0.len() as u64,
        name: "Ada".into(),
    })
}

async fn plain_create(TakoJson(input): TakoJson<CreateUser>) -> TakoJson<User> {
    TakoJson(User {
        id: input.slug.0.len() as u64,
        name: "Ada".into(),
    })
}

fn info() -> Info {
    Info {
        title: "Validation".into(),
        version: "1".into(),
        description: None,
        servers: Vec::new(),
        ext: BTreeMap::new(),
    }
}

fn router() -> Router {
    let mut router = Router::new(info());
    router.post("/users", create_user);
    router.get("/search", search);
    router.get("/users/{slug}", get_user);
    router.post("/form", create_form);
    router
}

fn get(path: &str) -> Request {
    http::request::Builder::new()
        .method(Method::GET)
        .uri(path)
        .body(TakoBody::empty())
        .expect("the request is well formed")
}

fn with_body(path: &str, content_type: &str, body: &str) -> Request {
    http::request::Builder::new()
        .method(Method::POST)
        .uri(path)
        .header(CONTENT_TYPE, content_type)
        .body(TakoBody::from(body.to_owned()))
        .expect("the request is well formed")
}

async fn send(router: &tako::router::Router, request: Request) -> (StatusCode, Value) {
    let response = router.dispatch(request).await;
    let status = response.status();
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("the response body is readable")
        .to_bytes();
    let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, body)
}

fn json_body(text: &str) -> Request {
    with_body("/users", "application/json", text)
}

#[tokio::test]
async fn a_request_that_satisfies_the_contract_reaches_the_handler() {
    let router = router().into_router();

    let (status, body) = send(&router, json_body(r#"{"slug":"ada"}"#)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, json!({"id": 3, "name": "Ada"}));

    let (status, body) = send(&router, get("/search?slug=ada&limit=5")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, json!({"id": 5, "name": "ada"}));

    let (status, body) = send(&router, get("/users/ada")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, json!({"id": 3, "name": "Ada"}));

    let (status, body) = send(
        &router,
        with_body("/form", "application/x-www-form-urlencoded", "slug=ada"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, json!({"id": 3, "name": "Ada"}));
}

#[tokio::test]
async fn a_body_that_breaks_the_contract_is_rejected_with_the_report() {
    let (status, body) = send(
        &router().into_router(),
        json_body(r#"{"slug":"Ada Lovelace"}"#),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        body,
        json!({
            "violations": [{
                "path": "/slug",
                "reason": "string does not match the pattern ^[a-z][a-z0-9-]*$"
            }]
        })
    );
}

#[tokio::test]
async fn a_query_that_breaks_the_contract_is_rejected_with_the_report() {
    let router = router().into_router();

    let (status, body) = send(&router, get("/search?slug=ada&limit=9")).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        body,
        json!({
            "violations": [{
                "path": "/limit",
                "reason": "integer is above its maximum of 5"
            }]
        })
    );

    let (status, body) = send(&router, get("/search?limit=2")).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        body,
        json!({
            "violations": [{
                "path": "/slug",
                "reason": "missing required field slug"
            }]
        })
    );
}

#[tokio::test]
async fn a_path_that_breaks_the_contract_is_rejected_with_the_report() {
    let (status, body) = send(&router().into_router(), get("/users/Ada")).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        body,
        json!({
            "violations": [{
                "path": "/slug",
                "reason": "string does not match the pattern ^[a-z][a-z0-9-]*$"
            }]
        })
    );
}

#[tokio::test]
async fn a_form_that_breaks_the_contract_is_rejected_with_the_report() {
    let (status, body) = send(
        &router().into_router(),
        with_body(
            "/form",
            "application/x-www-form-urlencoded",
            "slug=Ada+Lovelace",
        ),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        body,
        json!({
            "violations": [{
                "path": "/slug",
                "reason": "string does not match the pattern ^[a-z][a-z0-9-]*$"
            }]
        })
    );
}

#[test]
fn a_validated_operation_publishes_the_rejection_and_its_report() {
    let document = router().into_document().expect("the document validates");
    assert_eq!(document.operations.len(), 4);

    for operation in &document.operations {
        let error = operation
            .errors
            .iter()
            .find(|error| {
                error
                    .name
                    .iter()
                    .map(String::as_str)
                    .eq(["request", "rejected"])
            })
            .unwrap_or_else(|| panic!("{:?} publishes no rejection", operation.id));
        assert_eq!(error.status, ErrorStatus::Exact(422));
        assert_eq!(error.content, ErrorContent::Json);
        assert_eq!(error.retryable, Some(false));

        let Some(TypeRef::Named(report)) = &error.body else {
            panic!("the rejection carries the report")
        };
        let report = document
            .types
            .get(&report.id)
            .expect("the report type is published");
        assert_eq!(report.name, ["violation", "report"]);

        let TypeKind::Struct { fields, .. } = &report.definition else {
            panic!("the report is a struct")
        };
        assert_eq!(fields.len(), 1);
        assert_eq!(fields[0].wire, "violations");

        let TypeRef::Inline(InlineType::Array { item, .. }) = &fields[0].ty else {
            panic!("the report holds an array of violations")
        };
        let TypeRef::Named(violation) = item.as_ref() else {
            panic!("the violation type is named")
        };
        let violation = document
            .types
            .get(&violation.id)
            .expect("the violation type is published");
        assert_eq!(violation.name, ["violation"]);
    }
}

#[test]
fn the_plain_tako_extractors_stay_unvalidated() {
    let mut router = Router::new(info());
    router.post("/plain", plain_create);
    let document = router.into_document().expect("the document validates");
    assert_eq!(document.operations.len(), 1);
    assert!(
        document.operations[0].errors.is_empty(),
        "a plain Tako extractor publishes no rejection"
    );
}
