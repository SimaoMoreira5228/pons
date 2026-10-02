use axum::response::IntoResponse;
use pons_axum::body::Bytes;
use pons_axum::extract::{Multipart, State};
use pons_axum::http::StatusCode;
use pons_axum::routing::{delete, get, get_with, post, post_with};
use pons_axum::{
    Auth, DescribeRequest, DocumentedLayer, Header, HeaderSpec, Ndjson, NoContent, OperationCx,
    OperationMeta, OptionalHeader, RawBytes, Router, SessionAuth, WithHeader, WithOptionalHeader,
    WithStatus,
};
use pons_axum::{Form, Path, Query};
use pons_core::Schema;
use pons_ir::{
    AuthScheme, ContentKind, CredentialLocator, CredentialProvision, Info, InlineType, OpaqueWire,
    Operation, PathSegment, ResponseStatus, TypeKind, TypeRef,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::future::Future;
use tower_service::Service;

#[derive(Deserialize, Schema)]
struct UserPath {
    id: u64,
}

#[derive(Deserialize, Schema)]
struct FilePath {
    rest: String,
}

#[derive(Deserialize, Schema)]
#[serde(deny_unknown_fields)]
struct UserQuery {
    #[serde(default)]
    limit: Option<u16>,
}

#[derive(Deserialize, Schema)]
#[serde(deny_unknown_fields)]
struct Q {
    #[serde(default)]
    limit: Option<u16>,
}

#[derive(Deserialize, Schema)]
#[serde(deny_unknown_fields)]
struct RequiredNullableQ {
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

async fn get_file(Path(path): Path<FilePath>) -> pons_axum::extract::Json<User> {
    let _ = path.rest;
    pons_axum::extract::Json(User {
        id: 1,
        name: "Ada".into(),
    })
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
    pons_axum::Json(input): pons_axum::Json<CreateUser>,
) -> WithStatus<201, pons_axum::extract::Json<User>> {
    let name = input.name;
    WithStatus::new(pons_axum::extract::Json(User { id: 1, name }))
}

async fn created_user(
    pons_axum::Json(input): pons_axum::Json<CreateUser>,
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

async fn query_q(Query(query): Query<Q>) -> pons_axum::extract::Json<User> {
    let _ = query.limit;
    pons_axum::extract::Json(User {
        id: 1,
        name: "Ada".into(),
    })
}

async fn form_q(Form(form): Form<Q>) -> pons_axum::extract::Json<User> {
    let _ = form.limit;
    pons_axum::extract::Json(User {
        id: 1,
        name: "Ada".into(),
    })
}

async fn path_q(Path(path): Path<Q>) -> pons_axum::extract::Json<User> {
    let _ = path.limit;
    pons_axum::extract::Json(User {
        id: 1,
        name: "Ada".into(),
    })
}

async fn required_nullable_q(
    Query(query): Query<RequiredNullableQ>,
) -> pons_axum::extract::Json<User> {
    let _ = query.limit;
    pons_axum::extract::Json(User {
        id: 1,
        name: "Ada".into(),
    })
}

async fn multipart_user(_multipart: Multipart) -> pons_axum::extract::Json<User> {
    pons_axum::extract::Json(User {
        id: 1,
        name: "Ada".into(),
    })
}

#[derive(Deserialize, Serialize, Schema)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum FlattenedCreate {
    Simple {
        name: String,
        #[serde(flatten)]
        extra: BTreeMap<String, String>,
    },
}

async fn flattened_create(
    pons_axum::Json(FlattenedCreate::Simple { name, .. }): pons_axum::Json<FlattenedCreate>,
) -> pons_axum::extract::Json<User> {
    pons_axum::extract::Json(User { id: 1, name })
}

async fn delete_user() -> NoContent {
    NoContent
}

async fn unservable_status() -> WithStatus<600, pons_axum::extract::Json<User>> {
    WithStatus::new(pons_axum::extract::Json(User {
        id: 1,
        name: "Ada".into(),
    }))
}

async fn informational_status() -> WithStatus<100, pons_axum::extract::Json<User>> {
    WithStatus::new(pons_axum::extract::Json(User {
        id: 1,
        name: "Ada".into(),
    }))
}

async fn first_user() -> pons_axum::extract::Json<User> {
    pons_axum::extract::Json(User {
        id: 1,
        name: "Ada".into(),
    })
}

async fn second_user() -> NoContent {
    NoContent
}

async fn noop_layer(
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    next.run(request).await
}

async fn require_authorization(
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    if request.headers().contains_key("authorization") {
        next.run(request).await
    } else {
        StatusCode::UNAUTHORIZED.into_response()
    }
}

static LAYER_TRACE: std::sync::Mutex<Vec<&'static str>> = std::sync::Mutex::new(Vec::new());

async fn trace_layer_a(
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    LAYER_TRACE.lock().unwrap().push("a");
    next.run(request).await
}

async fn trace_layer_b(
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    LAYER_TRACE.lock().unwrap().push("b");
    next.run(request).await
}

fn block_on<F: Future>(future: F) -> F::Output {
    struct ThreadWaker(std::thread::Thread);

    impl std::task::Wake for ThreadWaker {
        fn wake(self: std::sync::Arc<Self>) {
            self.0.unpark();
        }
    }

    let waker = std::task::Waker::from(std::sync::Arc::new(ThreadWaker(std::thread::current())));
    let mut context = std::task::Context::from_waker(&waker);
    let mut future = std::pin::pin!(future);
    loop {
        match future.as_mut().poll(&mut context) {
            std::task::Poll::Ready(value) => return value,
            std::task::Poll::Pending => std::thread::park(),
        }
    }
}

fn get_status(app: &mut axum::Router<()>, path: &str, authorized: bool) -> StatusCode {
    let mut builder = pons_axum::http::Request::builder().uri(path);
    if authorized {
        builder = builder.header("authorization", "Bearer token");
    }
    let request: pons_axum::extract::Request = builder.body(axum::body::Body::empty()).unwrap();
    block_on(app.call(request)).unwrap().status()
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
        StatusCode::CREATED,
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

fn operation<'a>(document: &'a pons_ir::Document, literal: &str) -> &'a Operation {
    document
        .operations
        .iter()
        .find(|operation| {
            operation
                .path
                .iter()
                .any(|segment| matches!(segment, PathSegment::Literal { lit } if lit == literal))
        })
        .unwrap()
}

fn optional_integer(ty: &TypeRef) -> bool {
    matches!(ty, TypeRef::Inline(InlineType::Int { bits: 16, .. }))
}

#[test]
fn a_flat_carrier_publishes_a_nullable_optional_field_as_an_absent_key() {
    let document = Router::<()>::new(info())
        .route("/query", get(query_q))
        .route("/form-body", post(form_q))
        .route("/segment/{limit}", get(path_q))
        .into_document()
        .unwrap();

    let query = operation(&document, "query");
    let fields = pons_ir::query_fields(&document, query.query.as_ref().unwrap()).unwrap();
    assert_eq!(fields.len(), 1);
    assert_eq!(fields[0].wire, "limit");
    assert_eq!(fields[0].presence, pons_ir::Presence::Optional);
    assert!(optional_integer(&fields[0].ty));

    let TypeRef::Named(form) = &operation(&document, "form-body").body.as_ref().unwrap().ty else {
        panic!();
    };
    let TypeKind::Struct { fields, .. } = &document.types[&form.id].definition else {
        panic!();
    };
    assert_eq!(fields.len(), 1);
    assert_eq!(fields[0].wire, "limit");
    assert_eq!(fields[0].presence, pons_ir::Presence::Optional);
    assert!(optional_integer(&fields[0].ty));

    let PathSegment::Param { param, ty } = &operation(&document, "segment").path[1] else {
        panic!();
    };
    assert_eq!(param, "limit");
    assert!(optional_integer(ty));
}

#[test]
fn a_required_nullable_query_field_is_refused_by_name() {
    let errors = Router::<()>::new(info())
        .route("/required", get(required_nullable_q))
        .into_document()
        .unwrap_err();
    let errors = errors.iter().map(ToString::to_string).collect::<Vec<_>>();
    assert!(
        errors.iter().any(|error| {
            error.contains("\"limit\"") && error.contains("no encoding for a present null")
        }),
        "an Option<T> field without a serde default is required, and a query string has no encoding for a present null: {errors:?}"
    );
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
    assert_eq!(
        body.ty,
        TypeRef::Inline(InlineType::Opaque {
            codec: "axum.multipart".into(),
            wire: OpaqueWire::Unknown,
        })
    );
    assert!(operation.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "PONS-W101"
            && diagnostic.severity == pons_ir::DiagnosticSeverity::Warning
            && diagnostic.message.contains("multipart fields are opaque")
    }));
    assert!(router.into_document().is_ok());
}

