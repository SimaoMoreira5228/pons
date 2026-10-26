mod support;

use axum::response::IntoResponse;
use pons_axum::body::Bytes;
use pons_axum::extract::State;
use pons_axum::http::StatusCode;
use pons_axum::routing::{delete, get, post};
use pons_axum::{
    Header, HeaderSpec, Ndjson, NoContent, OptionalHeader, Path, Query, RawBytes, Router,
    WithHeader, WithOptionalHeader, WithStatus,
};
use pons_core::Schema;
use pons_ir::{
    AuthScheme, ContentKind, CredentialLocator, InlineType, PathSegment, ResponseStatus, TypeKind,
    TypeRef,
};
use serde::Serialize;
use support::router::*;
use support::*;

async fn get_user(
    Path(path): Path<UserPath>,
    Query(query): Query<UserQuery>,
    State(state): State<AppState>,
) -> axum::Json<User> {
    let _ = (path.id, query.limit, state);
    axum::Json(User {
        id: 1,
        name: "Ada".into(),
    })
}

async fn create_user(
    pons_axum::Json(input): pons_axum::Json<CreateUser>,
) -> WithStatus<201, axum::Json<User>> {
    let name = input.name;
    WithStatus::new(axum::Json(User { id: 1, name }))
}

async fn headers(
    Header(request_id): Header<RequestId>,
    OptionalHeader(retry): OptionalHeader<Retry>,
) -> axum::Json<User> {
    let _ = (request_id, retry);
    axum::Json(User {
        id: 1,
        name: "Ada".into(),
    })
}

async fn provisioned(_provides: Provides) -> axum::Json<User> {
    axum::Json(User {
        id: 1,
        name: "Ada".into(),
    })
}

async fn response_headers() -> WithStatus<201, WithHeader<TraceId, axum::Json<User>>> {
    WithStatus::new(WithHeader::new(
        axum::Json(User {
            id: 1,
            name: "Ada".into(),
        }),
        "trace-123".into(),
    ))
}

async fn optional_response_headers() -> WithOptionalHeader<Retry, axum::Json<User>> {
    WithOptionalHeader::new(
        axum::Json(User {
            id: 1,
            name: "Ada".into(),
        }),
        Some(3),
    )
}

async fn user_events()
-> pons_axum::Sse<UserEvent, impl futures_util::Stream<Item = UserEvent> + Send> {
    pons_axum::Sse::new(futures_util::stream::iter([UserEvent { id: 1 }]))
}

async fn user_ndjson() -> Ndjson<UserEvent, impl futures_util::Stream<Item = UserEvent> + Send> {
    Ndjson::new(futures_util::stream::iter([UserEvent { id: 1 }]))
}

async fn user_bytes() -> RawBytes<impl futures_util::Stream<Item = Bytes> + Send> {
    RawBytes::new(futures_util::stream::iter([Bytes::from_static(b"pdf")]))
}

#[derive(Serialize, Schema)]
struct UserEvent {
    id: u64,
}

impl pons_axum::SseEvent for UserEvent {
    const NAME: &'static str = "user";
}

#[derive(Serialize, Schema, pons_core::ApiError)]
#[serde(rename_all = "snake_case")]
enum UserError {
    #[pons(status = 404)]
    Missing,
}

#[allow(dead_code)]
#[derive(Serialize, Schema, pons_core::ApiError)]
enum CommonError {
    #[pons(status = 503)]
    Unavailable,
}

impl axum::response::IntoResponse for UserError {
    fn into_response(self) -> axum::response::Response {
        StatusCode::NOT_FOUND.into_response()
    }
}

async fn typed_user() -> Result<axum::Json<User>, UserError> {
    Err(UserError::Missing)
}

