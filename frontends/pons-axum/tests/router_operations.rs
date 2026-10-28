mod support;

use pons_axum::routing::{delete, get, get_with, head, post, post_with};
use pons_axum::{OperationMeta, Query, Router, WithStatus};
use pons_core::Schema;
use pons_ir::{
    AuthScheme, CredentialLocator, JsonPointer, PageSize, Pagination, PaginationRequest,
};
use serde::Deserialize;
use support::router::*;
use support::*;
use tower_service::Service;

fn content_length(app: &mut axum::Router<()>, method: &str, path: &str) -> String {
    let request: pons_axum::extract::Request = pons_axum::http::Request::builder()
        .method(method)
        .uri(path)
        .body(axum::body::Body::empty())
        .unwrap();
    let response = block_on(app.call(request)).unwrap();
    response.headers()["content-length"]
        .to_str()
        .unwrap()
        .to_owned()
}

async fn create_user(
    pons_axum::Json(input): pons_axum::Json<CreateUser>,
) -> WithStatus<201, axum::Json<User>> {
    let name = input.name;
    WithStatus::new(axum::Json(User { id: 1, name }))
}

#[derive(Deserialize, Schema)]
#[serde(deny_unknown_fields)]
struct PageQuery {
    #[serde(default)]
    cursor: Option<String>,
    #[serde(default)]
    limit: Option<u16>,
}

async fn list_page(Query(query): Query<PageQuery>) -> axum::Json<Vec<User>> {
    let _ = (query.cursor, query.limit);
    axum::Json(Vec::new())
}

fn pagination() -> Pagination {
    Pagination {
        request: PaginationRequest::Cursor {
            field: "cursor".into(),
        },
        items: JsonPointer::new("/items"),
        next: Some(JsonPointer::new("/next")),
        page_size: Some(PageSize {
            field: "limit".into(),
            default: Some(20),
            maximum: Some(100),
        }),
        ext: Default::default(),
    }
}

#[test]
fn a_route_axum_also_answers_with_head_says_the_document_describes_only_get() {
    fn head_diagnostic(document: &pons_ir::Document) -> Option<pons_ir::Diagnostic> {
        document.operations.iter().find_map(|operation| {
            operation
                .diagnostics
                .iter()
                .find(|entry| entry.code == "PONS-W103")
                .cloned()
        })
    }
    let get_route = Router::<()>::new(info()).route("/probe", get(first_user));
    let diagnostic =
        head_diagnostic(&get_route.document()).expect("a get route is answered with HEAD");
    assert_eq!(diagnostic.severity, pons_ir::DiagnosticSeverity::Warning);
    assert!(
        diagnostic.message.contains("HEAD") && diagnostic.message.contains("GET body"),
        "the diagnostic names what a HEAD request is served: {}",
        diagnostic.message
    );

    let mut app = built(get_route);
    assert_eq!(
        content_length(&mut app, "HEAD", "/probe"),
        content_length(&mut app, "GET", "/probe"),
        "axum measures the GET body it serves a HEAD request"
    );

    for document in [
        Router::<()>::new(info())
            .route("/probe", head(first_user))
            .document(),
        Router::<()>::new(info())
            .route("/probe", get(first_user).head(second_user))
            .document(),
        Router::<()>::new(info())
            .route("/probe", get(first_user))
            .route("/probe", head(second_user))
            .document(),
    ] {
        assert!(
            head_diagnostic(&document).is_none(),
            "axum serves the head handler the document describes: {:?}",
            document
                .operations
                .iter()
                .map(|operation| (operation.method, operation.diagnostics.len()))
                .collect::<Vec<_>>()
        );
    }

    let apart = Router::<()>::new(info())
        .route("/probe", get(first_user))
        .route("/probe/head", head(second_user))
        .document();
    assert_eq!(
        apart
            .operations
            .iter()
            .filter(|operation| operation
                .diagnostics
                .iter()
                .any(|entry| entry.code == "PONS-W103"))
            .count(),
        1,
        "the GET on a path no HEAD operation answers is the only one axum answers with HEAD"
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
fn operation_meta_overrides_the_semantics_the_method_derives() {
    let document = Router::<()>::new(info())
        .route(
            "/retry",
            post_with(
                create_user,
                OperationMeta::new("users.retry")
                    .safe(true)
                    .idempotent(true),
            ),
        )
        .route(
            "/dedup",
            get_with(
                first_user,
                OperationMeta::new("users.dedup")
                    .safe(false)
                    .idempotent(false),
            ),
        )
        .into_document()
        .unwrap();

    let post = operation(&document, "retry");
    assert!(
        post.semantics.safe && post.semantics.idempotent,
        "a POST the handler serves twice alike is safe to retry: {:?}",
        post.semantics
    );
    let get = operation(&document, "dedup");
    assert!(
        !get.semantics.safe && !get.semantics.idempotent,
        "a GET the handler serves at most once is safe to refuse a repeat: {:?}",
        get.semantics
    );
}

#[test]
fn operation_meta_publishes_the_pagination_it_declares() {
    let document = Router::<()>::new(info())
        .route(
            "/page",
            get_with(
                list_page,
                OperationMeta::new("users.page").pagination(pagination()),
            ),
        )
        .into_document()
        .unwrap();
    let published = operation(&document, "page")
        .semantics
        .pagination
        .as_ref()
        .expect("the operation declares its pagination");
    assert_eq!(
        published.request,
        PaginationRequest::Cursor {
            field: "cursor".into(),
        }
    );
    assert_eq!(published.items.as_str(), "/items");
    assert_eq!(
        published.next.as_ref().map(JsonPointer::as_str),
        Some("/next")
    );
    assert_eq!(
        published.page_size,
        Some(PageSize {
            field: "limit".into(),
            default: Some(20),
            maximum: Some(100),
        })
    );

    let refusal = refused(Router::<()>::new(info()).route(
        "/page",
        get_with(
            list_page,
            OperationMeta::new("users.page").pagination(Pagination {
                request: PaginationRequest::Offset {
                    field: "offset".into(),
                },
                ..pagination()
            }),
        ),
    ));
    assert!(
        refusal
            .contains("pagination field \"offset\" is not declared on the operation query or body"),
        "{refusal}"
    );
}

#[test]
fn an_operation_no_meta_overrides_keeps_the_semantics_its_method_derives() {
    let document = Router::<()>::new(info())
        .route("/users", post(create_user))
        .route("/users", get(first_user))
        .route("/users", delete(delete_user))
        .into_document()
        .unwrap();

    for (method, safe, idempotent) in [
        (pons_ir::Method::Post, false, false),
        (pons_ir::Method::Get, true, true),
        (pons_ir::Method::Delete, false, true),
    ] {
        let operation = document
            .operations
            .iter()
            .find(|operation| operation.method == method)
            .unwrap();
        assert_eq!(
            (operation.semantics.safe, operation.semantics.idempotent),
            (safe, idempotent),
            "a {method:?} operation that overrides nothing keeps the semantics the method gives it"
        );
    }
}

#[test]
fn document_lowering_adds_metadata_and_revalidates() {
    let document = Router::<()>::new(info())
        .try_lower_document(|mut document| {
            document
                .ext
                .insert("com.example.policy/v1".into(), "enabled".into());
            Ok(document)
        })
        .unwrap();
    assert_eq!(
        document.ext["com.example.policy/v1"].as_str(),
        Some("enabled")
    );

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
