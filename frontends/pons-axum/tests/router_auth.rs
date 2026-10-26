mod support;

use axum::response::IntoResponse;
use pons_axum::http::StatusCode;
use pons_axum::routing::{get, get_with};
use pons_axum::{
    Auth, AuthSpec, DescribeRequest, DocumentedLayer, OperationCx, OperationMeta, Router,
    SessionAuth,
};
use pons_ir::{ApiKeyLocation, AuthScheme};
use support::router::*;
use support::*;

async fn authenticated(auth: Auth<Provides>) -> axum::Json<User> {
    let _ = auth.into_inner();
    axum::Json(User {
        id: 1,
        name: "Ada".into(),
    })
}

struct MaybeSession;

impl<S: Send + Sync> pons_axum::extract::FromRequestParts<S> for MaybeSession {
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(
        _parts: &mut axum::http::request::Parts,
        _: &S,
    ) -> Result<Self, Self::Rejection> {
        Ok(MaybeSession)
    }
}

impl DescribeRequest for MaybeSession {
    fn describe(cx: &mut OperationCx) {
        cx.require_all(["session"]);
        cx.optional_auth();
    }
}

async fn maybe_session(_session: MaybeSession) -> axum::Json<User> {
    axum::Json(User {
        id: 1,
        name: "Ada".into(),
    })
}

struct TenantAuth;