async fn created_user(
    pons_axum::Json(input): pons_axum::Json<CreateUser>,
) -> Result<WithStatus<201, axum::Json<User>>, UserError> {
    let name = input.name;
    Ok(WithStatus::new(axum::Json(User { id: 1, name })))
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

async fn raw_headers(_headers: pons_axum::http::HeaderMap) -> axum::Json<User> {
    axum::Json(User {
        id: 1,
        name: "Ada".into(),
    })
}

async fn raw_response() -> axum::response::Response {
    StatusCode::OK.into_response()
}

async fn dynamic_status() -> (StatusCode, axum::Json<User>) {
    (
        StatusCode::CREATED,
        axum::Json(User {
            id: 1,
            name: "Ada".into(),
        }),
    )
}

async fn teapot() -> (StatusCode, axum::Json<User>) {
    (
        StatusCode::IM_A_TEAPOT,
        axum::Json(User {
            id: 1,
            name: "Ada".into(),
        }),
    )
}

async fn unservable_status() -> WithStatus<600, axum::Json<User>> {
    WithStatus::new(axum::Json(User {
        id: 1,
        name: "Ada".into(),
    }))
}

async fn informational_status() -> WithStatus<100, axum::Json<User>> {
    WithStatus::new(axum::Json(User {
        id: 1,
        name: "Ada".into(),
    }))
}

#[test]
fn raw_axum_boundaries_are_diagnosed_without_inventing_contracts() {
    let router = Router::<()>::new(info())
        .route("/raw-headers", get(raw_headers))
        .route("/raw-response", get(raw_response));
    let document = router.document();
    let headers = document
        .operations
        .iter()
        .find(|operation| {
            operation.path.iter().any(
                |segment| matches!(segment, PathSegment::Literal { lit } if lit == "raw-headers"),
            )
        })
        .unwrap();
    assert!(headers.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "PONS-E102"
            && diagnostic.severity == pons_ir::DiagnosticSeverity::Error
            && diagnostic.message.contains("raw HeaderMap")
    }));
    assert!(headers.headers.is_empty());
    let response = document
        .operations
        .iter()
        .find(|operation| {
            operation.path.iter().any(
                |segment| matches!(segment, PathSegment::Literal { lit } if lit == "raw-response"),
            )
        })
        .unwrap();
    assert!(response.responses.is_empty());
    assert!(response.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "PONS-E102"
            && diagnostic.severity == pons_ir::DiagnosticSeverity::Error
            && diagnostic.message.contains("raw Axum responses")
    }));
    assert!(router.into_document().is_err());
}

#[test]
fn dynamic_response_status_preserves_the_body_contract() {
    let document = Router::<()>::new(info())
        .route("/dynamic", get(dynamic_status))
        .into_document()
        .unwrap();
    let operation = &document.operations[0];
    assert_eq!(operation.responses.len(), 1);
    assert_eq!(operation.responses[0].status, ResponseStatus::Unknown);
    assert_eq!(operation.responses[0].content, ContentKind::Json);
    assert!(matches!(
        operation.responses[0].body,
        Some(TypeRef::Named(_))
    ));
    assert!(
        operation
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "PONS-W101"
                && diagnostic
                    .message
                    .contains("a (StatusCode, R) response publishes the unknown status")),
        "{:?} must say that the catch-all status the handler writes is never checked",
        operation.diagnostics
    );

    let response = (
        StatusCode::CREATED,
        axum::Json(User {
            id: 1,
            name: "Ada".into(),
        }),
    )
        .into_response();
    assert_eq!(response.status(), StatusCode::CREATED);
}

#[test]
fn the_catch_all_status_is_published_and_says_nothing_about_the_status_served() {
    let mut app = built(Router::<()>::new(info()).route("/teapot", get(teapot)));
    let document = Router::<()>::new(info())
        .route("/teapot", get(teapot))
        .into_document()
        .unwrap();
    let operation = &document.operations[0];
    assert_eq!(operation.responses[0].status, ResponseStatus::Unknown);
    assert_eq!(
        get_status(&mut app, "/teapot", false),
        StatusCode::IM_A_TEAPOT,
        "the handler writes the status itself, and the catch-all the document publishes never mentions it"
    );
}

#[test]
fn router_describes_no_content_response() {
    assert_eq!(NoContent.into_response().status(), StatusCode::NO_CONTENT);
    let document = Router::<()>::new(info())
        .route("/delete", delete(delete_user))
        .into_document()
        .unwrap();
    let operation = document
        .operations
        .iter()
        .find(|operation| operation.method == pons_ir::Method::Delete)
        .unwrap();
    assert_eq!(operation.responses[0].status, ResponseStatus::Exact(204));
    assert_eq!(operation.responses[0].content, ContentKind::Empty);
    assert!(operation.responses[0].body.is_none());
}

#[test]
fn router_describes_ndjson_stream() {
    let response = Ndjson::<UserEvent, _>::new(futures_util::stream::iter([UserEvent { id: 1 }]))
        .into_response();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers()[pons_axum::http::header::CONTENT_TYPE],
        "application/x-ndjson"
    );
    let document = Router::<()>::new(info())
        .route("/ndjson", get(user_ndjson))
        .into_document()
        .unwrap();
    let operation = document
        .operations
        .iter()
        .find(|operation| operation.method == pons_ir::Method::Get)
        .unwrap();
    let stream = operation.stream.as_ref().unwrap();
    assert!(matches!(
        stream.kind,
        pons_ir::StreamKind::Ndjson {
            item: TypeRef::Named(_)
        }
    ));
    assert_eq!(operation.responses[0].content, ContentKind::Stream);
    assert!(operation.responses[0].body.is_none());
}

