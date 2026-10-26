mod support;

use axum::body::Body;
use pons_axum::http::{Request, StatusCode};
use pons_axum::routing::{get_with, post, post_with};
use pons_axum::{Json, OperationMeta, Router};
use pons_core::Schema;
use pons_ir::{ContentKind, InlineType, ObjectOpenness, TypeKind, TypeRef};
use serde::Deserialize;
use serde_json::Value;
use support::validation::*;
use support::*;

#[derive(Deserialize, Schema)]
#[serde(deny_unknown_fields)]
struct Ordered {
    zeta: Retries,
    alpha: Retries,
}

async fn ordered(Json(body): Json<Ordered>) -> axum::Json<User> {
    let _ = (body.zeta.0, body.alpha.0);
    axum::Json(User { id: 1 })
}

async fn note_body(Json(note): Json<Note>) -> axum::Json<User> {
    let _ = note.text;
    axum::Json(User { id: 1 })
}

async fn vnd_note(VndNote(note): VndNote<Note>) -> axum::Json<User> {
    let _ = note.text;
    axum::Json(User { id: 1 })
}

async fn plain_item(Json(note): Json<Note>) -> axum::Json<User> {
    let _ = note.text;
    axum::Json(User { id: 1 })
}

fn openness(document: &pons_ir::Document, id: &str) -> ObjectOpenness {
    let TypeKind::Struct { openness, .. } = &body_type(document, id).definition else {
        panic!("{id} must publish a struct");
    };
    openness.clone()
}

fn ordered_router() -> Router<()> {
    Router::new(info()).route("/ordered", post(ordered))
}

#[test]
fn a_body_string_that_breaks_the_published_pattern_is_rejected() {
    let mut app = built(router());
    rejected(
        &mut app,
        json_request(
            "/users",
            r#"{"slug":"Ada Lovelace","retries":2,"tags":["x"]}"#,
        ),
        &["/slug: string does not match the pattern ^[a-z][a-z0-9-]*$"],
    );
}

