use axum::response::IntoResponse;
use pons_axum::extract::{Path, Query, State};
use pons_axum::http::StatusCode;
use pons_axum::routing::{get, post};
use pons_axum::{
    Header, HeaderSpec, OptionalHeader, Router, WithHeader, WithOptionalHeader, WithStatus,
};
use pons_core::Schema;
use pons_ir::{AuthScheme, ContentKind, Info, InlineType, PathSegment, TypeKind, TypeRef};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Deserialize, Schema)]
struct UserPath {
    id: u64,
}

#[derive(Deserialize, Schema)]
struct UserQuery {
    limit: Option<u16>,
}

#[derive(Serialize, Schema)]
struct User {
    id: u64,
    name: String,
}

#[derive(Serialize, Schema)]
struct UserEvent {
    id: u64,
}

impl pons_axum::SseEvent for UserEvent {
    const NAME: &'static str = "user";
}

#[derive(Deserialize, Schema)]
struct CreateUser {
    name: String,
}

struct RequestId;

impl HeaderSpec for RequestId {
    type Value = String;
    const NAME: &'static str = "X-Request-ID";
}

struct Retry;

impl HeaderSpec for Retry {
    type Value = u16;
    const NAME: &'static str = "X-Retry";
}

struct TraceId;

impl HeaderSpec for TraceId {
    type Value = String;
    const NAME: &'static str = "X-Trace-ID";
}

#[derive(Clone)]
struct AppState;

async fn get_user(
    Path(path): Path<UserPath>,
    Query(query): Query<UserQuery>,
    State(state): State<AppState>,
) -> pons_axum::extract::Json<User> {
    let _ = (path.id, query.limit, state);
    pons_axum::extract::Json(User {
        id: 1,
        name: "Ada".into(),
    })
}

async fn list_users(State(_state): State<AppState>) -> pons_axum::extract::Json<Vec<User>> {
    pons_axum::extract::Json(vec![User {
        id: 1,
        name: "Ada".into(),
    }])
}

async fn headers(
    Header(request_id): Header<RequestId>,
    OptionalHeader(retry): OptionalHeader<Retry>,
) -> pons_axum::extract::Json<User> {
    let _ = (request_id, retry);
    pons_axum::extract::Json(User {
        id: 1,
        name: "Ada".into(),
    })
}

async fn response_headers() -> WithStatus<201, WithHeader<TraceId, pons_axum::extract::Json<User>>>
{
    WithStatus::new(WithHeader::new(
        pons_axum::extract::Json(User {
            id: 1,
            name: "Ada".into(),
        }),
        "trace-123".into(),
    ))
}

async fn optional_response_headers() -> WithOptionalHeader<Retry, pons_axum::extract::Json<User>> {
    WithOptionalHeader::new(
        pons_axum::extract::Json(User {
            id: 1,
            name: "Ada".into(),
        }),
        Some(3),
    )
}

async fn health() {}

async fn user_events()
-> pons_axum::Sse<UserEvent, impl futures_util::Stream<Item = UserEvent> + Send> {
    pons_axum::Sse::new(futures_util::stream::iter([UserEvent { id: 1 }]))
}

#[derive(Serialize, Schema, pons_core::ApiError)]
#[serde(rename_all = "snake_case")]
enum UserError {
    #[pons(status = 404)]
    Missing,
}

impl axum::response::IntoResponse for UserError {
    fn into_response(self) -> axum::response::Response {
        StatusCode::NOT_FOUND.into_response()
    }
}

async fn typed_user() -> Result<pons_axum::extract::Json<User>, UserError> {
    Err(UserError::Missing)
}

async fn create_user(
    pons_axum::extract::Json(input): pons_axum::extract::Json<CreateUser>,
) -> WithStatus<201, pons_axum::extract::Json<User>> {
    let name = input.name;
    WithStatus::new(pons_axum::extract::Json(User { id: 1, name }))
}

