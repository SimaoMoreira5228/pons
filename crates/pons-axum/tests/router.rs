use axum::response::IntoResponse;
use pons_axum::body::Bytes;
use pons_axum::extract::{Form, Multipart, Path, Query, State};
use pons_axum::http::StatusCode;
use pons_axum::routing::{delete, get, post, post_with};
use pons_axum::{
    Auth, DescribeRequest, DocumentedLayer, Header, HeaderSpec, Ndjson, NoContent, OperationCx,
    OperationMeta, OptionalHeader, RawBytes, Router, SessionAuth, WithHeader, WithOptionalHeader,
    WithStatus,
};
use pons_core::Schema;
use pons_ir::{
    AuthScheme, ContentKind, CredentialLocator, CredentialProvision, Info, InlineType, PathSegment,
    TypeKind, TypeRef,
};
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

struct Provides;

impl pons_axum::extract::FromRequestParts<AppState> for Provides {
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(
        _: &mut pons_axum::http::request::Parts,
        _: &AppState,
    ) -> Result<Self, Self::Rejection> {
        Ok(Self)
    }
}

impl DescribeRequest for Provides {
    fn describe(cx: &mut OperationCx) {
        cx.provide(CredentialProvision {
            scheme: "session".into(),
            value: CredentialLocator::Body {
                pointer: "/token".into(),
            },
            expires_at: None,
            ext: BTreeMap::new(),
        });
    }
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

async fn provisioned(_provides: Provides) -> pons_axum::extract::Json<User> {
    pons_axum::extract::Json(User {
        id: 1,
        name: "Ada".into(),
    })
}

async fn authenticated(auth: Auth<Provides>) -> pons_axum::extract::Json<User> {
    let _ = auth.into_inner();
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

async fn user_ndjson() -> Ndjson<UserEvent, impl futures_util::Stream<Item = UserEvent> + Send> {
    Ndjson::new(futures_util::stream::iter([UserEvent { id: 1 }]))
}

async fn user_bytes() -> RawBytes<impl futures_util::Stream<Item = Bytes> + Send> {
    RawBytes::new(futures_util::stream::iter([Bytes::from_static(b"pdf")]))
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

async fn form_user(Form(input): Form<CreateUser>) -> pons_axum::extract::Json<User> {
    pons_axum::extract::Json(User {
        id: 1,
        name: input.name,
    })
}

async fn multipart_user(_multipart: Multipart) -> pons_axum::extract::Json<User> {
    pons_axum::extract::Json(User {
        id: 1,
        name: "Ada".into(),
    })
}

async fn delete_user() -> NoContent {
    NoContent
}

async fn noop_layer(
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    next.run(request).await
}

async fn raw_headers(_headers: pons_axum::http::HeaderMap) -> pons_axum::extract::Json<User> {
    pons_axum::extract::Json(User {
        id: 1,
        name: "Ada".into(),
    })
}

async fn raw_response() -> axum::response::Response {
    StatusCode::OK.into_response()
}

async fn dynamic_status() -> (StatusCode, pons_axum::extract::Json<User>) {
    (
        StatusCode::IM_A_TEAPOT,
        pons_axum::extract::Json(User {
            id: 1,
            name: "Ada".into(),
        }),
    )
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
fn router_describes_form_request_body() {
    let document = Router::<()>::new(info())
        .route("/form", post(form_user))
        .into_document()
        .unwrap();
    let operation = document
        .operations
        .iter()
        .find(|operation| {
            operation
                .path
                .iter()
                .any(|segment| matches!(segment, PathSegment::Literal { lit } if lit == "form"))
        })
        .unwrap();
    let body = operation.body.as_ref().unwrap();
    assert_eq!(body.content, ContentKind::Form);
    assert!(matches!(body.ty, TypeRef::Named(_)));
}

#[test]
fn router_describes_multipart_request_as_opaque() {
    let router = Router::<()>::new(info()).route("/multipart", post(multipart_user));
    let document = router.document();
    let operation = document
        .operations
        .iter()
        .find(|operation| {
            operation.path.iter().any(
                |segment| matches!(segment, PathSegment::Literal { lit } if lit == "multipart"),
            )
        })
        .unwrap();
    let body = operation.body.as_ref().unwrap();
    assert_eq!(body.content, ContentKind::Multipart);
    assert_eq!(body.ty, TypeRef::Inline(InlineType::Any));
    assert!(operation.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "PONS-W101" && diagnostic.message.contains("multipart fields are opaque")
    }));
    assert!(router.into_document().is_err());
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
        diagnostic.code == "PONS-W101" && diagnostic.message.contains("raw HeaderMap")
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
        diagnostic.code == "PONS-W101" && diagnostic.message.contains("raw Axum responses")
    }));
    assert!(router.into_document().is_err());
}