#[test]
fn a_body_number_outside_its_published_bounds_is_rejected() {
    let mut app = built(router());
    rejected(
        &mut app,
        json_request("/users", r#"{"slug":"ada","retries":9,"tags":["x"]}"#),
        &["/retries: integer is above its maximum of 5"],
    );
}

#[test]
fn a_body_missing_a_required_field_is_rejected() {
    let mut app = built(router());
    rejected(
        &mut app,
        json_request("/users", r#"{"slug":"ada","tags":["x"]}"#),
        &["/retries: missing required field retries"],
    );
}

#[test]
fn a_body_that_repeats_an_item_of_a_unique_array_is_rejected() {
    let mut app = built(router());
    rejected(
        &mut app,
        json_request("/users", r#"{"slug":"ada","retries":2,"tags":["x","x"]}"#),
        &["/tags: array item 1 repeats item 0"],
    );
}

#[test]
fn an_unknown_property_is_rejected_against_deny_unknown_fields() {
    let mut app = built(router());
    rejected(
        &mut app,
        json_request(
            "/users",
            r#"{"slug":"ada","retries":2,"tags":["x"],"nickname":"ada"}"#,
        ),
        &["/nickname: unknown property nickname"],
    );
}

#[test]
fn a_body_that_is_not_json_is_rejected_by_the_same_declared_response() {
    let mut app = built(router());
    let (status, body) = send(&mut app, json_request("/users", "slug=ada"));
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    let report: Value = serde_json::from_str(&body).unwrap();
    let reported = report["violations"].as_array().unwrap();
    assert_eq!(reported.len(), 1, "{body} must report one unreadable body");
    assert_eq!(reported[0]["path"], "");
    assert!(
        reported[0]["reason"]
            .as_str()
            .unwrap()
            .starts_with("expected a JSON document"),
        "{body} must name the JSON document it could not read"
    );
}

#[test]
fn a_text_body_is_published_as_text_and_is_never_rejected() {
    let document = router().into_document().unwrap();
    let note = operation(&document, "note");
    assert_eq!(note.body.as_ref().unwrap().content, ContentKind::Text);
    assert!(note.errors.is_empty());

    let mut app = built(router());
    for body in ["", "anything at all"] {
        accepted(&mut app, request("/note", "text/plain", body));
    }
}

#[test]
fn a_text_body_the_framework_reads_is_published_as_optional_and_says_what_it_rejects() {
    let document = Router::<()>::new(info())
        .route("/note", post(note))
        .into_document()
        .unwrap();
    let text = operation(&document, "note");
    assert!(
        !text.body.as_ref().unwrap().required,
        "a request with no body at all reaches the handler as an empty string"
    );
    assert!(
        text.errors.is_empty(),
        "the framework reads this body, so nothing pons validates is rejected: {:?}",
        text.errors
    );
    let message = text
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == "PONS-W101")
        .map(|diagnostic| diagnostic.message.as_str())
        .expect("a body the framework reads earns the rejections it can answer");
    assert!(
        message.contains("400") && message.contains("413"),
        "{message} must name the statuses the framework answers before the handler runs"
    );
    assert!(
        message.contains("declares none of"),
        "{message} must agree with what the operation declares"
    );

    let mut app = built(Router::<()>::new(info()).route("/note", post(note)));
    let empty: Request<Body> = Request::builder()
        .method("POST")
        .uri("/note")
        .body(Body::empty())
        .unwrap();
    let (status, body) = send(&mut app, empty);
    assert_eq!(status, StatusCode::OK, "{body}");

    let undecodable: Request<Body> = Request::builder()
        .method("POST")
        .uri("/note")
        .body(Body::from(&[0xff, 0xfe][..]))
        .unwrap();
    let (status, body) = send(&mut app, undecodable);
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");

    let oversized: Request<Body> = Request::builder()
        .method("POST")
        .uri("/note")
        .body(Body::from(vec![b'a'; 3_000_000]))
        .unwrap();
    let (status, body) = send(&mut app, oversized);
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE, "{body}");
}

#[test]
fn a_preserve_opaque_body_checks_its_fields_and_admits_it_accepts_unknown_properties() {
    let document = router().into_document().unwrap();
    assert_eq!(
        openness(
            &document,
            "validation_body::support::validation::OpenUser#input"
        ),
        ObjectOpenness::PreserveOpaque
    );

    let mut app = built(router());
    accepted(
        &mut app,
        json_request("/open-users", r#"{"slug":"ada","nickname":"ada"}"#),
    );
    rejected(
        &mut app,
        json_request("/open-users", r#"{"slug":"Ada"}"#),
        &["/slug: string does not match the pattern ^[a-z][a-z0-9-]*$"],
    );
}

#[test]
fn a_preserve_typed_body_checks_the_extra_properties_it_declares() {
    let document = router().into_document().unwrap();
    let ObjectOpenness::PreserveTyped { value } = openness(
        &document,
        "validation_body::support::validation::OpenRequest#input",
    ) else {
        panic!("the extra properties must be published as typed");
    };
    let TypeRef::Inline(InlineType::Map { value: extra, .. }) = value else {
        panic!("the extra properties must be published as a map of values");
    };
    assert!(matches!(
        *extra,
        TypeRef::Inline(InlineType::Int {
            minimum: Some(_),
            maximum: Some(_),
            ..
        })
    ));

    let mut app = built(router());
    accepted(
        &mut app,
        json_request("/typed-open", r#"{"slug":"ada","extra":2}"#),
    );
    rejected(
        &mut app,
        json_request("/typed-open", r#"{"slug":"ada","extra":9}"#),
        &["/extra: integer is above its maximum of 5"],
    );
}

#[test]
fn a_json_body_keeps_the_nullability_a_query_carrier_lowered() {
    let mut app = built(carriers());

    accepted(
        &mut app,
        json_request("/both-body", r#"{"slug":"ada","limit":null}"#),
    );
    accepted(
        &mut app,
        json_request("/both-body", r#"{"slug":"ada","limit":2}"#),
    );
    rejected(
        &mut app,
        json_request("/both-body", r#"{"slug":"ada","limit":"two"}"#),
        &["/limit: expected a number, found a string"],
    );
}

#[test]
fn a_body_that_announces_another_content_type_is_rejected() {
    let mut app = built(router());
    let (status, body) = send(
        &mut app,
        request(
            "/users",
            "text/plain",
            r#"{"slug":"ada","retries":2,"tags":["x"]}"#,
        ),
    );
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    let report: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(
        report["violations"][0]["reason"],
        "expected application/json, found text/plain"
    );
}

#[test]
fn a_body_that_announces_the_published_content_type_is_accepted() {
    let mut app = built(router());
    for content_type in ["application/json", "application/json; charset=utf-8"] {
        accepted(
            &mut app,
            request(
                "/users",
                content_type,
                r#"{"slug":"ada","retries":2,"tags":["x"]}"#,
            ),
        );
    }
}

#[test]
fn the_runtime_checks_a_request_in_the_order_the_document_publishes() {
    let document = ordered_router().into_document().unwrap();
    let TypeRef::Named(body) = &operation(&document, "ordered").body.as_ref().unwrap().ty else {
        panic!("a JSON body is published as a named type")
    };
    let TypeKind::Struct { fields, .. } = &document.types[&body.id].definition else {
        panic!("a JSON body is published as a struct")
    };
    assert_eq!(
        fields
            .iter()
            .map(|field| field.wire.as_str())
            .collect::<Vec<_>>(),
        ["alpha", "zeta"],
        "the published document sorts the fields it publishes"
    );

    let mut app = built(ordered_router());
    rejected(
        &mut app,
        json_request("/ordered", r#"{"zeta":9,"alpha":9}"#),
        &[
            "/alpha: integer is above its maximum of 5",
            "/zeta: integer is above its maximum of 5",
        ],
    );
}

#[test]
fn the_extract_module_reaches_the_extractors_that_check_the_contract() {
    async fn through_the_module(
        pons_axum::extract::Json(body): pons_axum::extract::Json<Ordered>,
    ) -> axum::Json<User> {
        let _ = (body.zeta.0, body.alpha.0);
        axum::Json(User { id: 1 })
    }
    let mut app = built(Router::<()>::new(info()).route("/module", post(through_the_module)));
    accepted(&mut app, json_request("/module", r#"{"zeta":1,"alpha":2}"#));
    rejected(
        &mut app,
        json_request("/module", r#"{"zeta":9,"alpha":2}"#),
        &["/zeta: integer is above its maximum of 5"],
    );
}

#[test]
fn a_request_that_carries_no_content_type_is_read_rather_than_rejected() {
    let mut app = built(
        Router::<()>::new(info())
            .route(
                "/note",
                get_with(note_body, OperationMeta::new("note.read")),
            )
            .route(
                "/posted",
                post_with(note_body, OperationMeta::new("note.posted")),
            ),
    );

    let (status, body) = send(&mut app, get_request("/note"));
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    let report: Value = serde_json::from_str(&body).unwrap();
    assert!(
        report["violations"][0]["reason"]
            .as_str()
            .unwrap()
            .starts_with("expected a JSON document"),
        "{body} must report the body it could not read, not a content type it never sent"
    );

    let mut announced = get_request("/note");
    announced
        .headers_mut()
        .insert("content-type", "text/plain".parse().unwrap());
    let (status, body) = send(&mut app, announced);
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    let report: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(
        report["violations"][0]["reason"], "expected application/json, found text/plain",
        "a content type the request announces is still the one the document demands"
    );

    let mut wrong_body = json_request("/posted", r#"{"text":"hi"}"#);
    wrong_body
        .headers_mut()
        .insert("content-type", "text/plain".parse().unwrap());
    let (status, body) = send(&mut app, wrong_body);
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    let report: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(
        report["violations"][0]["reason"],
        "expected application/json, found text/plain"
    );

    let unannounced = Request::builder()
        .method("POST")
        .uri("/posted")
        .body(Body::from(r#"{"text":"hi"}"#.to_owned()))
        .unwrap();
    let (status, body) = send(&mut app, unannounced);
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    let report: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(
        report["violations"][0]["reason"], "expected application/json, found ",
        "a payload the request carries is still the one the document demands"
    );
}

#[test]
fn a_body_is_accepted_with_the_media_type_its_operation_declares() {
    let document = Router::<()>::new(info())
        .route("/notes", post(vnd_note))
        .into_document()
        .unwrap();
    let body = operation(&document, "notes").body.as_ref().unwrap();
    assert_eq!(body.content, ContentKind::Json);
    assert_eq!(
        body.media
            .as_ref()
            .map(|media| media.to_string())
            .as_deref(),
        Some("application/vnd.note+json")
    );

    let mut app = built(Router::<()>::new(info()).route("/notes", post(vnd_note)));
    accepted(
        &mut app,
        request("/notes", "application/vnd.note+json", r#"{"text":"hi"}"#),
    );
    let (status, body) = send(
        &mut app,
        request("/notes", "application/json", r#"{"text":"hi"}"#),
    );
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    let report: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(
        report["violations"][0]["reason"],
        "expected application/vnd.note+json, found application/json"
    );
}

#[test]
fn the_media_type_of_the_route_the_router_picks_is_the_one_it_enforces() {
    fn router() -> Router<()> {
        Router::new(info())
            .route("/items/{id}", post(plain_item))
            .route("/items/special", post(vnd_note))
    }
    let document = router().into_document().unwrap();
    assert_eq!(
        operation(&document, "special")
            .body
            .as_ref()
            .unwrap()
            .media
            .as_ref()
            .map(ToString::to_string)
            .as_deref(),
        Some("application/vnd.note+json")
    );

    let mut app = built(router());
    accepted(
        &mut app,
        request(
            "/items/special",
            "application/vnd.note+json",
            r#"{"text":"hi"}"#,
        ),
    );
    let (status, body) = send(
        &mut app,
        request("/items/special", "application/json", r#"{"text":"hi"}"#),
    );
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    let report: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(
        report["violations"][0]["reason"],
        "expected application/vnd.note+json, found application/json",
        "the static route's media type is the one the runtime enforces"
    );
    accepted(
        &mut app,
        request("/items/other", "application/json", r#"{"text":"hi"}"#),
    );
}

#[test]
fn the_media_type_of_the_route_the_router_resolves_is_the_one_it_enforces() {
    fn router() -> Router<()> {
        Router::<()>::new(info())
            .route("/{lead}/x/y", post(plain_item))
            .route("/x/{first}/{second}", post(vnd_note))
    }
    let document = router().into_document().unwrap();
    let ids: Vec<Vec<Vec<String>>> = document
        .operations
        .iter()
        .map(|operation| operation.id.clone())
        .collect();
    assert_eq!(
        ids,
        [
            vec![vec!["plain".to_owned(), "item".to_owned()]],
            vec![vec!["vnd".to_owned(), "note".to_owned()]]
        ],
        "the operation ids sort the other way than the routes resolve"
    );

    let mut app = built(router());
    accepted(
        &mut app,
        request("/x/x/y", "application/vnd.note+json", r#"{"text":"hi"}"#),
    );
    let (status, body) = send(
        &mut app,
        request("/x/x/y", "application/json", r#"{"text":"hi"}"#),
    );
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    let report: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(
        report["violations"][0]["reason"],
        "expected application/vnd.note+json, found application/json",
        "the route axum resolves is the one whose media type is enforced"
    );
    accepted(
        &mut app,
        request("/z/x/y", "application/json", r#"{"text":"hi"}"#),
    );
}
