use axum::body::Body;
use axum::response::IntoResponse;
use pons_axum::http::{Request, StatusCode};
use pons_axum::routing::{get, post};
use pons_axum::{HeaderSpec, Info, Query, Router, WithErrorHeader, WithHeader};
use pons_axum::{WithOptionalErrorHeader, WithOptionalHeader};
use pons_core::Schema;
use pons_ir::{ContentKind, ErrorContent, ErrorStatus, Presence};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::future::Future;
use tower_service::Service;

#[derive(Debug, Serialize, Schema)]
struct User {
    id: u64,
}

struct RetryAfter;

impl HeaderSpec for RetryAfter {
    type Value = u16;
    const NAME: &'static str = "Retry-After";
}

struct TraceId;

impl HeaderSpec for TraceId {
    type Value = String;
    const NAME: &'static str = "X-Trace-ID";
}

#[derive(Debug, Serialize, Schema, pons_core::ApiError)]
enum UserError {
    #[pons(status = 429)]
    Limited,
}

impl IntoResponse for UserError {
    fn into_response(self) -> axum::response::Response {
        StatusCode::TOO_MANY_REQUESTS.into_response()
    }
}

#[derive(Debug, Serialize, Schema, pons_core::ApiError)]
enum SlotError {
    #[pons(status = 404)]
    Missing,
    #[pons(status = 409)]
    Conflict,
    #[pons(status = "4xx")]
    Throttled,
    #[pons(status = "default")]
    Unexpected,
}

impl IntoResponse for SlotError {
    fn into_response(self) -> axum::response::Response {
        match self {
            Self::Missing => StatusCode::CONFLICT.into_response(),
            Self::Conflict => StatusCode::OK.into_response(),
            Self::Throttled => StatusCode::TOO_MANY_REQUESTS.into_response(),
            Self::Unexpected => StatusCode::SERVICE_UNAVAILABLE.into_response(),
        }
    }
}

#[derive(Debug, Serialize, Schema, pons_core::ApiError)]
enum OverloadedError {
    #[pons(status = 429)]
    Limited,
}

impl IntoResponse for OverloadedError {
    fn into_response(self) -> axum::response::Response {
        StatusCode::SERVICE_UNAVAILABLE.into_response()
    }
}

#[derive(Debug, Serialize, Schema, pons_core::ApiError)]
enum TwinError {
    #[pons(status = 429)]
    Limited,
    #[pons(status = 409)]
    Conflict,
}

impl IntoResponse for TwinError {
    fn into_response(self) -> axum::response::Response {
        StatusCode::SERVICE_UNAVAILABLE.into_response()
    }
}

#[derive(Debug, Serialize, Schema, pons_core::ApiError)]
enum RedirectError {
    #[pons(status = "default")]
    Moved,
}

impl IntoResponse for RedirectError {
    fn into_response(self) -> axum::response::Response {
        let mut response = StatusCode::FOUND.into_response();
        response.headers_mut().insert(
            pons_axum::http::header::LOCATION,
            "/elsewhere".parse().unwrap(),
        );
        response
    }
}

fn required_error() -> WithErrorHeader<RetryAfter, UserError> {
    WithErrorHeader::new(UserError::Limited, 3)
}

async fn required_header() -> Result<axum::Json<User>, WithErrorHeader<RetryAfter, UserError>> {
    Err(required_error())
}

fn optional_error() -> WithOptionalErrorHeader<RetryAfter, UserError> {
    WithOptionalErrorHeader::new(UserError::Limited, Some(3))
}

async fn optional_header()
-> Result<axum::Json<User>, WithOptionalErrorHeader<RetryAfter, UserError>> {
    Err(optional_error())
}

#[derive(Debug, Serialize, Schema, pons_core::ApiError)]
enum MissingError {
    #[allow(dead_code)]
    #[pons(status = 404)]
    Missing,
}

