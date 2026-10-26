mod support;

use axum::body::Body;
use pons_axum::http::{Request, StatusCode};
use pons_axum::routing::{get, head_with};
use pons_axum::{NoContent, OperationMeta, Router};
use pons_ir::TypeRef;
use support::validation::*;
use support::*;

async fn form_on_get(pons_axum::Form(form): pons_axum::Form<SlugForm>) -> axum::Json<User> {
    axum::Json(User {
        id: form.slug.0.len() as u64,
    })
}

async fn raw_form_on_get(pons_axum::extract::RawForm(form): pons_axum::extract::RawForm) -> String {
    String::from_utf8_lossy(&form).into_owned()
}

async fn form_on_head(pons_axum::Form(form): pons_axum::Form<SlugForm>) -> NoContent {
    let _ = form.slug.0;
    NoContent
}

async fn scalar_form_on_get(pons_axum::Form(form): pons_axum::Form<u64>) -> axum::Json<User> {
    axum::Json(User { id: form })
}

#[test]
fn a_form_on_a_get_is_published_as_the_query_the_runtime_reads() {
    let document = Router::<()>::new(info())
        .route("/form", get(form_on_get))
        .into_document()
        .unwrap();
    let operation = &document.operations[0];
    assert!(
        operation.body.is_none(),
        "a form on a GET is read from the query, so the document publishes no request body"
    );
    let TypeRef::Named(form) = operation.query.as_ref().unwrap() else {
        panic!("a typed form is published as a named type")
    };
    assert!(
        form.id.ends_with("#form"),
        "the runtime resolves the carrier the document publishes: {}",
        form.id
    );

    let mut app = built(Router::<()>::new(info()).route("/form", get(form_on_get)));
    let (status, body) = send(&mut app, get_request("/form?slug=ada-lovelace"));
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(
        body.contains("\"id\":12"),
        "{body} must be read from the query the document publishes"
    );

    let errors = refused(Router::<()>::new(info()).route("/raw-form", get(raw_form_on_get)));
    assert!(
        errors.contains("PONS-E102") && errors.contains("query"),
        "{errors} must refuse a query this contract cannot publish opaquely"
    );
}

#[test]
fn a_form_on_a_head_is_published_as_the_query_the_runtime_reads() {
    fn router() -> Router<()> {
        Router::new(info()).route(
            "/head-form",
            head_with(form_on_head, OperationMeta::new("head.form")),
        )
    }
    let document = router().into_document().unwrap();
    let operation = &document.operations[0];
    assert!(
        operation.body.is_none(),
        "a form on a HEAD is read from the query, so the document publishes no request body: {:?}",
        operation.body
    );
    let TypeRef::Named(form) = operation.query.as_ref().unwrap() else {
        panic!("a typed form is published as a named type")
    };
    assert!(
        form.id.ends_with("#form"),
        "the runtime resolves the carrier the document publishes: {}",
        form.id
    );

    let mut app = built(router());
    let (status, body) = send(
        &mut app,
        Request::builder()
            .method("HEAD")
            .uri("/head-form?slug=ada-lovelace")
            .body(Body::empty())
            .unwrap(),
    );
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
    let (status, body) = send(
        &mut app,
        Request::builder()
            .method("HEAD")
            .uri("/head-form")
            .body(Body::empty())
            .unwrap(),
    );
    assert_eq!(
        status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "a HEAD the query does not satisfy is rejected as the report the operation declares: {body}"
    );

    let errors = refused(Router::new(info()).route(
        "/raw-head-form",
        head_with(raw_form_on_get, OperationMeta::new("head.raw_form")),
    ));
    assert!(
        errors.contains("PONS-E102") && errors.contains("struct"),
        "{errors} must refuse the opaque form type, whose fields a query cannot spell"
    );
}

#[test]
fn a_form_that_becomes_a_query_must_be_a_struct() {
    let errors = refused(Router::new(info()).route("/scalar", get(scalar_form_on_get)));
    assert!(
        errors.contains("PONS-E102") && errors.contains("struct"),
        "{errors} must name the struct the query slot a GET form becomes has to be"
    );
    assert!(
        !errors.contains("use a typed form"),
        "{errors} must not tell a user who already used a typed form to use one"
    );
}

#[test]
fn a_multipart_body_on_a_get_is_refused_because_axum_answers_400_first() {
    let errors = refused(Router::<()>::new(info()).route("/upload", get(upload)));
    assert!(
        errors.contains("PONS-E102") && errors.contains("boundary"),
        "{errors} must name the boundary a GET request never carries"
    );

    let mut app = axum::Router::new().route("/upload", axum::routing::get(upload));
    let (status, body) = send(&mut app, get_request("/upload"));
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(
        body.contains("boundary"),
        "{body} is the rejection the refusal names"
    );
}

#[test]
fn a_multipart_body_is_opaque_so_it_is_never_claimed_to_be_checked() {
    let document = router().into_document().unwrap();
    let upload = operation(&document, "upload");
    assert!(upload.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "PONS-W101" && diagnostic.message.contains("multipart fields are opaque")
    }));
    assert!(upload.errors.is_empty());
}
