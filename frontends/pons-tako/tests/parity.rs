#![cfg(feature = "typed-header")]

use std::collections::BTreeMap;
use std::str::FromStr;

use http::StatusCode;
use pons_core::TypeCx;
use pons_ir::{Direction, InlineType, Method, OpaqueWire, PathSegment, TypeRef};
use pons_tako::{Info, OperationMeta};
use serde::{Deserialize, Serialize};
use tako::extractors::json::Json as TakoJson;
use tako::extractors::path::Path as TakoPath;
use tako::extractors::query::Query as TakoQuery;
use tako::extractors::typed_header::TypedHeader;

#[derive(Deserialize, pons_core::Schema)]
struct UserId {
    id: u64,
}

#[derive(Deserialize, pons_core::Schema)]
#[serde(deny_unknown_fields)]
struct ListQuery {
    #[serde(default)]
    limit: Option<u16>,
}

#[derive(Deserialize, pons_core::Schema)]
#[serde(deny_unknown_fields)]
struct CreateUser {
    name: String,
}

#[derive(Serialize, pons_core::Schema)]
struct User {
    id: u64,
    name: String,
}

#[derive(Debug, Serialize, pons_core::Schema, pons_core::ApiError)]
#[serde(tag = "code", rename_all = "snake_case")]
enum ApiError {
    #[pons(status = 400)]
    Invalid { reason: String },
}

impl axum::response::IntoResponse for ApiError {
    fn into_response(self) -> axum::response::Response {
        axum::response::IntoResponse::into_response((StatusCode::BAD_REQUEST, axum::Json(self)))
    }
}

impl tako::responder::Responder for ApiError {
    fn into_response(self) -> tako::types::Response {
        let body = serde_json::to_value(self).unwrap_or(serde_json::Value::Null);
        tako::responder::Responder::into_response((StatusCode::BAD_REQUEST, body))
    }
}

struct UserAgentHeader;

struct OpaqueUserAgent(String);

impl FromStr for OpaqueUserAgent {
    type Err = std::convert::Infallible;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        Ok(Self(text.to_owned()))
    }
}

impl pons_core::Schema for OpaqueUserAgent {
    fn schema(_: &mut TypeCx, _: Direction) -> TypeRef {
        TypeRef::Inline(InlineType::Opaque {
            codec: "tako.typed-header".into(),
            wire: OpaqueWire::Text,
            ext: Default::default(),
        })
    }
}

impl pons_axum::HeaderSpec for UserAgentHeader {
    type Value = OpaqueUserAgent;
    const NAME: &'static str = "user-agent";
}

async fn axum_get_user(
    pons_axum::Path(user): pons_axum::Path<UserId>,
    pons_axum::Query(query): pons_axum::Query<ListQuery>,
    pons_axum::Header(user_agent): pons_axum::Header<UserAgentHeader>,
) -> axum::Json<User> {
    let _ = (query.limit, user_agent.0.len());
    axum::Json(User {
        id: user.id,
        name: "Ada".into(),
    })
}

async fn axum_create_user(
    pons_axum::Json(input): pons_axum::Json<CreateUser>,
) -> Result<pons_axum::WithStatus<201, axum::Json<User>>, ApiError> {
    Ok(pons_axum::WithStatus::new(axum::Json(User {
        id: 1,
        name: input.name,
    })))
}

async fn tako_get_user(
    TakoPath(user): TakoPath<UserId>,
    TakoQuery(query): TakoQuery<ListQuery>,
    TypedHeader(_user_agent): TypedHeader<headers::UserAgent>,
) -> TakoJson<User> {
    let _ = query.limit;
    TakoJson(User {
        id: user.id,
        name: "Ada".into(),
    })
}

async fn tako_create_user(
    TakoJson(input): TakoJson<CreateUser>,
) -> Result<pons_tako::WithStatus<201, TakoJson<User>>, ApiError> {
    Ok(pons_tako::WithStatus::new(TakoJson(User {
        id: 1,
        name: input.name,
    })))
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

fn axum_get_meta() -> pons_axum::OperationMeta {
    pons_axum::OperationMeta::new("users.get")
        .summary("Fetch a user")
        .tag("users")
}

fn axum_create_meta() -> pons_axum::OperationMeta {
    pons_axum::OperationMeta::new("users.create")
        .summary("Create a user")
        .tag("users")
}

fn tako_get_meta() -> OperationMeta {
    OperationMeta::new("users.get")
        .summary("Fetch a user")
        .tag("users")
}

fn tako_create_meta() -> OperationMeta {
    OperationMeta::new("users.create")
        .summary("Create a user")
        .tag("users")
}

fn axum_router() -> pons_axum::Router<()> {
    let nested = pons_axum::Router::<()>::new(info()).route(
        "/users/{id}",
        pons_axum::routing::get_with(axum_get_user, axum_get_meta()),
    );
    pons_axum::Router::<()>::new(info())
        .route(
            "/users",
            pons_axum::routing::post_with(axum_create_user, axum_create_meta()),
        )
        .nest("/v1", nested)
}

fn tako_router() -> pons_tako::Router {
    let mut router = pons_tako::Router::new(info());
    router.scope("/v1", |router| {
        router.get_with("/users/{id}", tako_get_user, tako_get_meta());
    });
    router.post_with("/users", tako_create_user, tako_create_meta());
    router
}

fn declares_request_rejection(document: &pons_ir::Document) -> bool {
    document.operations.iter().any(|operation| {
        operation.errors.iter().any(|error| {
            error.status == pons_ir::ErrorStatus::Exact(422)
                && error
                    .name
                    .iter()
                    .map(String::as_str)
                    .eq(["request", "rejected"])
        })
    })
}

fn get(document: &pons_ir::Document) -> &pons_ir::Operation {
    document
        .operations
        .iter()
        .find(|operation| operation.method == Method::Get)
        .expect("the get operation")
}

#[test]
fn tako_and_axum_describe_the_same_document() {
    let error = ApiError::Invalid {
        reason: "missing name".into(),
    };
    assert_eq!(serde_json::to_value(&error).unwrap()["code"], "invalid");
    let axum = axum_router().into_document().unwrap();
    let tako = tako_router().into_document().unwrap();
    assert_eq!(axum.operations.len(), 2);
    assert_eq!(tako.operations.len(), 2);

    let ids = |document: &pons_ir::Document| {
        document
            .operations
            .iter()
            .map(|operation| operation.id.clone())
            .collect::<Vec<_>>()
    };
    assert_eq!(ids(&axum), ids(&tako));

    assert_eq!(get(&axum).path, get(&tako).path);
    assert_eq!(
        get(&axum).path.first(),
        Some(&PathSegment::Literal { lit: "v1".into() })
    );

    assert!(
        declares_request_rejection(&axum),
        "the Axum request extractors publish the rejection they serve"
    );
    assert!(
        !declares_request_rejection(&tako),
        "Tako does not validate requests, so its document declares no rejection"
    );

    pons_ir::equivalent_ignoring_request_rejections(&axum, &tako).unwrap_or_else(|differences| {
        panic!(
            "the documents differ:\n{}",
            differences
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join("\n")
        )
    });
}
