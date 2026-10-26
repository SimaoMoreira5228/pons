mod support;

use pons_axum::extract::State;
use pons_axum::http::StatusCode;
use pons_axum::routing::{get, get_with, post};
use pons_axum::{DescribeRequest, OperationCx, OperationMeta, Path, Query, Router, WithStatus};
use pons_core::Schema;
use pons_ir::{ContentKind, InlineType, PathSegment, TypeRef};
use serde::Deserialize;
use support::router::*;
use support::*;
use tower_service::Service;

#[derive(Deserialize, Schema)]
struct FilePath {
    rest: String,
}

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

async fn get_file(Path(path): Path<FilePath>) -> axum::Json<User> {
    let _ = path.rest;
    axum::Json(User {
        id: 1,
        name: "Ada".into(),
    })
}

struct VendorJson<T>(pub T);

impl<T, S> axum::extract::FromRequest<S> for VendorJson<T>
where
    T: Schema + serde::de::DeserializeOwned,
    S: Send + Sync,
{
    type Rejection = pons_axum::ViolationReport;

    async fn from_request(
        request: pons_axum::extract::Request,
        state: &S,
    ) -> Result<Self, Self::Rejection> {
        pons_axum::Json::<T>::from_request(request, state)
            .await
            .map(|pons_axum::Json(input)| VendorJson(input))
    }
}

impl<T: Schema> DescribeRequest for VendorJson<T> {
    fn describe(cx: &mut OperationCx) {
        cx.request_body_with_media::<T>(
            ContentKind::Json,
            Some("application/vnd.user+json".into()),
        );
        cx.mark_validated();
    }
}

async fn create_user(
    pons_axum::Json(input): pons_axum::Json<CreateUser>,
) -> WithStatus<201, axum::Json<User>> {
    let name = input.name;
    WithStatus::new(axum::Json(User { id: 1, name }))
}

async fn create_vendor_json(VendorJson(input): VendorJson<CreateUser>) -> axum::Json<User> {
    axum::Json(User {
        id: 1,
        name: input.name,
    })
}

async fn get_limit_path(Path(limit): Path<u64>) -> axum::Json<User> {
    let _ = limit;
    axum::Json(User {
        id: 1,
        name: "Ada".into(),
    })
}