impl AuthSpec for TenantAuth {
    fn apply(cx: &mut OperationCx) {
        cx.require_all(["tenant"]);
    }
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

async fn require_tenant(
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    if request.headers().contains_key("x-tenant") {
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

fn status_with_header(
    app: &mut axum::Router<()>,
    path: &str,
    name: &str,
    value: &str,
) -> StatusCode {
    request_status(app, path, Some((name, value)))
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
fn optional_auth_survives_a_router_that_says_nothing_about_auth() {
    let document = Router::<AppState>::new(info())
        .auth_scheme("session", AuthScheme::Bearer { format: None })
        .route("/maybe", get(maybe_session))
        .into_document()
        .unwrap();
    let operation = &document.operations[0];
    assert_eq!(
        operation.auth.alternatives,
        vec![vec!["session".to_owned()]]
    );
    assert!(
        operation.auth.optional,
        "a router with no auth layer says nothing about auth, so it must not make the operation required"
    );

    let document = DocumentedLayer::new(axum::middleware::from_fn(noop_layer), SessionAuth)
        .layer(Router::<AppState>::new(info()).route("/maybe", get(maybe_session)))
        .auth_scheme("session", AuthScheme::Bearer { format: None })
        .into_document()
        .unwrap();
    assert!(
        !document.operations[0].auth.optional,
        "a layer that requires a session overrides the operation that treats it as optional"
    );
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
fn a_group_layer_covers_only_its_group() {
    fn router() -> Router<()> {
        let group = DocumentedLayer::new(
            axum::middleware::from_fn(require_authorization),
            SessionAuth,
        )
        .layer(
            Router::<AppState>::new(info())
                .route(
                    "/group/first",
                    get_with(list_users, OperationMeta::new("group.first")),
                )
                .route(
                    "/group/second",
                    get_with(list_users, OperationMeta::new("group.second")),
                ),
        );
        Router::<AppState>::new(info())
            .merge(group)
            .route(
                "/open",
                get_with(list_users, OperationMeta::new("group.open")),
            )
            .auth_scheme("session", AuthScheme::Bearer { format: None })
            .with_state::<()>(AppState)
    }

    let document = router().into_document().unwrap();
    for (path, authenticated) in [("group", true), ("open", false)] {
        let operation = operation(&document, path);
        let expected = if authenticated {
            vec![vec!["session".to_owned()]]
        } else {
            Vec::new()
        };
        assert_eq!(
            operation.auth.alternatives,
            expected,
            "{} must be documented as authenticated only inside its group",
            operation.id.last().unwrap().join("_")
        );
    }

    let mut app = built(router());
    for path in ["/group/first", "/group/second"] {
        assert_eq!(get_status(&mut app, path, false), StatusCode::UNAUTHORIZED);
        assert_eq!(get_status(&mut app, path, true), StatusCode::OK);
    }
    assert_eq!(
        get_status(&mut app, "/open", false),
        StatusCode::OK,
        "the layer on the group must not run for a sibling route"
    );
}

#[test]
fn a_global_layer_covers_every_route() {
    fn router() -> Router<()> {
        DocumentedLayer::new(
            axum::middleware::from_fn(require_authorization),
            SessionAuth,
        )
        .layer(
            Router::<AppState>::new(info())
                .route(
                    "/first",
                    get_with(list_users, OperationMeta::new("global.first")),
                )
                .route(
                    "/second",
                    get_with(list_users, OperationMeta::new("global.second")),
                ),
        )
        .auth_scheme("session", AuthScheme::Bearer { format: None })
        .with_state::<()>(AppState)
    }

    let document = router().into_document().unwrap();
    for operation in &document.operations {
        assert_eq!(
            operation.auth.alternatives,
            vec![vec!["session".to_owned()]]
        );
    }

    let mut app = built(router());
    for path in ["/first", "/second"] {
        assert_eq!(get_status(&mut app, path, false), StatusCode::UNAUTHORIZED);
        assert_eq!(get_status(&mut app, path, true), StatusCode::OK);
    }
}

#[test]
fn merged_and_nested_groups_keep_their_own_layers() {
    fn router() -> Router<()> {
        let api = DocumentedLayer::new(
            axum::middleware::from_fn(require_authorization),
            SessionAuth,
        )
        .layer(Router::<AppState>::new(info()).route(
            "/users",
            get_with(list_users, OperationMeta::new("api.users")),
        ));
        let teams = DocumentedLayer::new(axum::middleware::from_fn(require_tenant), TenantAuth)
            .layer(Router::<AppState>::new(info()).route(
                "/teams",
                get_with(list_users, OperationMeta::new("teams.list")),
            ));
        Router::<AppState>::new(info())
            .nest("/api", api)
            .merge(teams)
            .route(
                "/open",
                get_with(list_users, OperationMeta::new("open.list")),
            )
            .auth_scheme("session", AuthScheme::Bearer { format: None })
            .auth_scheme(
                "tenant",
                AuthScheme::ApiKey {
                    location: ApiKeyLocation::Header,
                    name: "x-tenant".into(),
                },
            )
            .with_state::<()>(AppState)
    }

    let document = router().into_document().unwrap();
    assert_eq!(
        operation(&document, "api").path,
        vec![literal("api"), literal("users")]
    );
    assert_eq!(
        operation(&document, "api").auth.alternatives,
        vec![vec!["session".to_owned()]]
    );
    assert_eq!(operation(&document, "teams").path, vec![literal("teams")]);
    assert_eq!(
        operation(&document, "teams").auth.alternatives,
        vec![vec!["tenant".to_owned()]]
    );
    assert!(operation(&document, "open").auth.alternatives.is_empty());

    let mut app = built(router());
    assert_eq!(
        get_status(&mut app, "/api/users", false),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(get_status(&mut app, "/api/users", true), StatusCode::OK);
    assert_eq!(
        status_with_header(&mut app, "/teams", "authorization", "Bearer token"),
        StatusCode::UNAUTHORIZED,
        "the session layer on the nested group must not run for a merged sibling"
    );
    assert_eq!(
        status_with_header(&mut app, "/teams", "x-tenant", "acme"),
        StatusCode::OK
    );
    assert_eq!(get_status(&mut app, "/open", false), StatusCode::OK);
}

#[test]
fn layers_keep_their_registration_order_within_a_group() {
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
            .auth_scheme("session", AuthScheme::Bearer { format: None })
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
            .auth_scheme("session", AuthScheme::Bearer { format: None })
    }

    LAYER_TRACE.lock().unwrap().clear();
    let mut app = built(router().with_state::<()>(AppState));
    assert_eq!(get_status(&mut app, "/traced", true), StatusCode::OK);
    assert_eq!(LAYER_TRACE.lock().unwrap().as_slice(), ["b", "a"]);

    LAYER_TRACE.lock().unwrap().clear();
    let mut app = built(doubled().with_state::<()>(AppState));
    assert_eq!(get_status(&mut app, "/doubled", true), StatusCode::OK);
    assert_eq!(LAYER_TRACE.lock().unwrap().as_slice(), ["a", "a"]);
}

#[test]
fn a_route_added_after_a_layer_is_not_covered() {
    fn router() -> Router<()> {
        DocumentedLayer::new(
            axum::middleware::from_fn(require_authorization),
            SessionAuth,
        )
        .layer(Router::<AppState>::new(info()).route(
            "/before",
            get_with(list_users, OperationMeta::new("scoped.before")),
        ))
        .route(
            "/after",
            get_with(list_users, OperationMeta::new("scoped.after")),
        )
        .auth_scheme("session", AuthScheme::Bearer { format: None })
        .with_state::<()>(AppState)
    }

    let document = router().into_document().unwrap();
    assert_eq!(
        operation(&document, "before").auth.alternatives,
        vec![vec!["session".to_owned()]]
    );
    assert!(
        operation(&document, "after").auth.alternatives.is_empty(),
        "a route added after the layer is outside the layer's scope"
    );

    let mut app = built(router());
    assert_eq!(
        get_status(&mut app, "/before", false),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(get_status(&mut app, "/before", true), StatusCode::OK);
    assert_eq!(
        get_status(&mut app, "/after", false),
        StatusCode::OK,
        "the layer must not run for a route added after it"
    );
}

#[test]
fn merging_conflicting_auth_schemes_fails_validation() {
    let errors = Router::<()>::new(info())
        .auth_scheme("session", AuthScheme::Bearer { format: None })
        .merge(Router::<()>::new(info()).auth_scheme(
            "session",
            AuthScheme::ApiKey {
                location: ApiKeyLocation::Header,
                name: "x-session".into(),
            },
        ))
        .into_document()
        .unwrap_err();
    assert!(
        errors
            .iter()
            .any(|error| error.to_string().contains("auth scheme session")),
        "{errors:?}"
    );
}