#[test]
fn enum_payload_flatten_refuses_to_publish_a_narrowed_body() {
    let router = Router::<()>::new(info()).route("/flattened", post(flattened_create));
    let document = router.document();
    let operation = &document.operations[0];
    assert!(operation.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "PONS-E101"
            && diagnostic.severity == pons_ir::DiagnosticSeverity::Error
            && diagnostic.message.contains("flatten")
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
    assert!(operation.diagnostics.is_empty());

    let response = (
        StatusCode::CREATED,
        pons_axum::extract::Json(User {
            id: 1,
            name: "Ada".into(),
        }),
    )
        .into_response();
    assert_eq!(response.status(), StatusCode::CREATED);
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
fn documented_layer_applies_auth_to_routes_added_after_it() {
    fn router() -> Router<AppState> {
        DocumentedLayer::new(axum::middleware::from_fn(noop_layer), SessionAuth)
            .layer(Router::<AppState>::new(info()).route(
                "/before",
                get_with(list_users, OperationMeta::new("session.before")),
            ))
            .route(
                "/after",
                get_with(list_users, OperationMeta::new("session.after")),
            )
    }
    let document = router()
        .auth_scheme("session", AuthScheme::Bearer { format: None })
        .into_document()
        .unwrap();
    assert_eq!(document.operations.len(), 2);
    for operation in &document.operations {
        assert_eq!(
            operation.auth.alternatives,
            vec![vec!["session".to_owned()]]
        );
        assert!(!operation.auth.optional);
    }
    let snapshot = router().document();
    assert_eq!(snapshot.operations.len(), 2);
    for operation in &snapshot.operations {
        assert_eq!(
            operation.auth.alternatives,
            vec![vec!["session".to_owned()]]
        );
    }
}

#[test]
fn documented_layer_enforces_auth_on_routes_added_after_it() {
    fn router() -> Router<()> {
        DocumentedLayer::new(
            axum::middleware::from_fn(require_authorization),
            SessionAuth,
        )
        .layer(Router::<AppState>::new(info()).route(
            "/before",
            get_with(list_users, OperationMeta::new("session.before")),
        ))
        .route(
            "/after",
            get_with(list_users, OperationMeta::new("session.after")),
        )
        .auth_scheme("session", AuthScheme::Bearer { format: None })
        .with_state::<()>(AppState)
    }

    let document = router().into_document().unwrap();
    assert_eq!(document.operations.len(), 2);
    for id in ["before", "after"] {
        let operation = document
            .operations
            .iter()
            .find(|operation| operation.id.last().unwrap().join("_") == id)
            .unwrap();
        assert_eq!(
            operation.auth.alternatives,
            vec![vec!["session".to_owned()]],
            "/{id} must be documented as authenticated"
        );
    }

    let mut app = router().into_axum();
    for path in ["/before", "/after"] {
        assert_eq!(get_status(&mut app, path, false), StatusCode::UNAUTHORIZED);
        assert_eq!(get_status(&mut app, path, true), StatusCode::OK);
    }
}

#[test]
fn pending_layers_are_applied_once_each_in_registration_order() {
    fn router() -> Router<AppState> {
        DocumentedLayer::new(axum::middleware::from_fn(trace_layer_b), SessionAuth)
            .layer(
                DocumentedLayer::new(axum::middleware::from_fn(trace_layer_a), SessionAuth).layer(
                    Router::<AppState>::new(info()).route(
                        "/traced",
                        get_with(list_users, OperationMeta::new("session.traced")),
                    ),
                ),
            )
            .route(
                "/traced-after",
                get_with(list_users, OperationMeta::new("session.traced_after")),
            )
    }
    fn doubled() -> Router<AppState> {
        DocumentedLayer::new(axum::middleware::from_fn(trace_layer_a), SessionAuth)
            .layer(
                DocumentedLayer::new(axum::middleware::from_fn(trace_layer_a), SessionAuth).layer(
                    Router::<AppState>::new(info()).route(
                        "/doubled",
                        get_with(list_users, OperationMeta::new("session.doubled")),
                    ),
                ),
            )
            .route(
                "/doubled-after",
                get_with(list_users, OperationMeta::new("session.doubled_after")),
            )
    }

    LAYER_TRACE.lock().unwrap().clear();
    let mut app = router().with_state::<()>(AppState).into_axum();
    assert_eq!(get_status(&mut app, "/traced-after", true), StatusCode::OK);
    assert_eq!(LAYER_TRACE.lock().unwrap().as_slice(), ["b", "a"]);

    LAYER_TRACE.lock().unwrap().clear();
    let mut app = doubled().with_state::<()>(AppState).into_axum();
    assert_eq!(get_status(&mut app, "/doubled-after", true), StatusCode::OK);
    assert_eq!(LAYER_TRACE.lock().unwrap().as_slice(), ["a", "a"]);
}

#[test]
fn wildcard_path_segments_are_diagnosed_instead_of_claiming_a_plain_param() {
    let router = Router::<()>::new(info()).route("/files/{*rest}", get(get_file));
    let document = router.document();
    let operation = &document.operations[0];
    let PathSegment::Param { param, ty } = &operation.path[1] else {
        panic!();
    };
    assert_eq!(param, "rest");
    assert!(matches!(ty, TypeRef::Inline(InlineType::String { .. })));
    assert!(operation.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "PONS-W102" && diagnostic.message.contains("wildcard")
    }));
    assert!(router.into_document().is_ok());
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

#[test]
#[should_panic(expected = "Overlapping method route")]
fn a_replaced_method_endpoint_is_refused_instead_of_described_twice() {
    let router = Router::<()>::new(info()).route("/replaced", post(first_user).post(second_user));
    let _ = router.document();
}

#[test]
#[should_panic(expected = "Overlapping method route")]
fn a_second_route_for_the_same_path_and_method_is_refused() {
    let router = Router::<()>::new(info())
        .route("/twice", get(first_user))
        .route("/twice", get(second_user));
    let _ = router.document();
}