fn post_json(app: &mut axum::Router<()>, path: &str, media_type: &str) -> (StatusCode, String) {
    let request: pons_axum::extract::Request = pons_axum::http::Request::builder()
        .method("POST")
        .uri(path)
        .header("content-type", media_type)
        .body(axum::body::Body::from(r#"{"name":"Ada"}"#))
        .unwrap();
    let response = block_on(app.call(request)).unwrap();
    let status = response.status();
    let body = block_on(axum::body::to_bytes(response.into_body(), usize::MAX)).unwrap();
    (status, String::from_utf8_lossy(&body).into_owned())
}

#[test]
fn nesting_prefixes_paths_and_keeps_path_types() {
    fn router() -> Router<()> {
        let nested = Router::<AppState>::new(info())
            .route("/users/{id}", get(get_user))
            .route("/", get(list_users));
        Router::<AppState>::new(info())
            .nest("/api", nested)
            .nest(
                "/v1/",
                Router::<AppState>::new(info()).route("/health", get(health)),
            )
            .with_state::<()>(AppState)
    }

    let document = router().into_document().unwrap();
    let user = document
        .operations
        .iter()
        .find(|operation| {
            operation
                .path
                .iter()
                .any(|segment| matches!(segment, PathSegment::Param { param, .. } if param == "id"))
        })
        .unwrap();
    assert_eq!(user.path.len(), 3);
    assert_eq!(user.path[0], literal("api"));
    assert_eq!(user.path[1], literal("users"));
    let PathSegment::Param { param, ty } = &user.path[2] else {
        panic!();
    };
    assert_eq!(param, "id");
    assert!(matches!(
        ty,
        TypeRef::Inline(InlineType::Int { bits: 64, .. })
    ));
    let root = document
        .operations
        .iter()
        .find(|operation| operation.path == vec![literal("api")])
        .expect("a nested root route is served at the prefix itself");
    assert_eq!(root.method, pons_ir::Method::Get);
    assert!(
        document
            .operations
            .iter()
            .any(|operation| operation.path == vec![literal("v1"), literal("health")])
    );

    let mut app = built(router());
    assert_eq!(get_status(&mut app, "/api/users/1", false), StatusCode::OK);
    assert_eq!(get_status(&mut app, "/api", false), StatusCode::OK);
    assert_eq!(get_status(&mut app, "/v1/health", false), StatusCode::OK);
}

#[test]
fn a_nest_prefix_the_document_cannot_carry_is_refused() {
    for (prefix, expected) in [
        ("/api/{tenant}", "contains a parameter"),
        ("/api/{*rest}", "contains a wildcard"),
        ("/", "use Router::merge"),
    ] {
        let router = Router::<()>::new(info()).nest(
            prefix,
            Router::<()>::new(info()).route("/health", get(health)),
        );
        let document = router.document();
        assert!(
            document
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.message.contains(expected)),
            "the prefix {prefix:?} must be refused with a message that names the reason"
        );
    }
    assert!(
        Router::<()>::new(info())
            .nest(
                "/api/{tenant}",
                Router::<()>::new(info()).route("/health", get(health))
            )
            .into_document()
            .is_err()
    );
}

#[test]
fn merging_conflicting_operation_ids_fails_validation() {
    let errors = Router::<()>::new(info())
        .route(
            "/first",
            get_with(health, OperationMeta::new("duplicate.operation")),
        )
        .merge(Router::<()>::new(info()).route(
            "/second",
            get_with(health, OperationMeta::new("duplicate.operation")),
        ))
        .into_document()
        .unwrap_err();
    assert!(
        errors
            .iter()
            .any(|error| error.to_string().contains("duplicate operation id")),
        "{errors:?}"
    );
}

#[test]
fn a_wildcard_path_segment_is_refused_instead_of_claiming_a_plain_param() {
    let document = Router::<()>::new(info())
        .route("/files/{*rest}", get(get_file))
        .document();
    let operation = &document.operations[0];
    let PathSegment::Param { param, ty } = &operation.path[1] else {
        panic!();
    };
    assert_eq!(param, "rest");
    assert!(matches!(ty, TypeRef::Inline(InlineType::String { .. })));
    assert!(
        Router::<()>::new(info())
            .route("/files/{*rest}", get(get_file))
            .into_document()
            .is_err(),
        "a route whose wildcard the IR cannot describe must not be published"
    );
}

#[test]
fn a_trailing_slash_route_is_published_at_the_path_axum_serves() {
    fn router() -> Router<()> {
        Router::<()>::new(info()).route("/files/", post(create_vendor_json))
    }
    assert_eq!(
        router().into_document().unwrap().operations[0].path,
        vec![literal("files"), literal("")],
        "axum serves /files and /files/ as two routes, so the published path keeps the empty segment"
    );

    let mut app = built(router());
    let (status, body) = post_json(&mut app, "/files/", "application/vnd.user+json");
    assert_eq!(
        status,
        StatusCode::OK,
        "the media type the operation declares is the one the runtime enforces: {body}"
    );
    let (status, body) = post_json(&mut app, "/files/", "application/json");
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert_eq!(
        post_json(&mut built(router()), "/files", "application/vnd.user+json").0,
        StatusCode::NOT_FOUND,
        "the route the document publishes is the one the server serves, and no other"
    );
}

#[test]
fn a_trailing_slash_route_and_its_bare_sibling_cannot_be_confused() {
    fn router() -> Router<()> {
        Router::<()>::new(info())
            .route("/files", post(create_user))
            .route("/files/", post(create_vendor_json))
    }
    let document = router().into_document().unwrap();
    assert_eq!(document.operations[0].path, vec![literal("files")]);
    assert_eq!(
        document.operations[1].path,
        vec![literal("files"), literal("")],
        "two routes differ by a trailing slash, so they publish different paths"
    );

    let mut app = built(router());
    assert_eq!(
        post_json(&mut app, "/files", "application/json").0,
        StatusCode::CREATED
    );
    assert_eq!(
        post_json(&mut app, "/files/", "application/vnd.user+json").0,
        StatusCode::OK
    );
    for (path, media_type) in [
        ("/files", "application/vnd.user+json"),
        ("/files/", "application/json"),
    ] {
        let (status, body) = post_json(&mut app, path, media_type);
        assert_eq!(
            status,
            StatusCode::UNPROCESSABLE_ENTITY,
            "the runtime resolves {path} to its own operation, which refuses {media_type}: {body}"
        );
    }
}

#[test]
fn an_interior_empty_segment_is_refused_instead_of_naming_another_route() {
    let document = Router::<()>::new(info())
        .route("/a//b", get(first_user))
        .route("/a/b", get(second_user))
        .document();
    let interior = &document.operations[0];
    assert_eq!(
        interior.path,
        vec![literal("a"), literal(""), literal("b")],
        "the published path still names the route the server serves"
    );
    assert!(interior.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "PONS-E102"
            && diagnostic.severity == pons_ir::DiagnosticSeverity::Error
            && diagnostic.message.contains("/a//b")
            && diagnostic.message.contains("empty path segment")
    }));
    assert_eq!(
        document.operations[1].path,
        vec![literal("a"), literal("b")],
        "the two routes publish different paths, so they cannot collapse onto one"
    );
    assert!(
        Router::<()>::new(info())
            .route("/a//b", get(first_user))
            .into_document()
            .is_err(),
        "a route whose interior empty segment no published path names must not be published"
    );
}

#[test]
fn an_escaped_brace_segment_is_published_as_the_literal_axum_serves() {
    fn router() -> Router<()> {
        Router::<()>::new(info()).route("/{{literal}}/{limit}", get(get_limit_path))
    }
    let document = router().into_document().unwrap();
    let operation = &document.operations[0];
    assert_eq!(
        operation.path[0],
        literal("{literal}"),
        "matchit unescapes the doubled braces to the static segment a client requests, so it is a literal and not a parameter"
    );
    let PathSegment::Param { param, ty } = &operation.path[1] else {
        panic!("the second segment is a parameter: {:?}", operation.path);
    };
    assert_eq!(param, "limit");
    assert!(
        matches!(ty, TypeRef::Inline(InlineType::Int { bits: 64, .. })),
        "the escaped literal is not a parameter, so the one Path extractor still names the one parameter: {ty:?}"
    );
    assert!(
        !operation
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("typed as strings"))
    );

    assert_eq!(
        get_status(&mut built(router()), "/{literal}/7", false),
        StatusCode::OK,
        "the published path and the path the server serves are the same request"
    );
    assert_eq!(
        get_status(&mut built(router()), "/{{literal}}/7", false),
        StatusCode::NOT_FOUND
    );
}