impl IntoResponse for MissingError {
    fn into_response(self) -> axum::response::Response {
        StatusCode::CONFLICT.into_response()
    }
}

async fn bare_error() -> Result<axum::Json<User>, MissingError> {
    Err(MissingError::Missing)
}

async fn guarded_error() -> Result<axum::Json<User>, WithErrorHeader<TraceId, MissingError>> {
    Err(WithErrorHeader::new(
        MissingError::Missing,
        "trace-1".into(),
    ))
}

#[derive(Deserialize, Schema)]
#[serde(deny_unknown_fields)]
struct TraceQuery {
    trace: String,
}

async fn traced(Query(query): Query<TraceQuery>) -> WithHeader<TraceId, axum::Json<User>> {
    WithHeader::new(axum::Json(User { id: 1 }), query.trace)
}

async fn maybe_traced(
    Query(query): Query<TraceQuery>,
) -> WithOptionalHeader<TraceId, axum::Json<User>> {
    WithOptionalHeader::new(axum::Json(User { id: 1 }), Some(query.trace))
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

fn send(app: &mut axum::Router<()>, uri: &str) -> (StatusCode, String) {
    let request: Request<Body> = Request::builder().uri(uri).body(Body::empty()).unwrap();
    let response = block_on(app.call(request)).unwrap();
    let status = response.status();
    let bytes = block_on(axum::body::to_bytes(response.into_body(), usize::MAX)).unwrap();
    (status, String::from_utf8(bytes.to_vec()).unwrap())
}

fn served_status(router: Router<()>, uri: &str) -> StatusCode {
    let mut app = router
        .into_axum()
        .expect("the fixture router installs a document that validates");
    send(&mut app, uri).0
}

fn router() -> Router<()> {
    Router::<()>::new(info())
        .route("/bare", get(bare_error))
        .route("/guarded", get(guarded_error))
        .route("/traced", get(traced))
        .route("/maybe-traced", get(maybe_traced))
}

fn diagnostic<'a>(operation: &'a pons_ir::Operation, code: &str) -> Option<&'a str> {
    operation
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == code)
        .map(|diagnostic| diagnostic.message.as_str())
}

#[test]
fn a_bare_result_says_the_error_status_it_declares_is_not_enforced() {
    let document = router().into_document().unwrap();
    let bare = &document.operations[0];
    assert_eq!(bare.errors[0].status, ErrorStatus::Exact(404));

    let served = served_status(router(), "/bare");
    assert_eq!(
        served,
        StatusCode::CONFLICT,
        "nothing rewrites the status a bare Result serves, so the document cannot claim 404 alone"
    );
    let message = diagnostic(bare, "PONS-W101")
        .expect("a bare Result must say the statuses it declares are not enforced");
    assert!(
        message.contains("Exact(404)") && message.contains("WithErrorHeader"),
        "{message} must name the status it declares and the wrapper that enforces it"
    );

    let guarded = &document.operations[1];
    assert!(
        diagnostic(guarded, "PONS-W101").is_none(),
        "{:?} wraps the error, so nothing is left unenforced",
        guarded.diagnostics
    );
    assert_eq!(
        served_status(router(), "/guarded"),
        StatusCode::NOT_FOUND,
        "the wrapper serves the status its error type declares"
    );
}

#[test]
fn a_response_header_that_is_no_header_value_degrades_to_a_status_the_document_declares() {
    let document = router().into_document().unwrap();
    for (index, path) in [(2, "traced"), (3, "maybe-traced")] {
        let operation = &document.operations[index];
        let degraded = operation
            .errors
            .iter()
            .find(|error| error.status == ErrorStatus::Exact(500))
            .unwrap_or_else(|| panic!("/{path} declares a response header, so it can degrade"));
        assert_eq!(degraded.content, ErrorContent::Empty);
        assert!(degraded.body.is_none());
    }

    for path in ["/traced", "/maybe-traced"] {
        let served = served_status(router(), &format!("{path}?trace=bad%0Avalue"));
        assert_eq!(
            served,
            StatusCode::INTERNAL_SERVER_ERROR,
            "a value the handler read from the request that is not a header value degrades"
        );
    }
    assert_eq!(
        served_status(router(), "/traced?trace=trace-1"),
        StatusCode::OK
    );
}