async fn created_user(
    pons_axum::extract::Json(input): pons_axum::extract::Json<CreateUser>,
) -> Result<WithStatus<201, pons_axum::extract::Json<User>>, UserError> {
    let name = input.name;
    Ok(WithStatus::new(pons_axum::extract::Json(User {
        id: 1,
        name,
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

#[test]
fn router_describes_path_query_body_response_and_state() {
    let document = Router::<AppState>::new(info())
        .route("/users/{id}", get(get_user))
        .route("/users", get(list_users).post(create_user))
        .route("/headers", get(headers))
        .route("/response-headers", get(response_headers))
        .route("/optional-response-headers", get(optional_response_headers))
        .route("/created", post(created_user))
        .route("/", get(health))
        .route("/typed", get(typed_user))
        .route("/events", get(user_events))
        .with_state::<()>(AppState)
        .auth_scheme("session", AuthScheme::Bearer { format: None })
        .into_document()
        .unwrap();

    let get_user = document
        .operations
        .iter()
        .find(|operation| operation.method == pons_ir::Method::Get)
        .unwrap();
    assert_eq!(get_user.id.last().unwrap().join("_"), "get_user");
    assert_eq!(get_user.path.len(), 2);
    let PathSegment::Param { param, ty } = &get_user.path[1] else {
        panic!();
    };
    assert_eq!(param, "id");
    assert!(matches!(
        ty,
        TypeRef::Inline(InlineType::Int { bits: 64, .. })
    ));
    assert!(matches!(get_user.query, Some(TypeRef::Named(_))));
    assert!(get_user.body.is_none());
    assert_eq!(get_user.responses[0].content, ContentKind::Json);
    assert!(
        get_user
            .responses
            .iter()
            .any(|response| matches!(response.body, Some(TypeRef::Named(_))))
    );

    let headers = document
        .operations
        .iter()
        .find(|operation| {
            operation
                .path
                .iter()
                .any(|segment| matches!(segment, PathSegment::Literal { lit } if lit == "headers"))
        })
        .unwrap();
    assert_eq!(headers.headers[0].wire, "X-Request-ID");
    assert_eq!(
        headers.headers[0].ident,
        vec!["request".to_owned(), "id".to_owned()]
    );
    assert_eq!(headers.headers[0].presence, pons_ir::Presence::Required);
    assert!(matches!(
        headers.headers[0].ty,
        TypeRef::Inline(InlineType::String { .. })
    ));
    assert_eq!(headers.headers[1].wire, "X-Retry");
    assert_eq!(headers.headers[1].presence, pons_ir::Presence::Optional);
    assert!(matches!(
        headers.headers[1].ty,
        TypeRef::Inline(InlineType::Int { bits: 16, .. })
    ));

    let response_headers = document
        .operations
        .iter()
        .find(|operation| {
            operation.path.iter().any(|segment| {
                matches!(segment, PathSegment::Literal { lit } if lit == "response-headers")
            })
        })
        .unwrap();
    assert_eq!(response_headers.responses[0].status, 201);
    assert_eq!(response_headers.responses[0].headers[0].wire, "X-Trace-ID");
    assert_eq!(
        response_headers.responses[0].headers[0].presence,
        pons_ir::Presence::Required
    );
    assert!(matches!(
        response_headers.responses[0].headers[0].ty,
        TypeRef::Inline(InlineType::String { .. })
    ));

    let optional_response_headers = document
        .operations
        .iter()
        .find(|operation| {
            operation.path.iter().any(|segment| {
                matches!(segment, PathSegment::Literal { lit } if lit == "optional-response-headers")
            })
        })
        .unwrap();
    assert_eq!(
        optional_response_headers.responses[0].headers[0].presence,
        pons_ir::Presence::Optional
    );
    assert!(matches!(
        optional_response_headers.responses[0].headers[0].ty,
        TypeRef::Inline(InlineType::Int { bits: 16, .. })
    ));

    let post = document
        .operations
        .iter()
        .find(|operation| operation.method == pons_ir::Method::Post)
        .unwrap();
    assert_eq!(post.body.as_ref().unwrap().content, ContentKind::Json);
    assert_eq!(post.responses[0].status, 201);
    assert!(
        !post
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "PONS-W101")
    );
    let response = WithStatus::<201, _>::new(pons_axum::extract::Json(User {
        id: 1,
        name: "Ada".into(),
    }))
    .into_response();
    assert_eq!(response.status(), StatusCode::CREATED);

    let response = WithHeader::<TraceId, _>::new(
        pons_axum::extract::Json(User {
            id: 1,
            name: "Ada".into(),
        }),
        "trace-123".into(),
    )
    .into_response();
    assert_eq!(response.headers()["x-trace-id"], "trace-123");
    let response = WithHeader::<TraceId, _>::new(
        pons_axum::extract::Json(User {
            id: 1,
            name: "Ada".into(),
        }),
        "bad\nvalue".into(),
    )
    .into_response();
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let response = WithStatus::<201, _>::new(WithHeader::<TraceId, _>::new(
        pons_axum::extract::Json(User {
            id: 1,
            name: "Ada".into(),
        }),
        "bad\nvalue".into(),
    ))
    .into_response();
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);

    let response = WithOptionalHeader::<Retry, _>::new(
        pons_axum::extract::Json(User {
            id: 1,
            name: "Ada".into(),
        }),
        None,
    )
    .into_response();
    assert!(!response.headers().contains_key("x-retry"));

    let created = document
        .operations
        .iter()
        .find(|operation| {
            operation
                .path
                .iter()
                .any(|segment| matches!(segment, PathSegment::Literal { lit } if lit == "created"))
        })
        .unwrap();
    assert_eq!(created.responses[0].status, 201);
    assert_eq!(created.errors[0].status, pons_ir::ErrorStatus::Exact(404));
    assert!(document.types.values().any(|definition| {
        matches!(&definition.definition, TypeKind::Struct { fields, .. } if fields.iter().any(|field| field.wire == "name"))
    }));
    let health = document
        .operations
        .iter()
        .find(|operation| {
            operation
                .id
                .last()
                .is_some_and(|name| name.join("_") == "health")
        })
        .unwrap();
    assert!(health.path.is_empty());
    let typed = document
        .operations
        .iter()
        .find(|operation| {
            operation
                .path
                .iter()
                .any(|segment| matches!(segment, PathSegment::Literal { lit } if lit == "typed"))
        })
        .unwrap();
    assert_eq!(typed.errors.len(), 1);
    assert_eq!(typed.errors[0].status, pons_ir::ErrorStatus::Exact(404));
    let events = document
        .operations
        .iter()
        .find(|operation| operation.stream.is_some())
        .unwrap();
    let Some(pons_ir::StreamDescription {
        kind: pons_ir::StreamKind::Sse { events: messages },
    }) = &events.stream
    else {
        panic!()
    };
    assert_eq!(messages[0].event, "user");
    assert!(matches!(messages[0].data, TypeRef::Named(_)));
    assert_eq!(events.responses[0].content, ContentKind::Stream);
}
