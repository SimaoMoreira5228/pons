use std::collections::BTreeMap;

use actix_web::middleware::DefaultHeaders;
use actix_web::test;
use actix_web::test::TestRequest;
use pons_actix::{DocumentedLayer, Info, OperationMeta, Router, SessionAuth, routing::get_with};
use pons_ir::AuthScheme;

fn info() -> Info {
    Info {
        title: "Scopes".into(),
        version: "1".into(),
        description: None,
        servers: Vec::new(),
        ext: BTreeMap::new(),
    }
}

async fn open() -> String {
    "ok".into()
}

fn router() -> Router {
    let group = Router::<()>::new(info())
        .route("/group", get_with(open, OperationMeta::new("group.view")))
        .route(
            "/group/other",
            get_with(open, OperationMeta::new("group.other")),
        );
    DocumentedLayer::new(DefaultHeaders::new().add(("x-scope", "group")), SessionAuth)
        .scope(group)
        .auth_scheme("session", AuthScheme::Bearer { format: None })
        .route("/open", get_with(open, OperationMeta::new("open.view")))
}

fn global() -> Router {
    DocumentedLayer::new(
        DefaultHeaders::new().add(("x-scope", "global")),
        SessionAuth,
    )
    .global(router())
}

#[actix_web::test]
async fn a_scoped_layer_covers_only_its_group() {
    let document = router()
        .into_document()
        .expect("the scoped router describes a contract");
    for operation in &document.operations {
        let id = operation
            .id
            .iter()
            .flatten()
            .cloned()
            .collect::<Vec<_>>()
            .join(".");
        let authenticated = !operation.auth.alternatives.is_empty();
        assert_eq!(authenticated, id.starts_with("group."), "{id}");
    }

    let app = test::init_service(router().into_actix().expect("the router validates")).await;
    let grouped = test::call_service(&app, TestRequest::get().uri("/group").to_request()).await;
    assert_eq!(grouped.headers().get("x-scope").unwrap(), "group");
    let open = test::call_service(&app, TestRequest::get().uri("/open").to_request()).await;
    assert!(open.headers().get("x-scope").is_none());
}

#[actix_web::test]
async fn a_global_layer_covers_every_operation() {
    let document = global()
        .into_document()
        .expect("the global layer describes a contract");
    assert!(
        document
            .operations
            .iter()
            .all(|operation| !operation.auth.alternatives.is_empty())
    );

    let app = test::init_service(global().into_actix().expect("the router validates")).await;
    for path in ["/group", "/group/other", "/open"] {
        let response = test::call_service(&app, TestRequest::get().uri(path).to_request()).await;
        assert!(response.headers().get("x-scope").is_some());
    }
}

#[actix_web::test]
async fn nested_routes_prefix_both_the_document_and_the_runtime_path() {
    let nested =
        Router::<()>::new(info()).route("/users", get_with(open, OperationMeta::new("users.view")));
    let router = Router::<()>::new(info()).nest("/api", nested);
    let document = router
        .into_document()
        .expect("the nested router describes a contract");
    assert_eq!(
        document.operations[0].path,
        vec![
            pons_ir::PathSegment::Literal { lit: "api".into() },
            pons_ir::PathSegment::Literal {
                lit: "users".into()
            },
        ]
    );

    let nested =
        Router::<()>::new(info()).route("/users", get_with(open, OperationMeta::new("users.view")));
    let app = test::init_service(
        Router::<()>::new(info())
            .nest("/api", nested)
            .into_actix()
            .expect("the nested router validates"),
    )
    .await;
    let response =
        test::call_service(&app, TestRequest::get().uri("/api/users").to_request()).await;
    assert_eq!(response.status(), actix_web::http::StatusCode::OK);
}

#[actix_web::test]
async fn a_parameter_prefix_is_refused() {
    let nested =
        Router::<()>::new(info()).route("/users", get_with(open, OperationMeta::new("users.view")));
    assert!(
        Router::<()>::new(info())
            .nest("/{tenant}", nested)
            .into_document()
            .is_err()
    );
}