#[test]
fn error_header_wrapper_preserves_runtime_and_contract_metadata() {
    let required = required_error().into_response();
    assert_eq!(required.status(), StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(required.headers()["Retry-After"], "3");

    let optional = optional_error().into_response();
    assert_eq!(optional.headers()["Retry-After"], "3");

    let document = Router::<()>::new(info())
        .route("/required", post(required_header))
        .route("/optional", post(optional_header))
        .into_document()
        .unwrap();
    let required_operation = document
        .operations
        .iter()
        .find(|operation| operation.path.iter().any(|segment| matches!(segment, pons_ir::PathSegment::Literal { lit } if lit == "required")))
        .unwrap();
    assert_eq!(required_operation.responses[0].content, ContentKind::Json);
    assert_eq!(required_operation.errors[0].content, ErrorContent::Json);
    assert_eq!(required_operation.errors[0].headers[0].wire, "Retry-After");
    assert_eq!(
        required_operation.errors[0].headers[0].presence,
        Presence::Required
    );

    let optional_operation = document
        .operations
        .iter()
        .find(|operation| operation.path.iter().any(|segment| matches!(segment, pons_ir::PathSegment::Literal { lit } if lit == "optional")))
        .unwrap();
    assert_eq!(
        optional_operation.errors[0].headers[0].presence,
        Presence::Optional
    );
}

#[test]
fn served_error_status_must_be_one_the_error_type_declares() {
    let declared = WithErrorHeader::<RetryAfter, _>::new(SlotError::Missing, 1).into_response();
    assert_eq!(declared.status(), StatusCode::CONFLICT);
    assert!(!declared.headers().contains_key("content-type"));

    let by_default = WithErrorHeader::<RetryAfter, _>::new(SlotError::Conflict, 1).into_response();
    assert_eq!(by_default.status(), StatusCode::OK);

    let declared = WithErrorHeader::<RetryAfter, _>::new(SlotError::Throttled, 1).into_response();
    assert_eq!(declared.status(), StatusCode::TOO_MANY_REQUESTS);

    let declared = WithErrorHeader::<RetryAfter, _>::new(SlotError::Unexpected, 1).into_response();
    assert_eq!(declared.status(), StatusCode::SERVICE_UNAVAILABLE);

    let single = WithErrorHeader::<RetryAfter, _>::new(OverloadedError::Limited, 1).into_response();
    assert_eq!(single.status(), StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(single.headers()["Retry-After"], "1");
    assert!(!single.headers().contains_key("content-type"));

    let none = WithErrorHeader::<RetryAfter, _>::new(TwinError::Limited, 1).into_response();
    assert_eq!(none.status(), StatusCode::INTERNAL_SERVER_ERROR);

    let none = WithErrorHeader::<RetryAfter, _>::new(TwinError::Conflict, 1).into_response();
    assert_eq!(none.status(), StatusCode::INTERNAL_SERVER_ERROR);
}

#[test]
fn the_default_error_status_declares_every_status() {
    let response = WithErrorHeader::<RetryAfter, _>::new(RedirectError::Moved, 1).into_response();
    assert_eq!(response.status(), StatusCode::FOUND);
    assert_eq!(response.headers()["location"], "/elsewhere");
    assert_eq!(response.headers()["Retry-After"], "1");
}

#[test]
fn a_nested_invalid_error_header_is_not_laundered_into_a_typed_error() {
    let inner = WithErrorHeader::<TraceId, _>::new(UserError::Limited, "bad\nvalue".into());
    let response = WithErrorHeader::<RetryAfter, _>::new(inner, 1).into_response();
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert!(!response.headers().contains_key("Retry-After"));
}