#[test]
fn router_describes_raw_bytes_stream() {
    let response =
        RawBytes::new(futures_util::stream::iter([Bytes::from_static(b"pdf")])).into_response();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers()[pons_axum::http::header::CONTENT_TYPE],
        "application/octet-stream"
    );
    let document = Router::<()>::new(info())
        .route("/bytes", get(user_bytes))
        .into_document()
        .unwrap();
    let operation = document
        .operations
        .iter()
        .find(|operation| operation.method == pons_ir::Method::Get)
        .unwrap();
    assert!(matches!(
        operation.stream.as_ref().unwrap().kind,
        pons_ir::StreamKind::Bytes { ref content_type } if content_type == "application/octet-stream"
    ));
    assert_eq!(operation.responses[0].content, ContentKind::Stream);
    assert!(operation.responses[0].body.is_none());
}

#[test]
fn router_describes_path_query_body_response_and_state() {
    let document = Router::<AppState>::new(info())
        .route("/users/{id}", get(get_user))
        .route("/users", get(list_users).post(create_user))
        .route("/headers", get(headers))
        .route("/provisioned", get(provisioned))
        .route("/response-headers", get(response_headers))
        .route("/optional-response-headers", get(optional_response_headers))
        .route("/created", post(created_user))
        .route("/", get(health))
        .route("/typed", get(typed_user))
        .route("/events", get(user_events))
        .with_state::<()>(AppState)
        .auth_scheme("session", AuthScheme::Bearer { format: None })
        .common_error::<CommonError>()
        .into_document()
        .unwrap();
    assert_eq!(document.common_errors.len(), 1);
    assert_eq!(
        document.common_errors[0].status,
        pons_ir::ErrorStatus::Exact(503)
    );
    let Some(pons_ir::TypeRef::Named(error_type)) = &document.common_errors[0].body else {
        panic!()
    };
    assert!(document.types.contains_key(&error_type.id));

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

    let provisioned = document
        .operations
        .iter()
        .find(|operation| {
            operation.path.iter().any(
                |segment| matches!(segment, PathSegment::Literal { lit } if lit == "provisioned"),
            )
        })
        .unwrap();
    assert_eq!(provisioned.provides.as_ref().unwrap().scheme, "session");
    assert_eq!(
        provisioned.provides.as_ref().unwrap().value,
        CredentialLocator::Body {
            pointer: "/token".into()
        }
    );

    let response_headers = document
        .operations
        .iter()
        .find(|operation| {
            operation.path.iter().any(|segment| {
                matches!(segment, PathSegment::Literal { lit } if lit == "response-headers")
            })
        })
        .unwrap();
    assert_eq!(
        response_headers.responses[0].status,
        ResponseStatus::Exact(201)
    );
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
    assert_eq!(post.responses[0].status, ResponseStatus::Exact(201));
    assert!(
        post.errors
            .iter()
            .any(|error| error.status == pons_ir::ErrorStatus::Exact(422)),
        "a pons body reader answers a body it cannot read as the rejection it declares: {:?}",
        post.errors
    );
    assert!(
        !post.diagnostics.iter().any(|diagnostic| {
            diagnostic.code == "PONS-W101" && diagnostic.message.contains("413")
        }),
        "{:?} must not claim a status pons reads the body before the framework can answer it",
        post.diagnostics
    );
    let response = WithStatus::<201, _>::new(axum::Json(User {
        id: 1,
        name: "Ada".into(),
    }))
    .into_response();
    assert_eq!(response.status(), StatusCode::CREATED);

    let response = WithHeader::<TraceId, _>::new(
        axum::Json(User {
            id: 1,
            name: "Ada".into(),
        }),
        "trace-123".into(),
    )
    .into_response();
    assert_eq!(response.headers()["x-trace-id"], "trace-123");
    let response = WithHeader::<TraceId, _>::new(
        axum::Json(User {
            id: 1,
            name: "Ada".into(),
        }),
        "bad\nvalue".into(),
    )
    .into_response();
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let response = WithStatus::<201, _>::new(WithHeader::<TraceId, _>::new(
        axum::Json(User {
            id: 1,
            name: "Ada".into(),
        }),
        "bad\nvalue".into(),
    ))
    .into_response();
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);

    let response = WithOptionalHeader::<Retry, _>::new(
        axum::Json(User {
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
    assert_eq!(created.responses[0].status, ResponseStatus::Exact(201));
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

#[test]
#[should_panic(expected = "representable by Pons IR")]
fn with_status_refuses_unservable_statuses_while_building_the_route() {
    let _ = Router::<()>::new(info()).route("/unservable", get(unservable_status));
}

#[test]
fn a_one_hundred_response_contract_is_refused_when_the_document_is_validated() {
    let router = Router::<()>::new(info()).route("/informational", get(informational_status));
    let errors = router.into_document().unwrap_err();
    assert!(
        errors
            .iter()
            .any(|error| error.to_string().contains("cannot carry a body")),
        "a 1xx contract must be refused at build time: {errors:?}"
    );
}