#[test]
fn dynamic_response_status_is_diagnosed_without_inventing_a_contract() {
    let router = Router::<()>::new(info()).route("/dynamic", get(dynamic_status));
    let document = router.document();
    let operation = &document.operations[0];
    assert!(operation.responses.is_empty());
    assert!(operation.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "PONS-W102" && diagnostic.message.contains("dynamic response status")
    }));
    assert!(router.into_document().is_err());
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
    assert_eq!(operation.responses[0].status, 204);
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
fn auth_wrapper_delegates_extraction_and_requires_session() {
    let document = Router::<AppState>::new(info())
        .auth_scheme("session", AuthScheme::Bearer { format: None })
        .route("/authenticated", get(authenticated))
        .into_document()
        .unwrap();
    let operation = document
        .operations
        .iter()
        .find(|operation| operation.method == pons_ir::Method::Get)
        .unwrap();
    assert_eq!(
        operation.auth.alternatives,
        vec![vec!["session".to_owned()]]
    );
    assert!(!operation.auth.optional);
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
fn documented_layer_applies_auth_to_the_group() {
    let router = Router::<AppState>::new(info()).route("/authenticated", get(authenticated));
    let router =
        DocumentedLayer::new(axum::middleware::from_fn(noop_layer), SessionAuth).layer(router);
    let document = router
        .auth_scheme("session", AuthScheme::Bearer { format: None })
        .into_document()
        .unwrap();
    assert_eq!(
        document.operations[0].auth.alternatives,
        vec![vec!["session".to_owned()]]
    );
}

#[test]
fn operation_meta_overrides_identity_and_adds_contract_metadata() {
    let document = Router::<()>::new(info())
        .auth_scheme("session", AuthScheme::Bearer { format: None })
        .route(
            "/login",
            post_with(
                create_user,
                OperationMeta::new("auth.login")
                    .summary("Create a session")
                    .tag("auth")
                    .provides(
                        "session",
                        CredentialLocator::Body {
                            pointer: "/token".into(),
                        },
                    )
                    .expires_at(CredentialLocator::Body {
                        pointer: "/expires".into(),
                    }),
            ),
        )
        .into_document()
        .unwrap();
    let operation = &document.operations[0];
    assert_eq!(
        operation.id,
        vec![vec![String::from("auth")], vec![String::from("login")]]
    );
    let doc = operation.doc.as_ref().unwrap();
    assert_eq!(doc.summary.as_deref(), Some("Create a session"));
    assert_eq!(doc.tags, vec!["auth".to_owned()]);
    assert_eq!(operation.provides.as_ref().unwrap().scheme, "session");
    assert_eq!(
        operation.provides.as_ref().unwrap().expires_at,
        Some(CredentialLocator::Body {
            pointer: "/expires".into(),
        })
    );
}

#[test]
fn document_lowering_adds_metadata_and_revalidates() {
    let document = Router::<()>::new(info())
        .try_lower_document(|mut document| {
            document.ext.insert("policy".into(), "enabled".into());
            Ok(document)
        })
        .unwrap();
    assert_eq!(document.ext["policy"].as_str(), Some("enabled"));

    let errors = Router::<()>::new(info())
        .try_lower_document(|mut document| {
            document.ir_version = "9.0".into();
            Ok(document)
        })
        .unwrap_err();
    assert!(
        errors
            .iter()
            .any(|error| error.to_string().contains("unsupported IR version"))
    );
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
