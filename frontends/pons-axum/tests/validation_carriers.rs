mod support;

use axum::body::Body;
use pons_axum::http::{Request, StatusCode};
use pons_axum::routing::post;
use pons_axum::{Json, Router};
use pons_core::Schema;
use pons_ir::PathSegment;
use serde::Deserialize;
use serde_json::Value;
use std::collections::BTreeMap;
use support::validation::*;
use support::*;

#[derive(Deserialize, Schema)]
#[serde(rename_all = "snake_case")]
enum Level {
    Info,
    Warn,
}

#[derive(Deserialize, Schema)]
#[serde(deny_unknown_fields)]
struct Levels {
    levels: Vec<Level>,
}

async fn levels(pons_axum::Form(form): pons_axum::Form<Levels>) -> axum::Json<User> {
    let _ = form.levels;
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

#[test]
fn a_query_outside_its_published_bounds_is_rejected() {
    let mut app = built(router());
    rejected(
        &mut app,
        get_request("/search?slug=ada&limit=9"),
        &["/limit: integer is above its maximum of 5"],
    );
    rejected(
        &mut app,
        get_request("/search?limit=2"),
        &["/slug: missing required field slug"],
    );
}

#[test]
fn a_path_parameter_outside_its_published_pattern_is_rejected() {
    let mut app = built(router());
    rejected(
        &mut app,
        get_request("/users/Ada%20Lovelace"),
        &["/slug: string does not match the pattern ^[a-z][a-z0-9-]*$"],
    );
}

#[test]
fn a_form_field_outside_its_published_pattern_is_rejected() {
    let mut app = built(router());
    rejected(
        &mut app,
        request(
            "/form",
            "application/x-www-form-urlencoded",
            "slug=Ada%20Lovelace",
        ),
        &["/slug: string does not match the pattern ^[a-z][a-z0-9-]*$"],
    );
}

#[test]
fn a_header_value_outside_its_published_bounds_is_rejected() {
    let mut app = built(router());
    rejected(
        &mut app,
        header_request("/retried", "x-retries", "9"),
        &[": integer is above its maximum of 5"],
    );
}

#[test]
fn a_header_that_is_not_the_text_token_its_type_reads_is_rejected() {
    let mut app = built(router());
    accepted(&mut app, header_request("/flagged", "x-debug", "true"));
    rejected(
        &mut app,
        header_request("/flagged", "x-debug", "1"),
        &[": expected a boolean, found a string"],
    );
}

#[test]
fn the_default_body_limit_still_guards_the_body_the_validator_reads() {
    let mut app = built(router());
    let oversized = format!(
        r#"{{"slug":"ada","retries":2,"tags":["{}"]}}"#,
        "x".repeat(3_000_000)
    );
    let (status, body) = send(&mut app, json_request("/users", &oversized));
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(
        body.contains("a readable request body"),
        "{body} must report the body it refused to buffer"
    );
}

#[test]
fn a_request_that_satisfies_the_contract_reaches_the_handler() {
    let mut app = built(router());
    for request in [
        json_request("/users", r#"{"slug":"ada","retries":2,"tags":["x","y"]}"#),
        get_request("/search?slug=ada&limit=5"),
        get_request("/search?slug=ada"),
        get_request("/users/ada-lovelace"),
        request(
            "/form",
            "application/x-www-form-urlencoded",
            "slug=ada-lovelace",
        ),
    ] {
        accepted(&mut app, request);
    }
    accepted(&mut app, header_request("/retried", "x-retries", "3"));
}

#[test]
fn a_carrier_newtype_reads_the_struct_it_wraps() {
    let mut app = built(carriers());
    accepted(&mut app, get_request("/newtype-query?cursor=2"));
    accepted(
        &mut app,
        request(
            "/newtype-form",
            "application/x-www-form-urlencoded",
            "cursor=2",
        ),
    );
    rejected(
        &mut app,
        get_request("/newtype-query?cursor=9"),
        &["/cursor: integer is above its maximum of 5"],
    );
    rejected(
        &mut app,
        request(
            "/newtype-form",
            "application/x-www-form-urlencoded",
            "cursor=9",
        ),
        &["/cursor: integer is above its maximum of 5"],
    );
}

#[test]
fn a_newtype_wrapped_flattened_member_reads_its_own_fields() {
    let mut app = built(carriers());
    accepted(&mut app, get_request("/flattened?name=ada&cursor=2"));
    rejected(
        &mut app,
        get_request("/flattened?name=ada&cursor=9"),
        &["/cursor: integer is above its maximum of 5"],
    );
}

#[test]
fn an_array_field_reads_either_spelling_a_carrier_uses() {
    fn form(body: &str) -> Request<Body> {
        request("/tags-form", "application/x-www-form-urlencoded", body)
    }
    let mut app = built(carriers());

    for body in ["tags=1&tags=2", "tags=%5B1%2C2%5D", "tags=1"] {
        accepted(&mut app, form(body));
    }
    for body in ["tags=1&tags=9", "tags=%5B1%2C9%5D", "tags=nine"] {
        let expected: &[&str] = match body {
            "tags=nine" => &["/tags/0: expected a number, found a string"],
            _ => &["/tags/1: integer is above its maximum of 5"],
        };
        rejected(&mut app, form(body), expected);
    }
}

#[test]
fn a_repeated_field_that_is_not_an_array_is_not_reported_as_a_duplicate_array_item() {
    let mut app = built(carriers());
    rejected(
        &mut app,
        get_request("/newtype-query?cursor=1&cursor=2"),
        &["/cursor: expected one value for a field that is not an array, found 2 values"],
    );
    rejected(
        &mut app,
        get_request("/newtype-query?cursor=1&cursor=1&cursor=2"),
        &["/cursor: expected one value for a field that is not an array, found 3 values"],
    );
}

#[test]
fn a_wildcard_path_segment_is_refused_instead_of_published_as_one_parameter() {
    fn router() -> Router<()> {
        Router::<()>::new(info()).route("/files/{*rest}", post(vnd_note))
    }
    let document = router().document();
    let files = operation(&document, "files");
    let diagnostic = files
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == "PONS-E102")
        .expect("a wildcard segment cannot be described, so it must be diagnosed");
    assert_eq!(
        diagnostic.severity,
        pons_ir::DiagnosticSeverity::Error,
        "{diagnostic:?} must be an error, not a warning about a route nobody reads"
    );
    assert!(
        diagnostic.message.contains("wildcard"),
        "{} must name the wildcard it cannot publish",
        diagnostic.message
    );

    let errors = refused(router());
    assert!(
        errors.contains("PONS-E102") && errors.contains("wildcard"),
        "{errors} must refuse a route the IR cannot express"
    );
    assert!(
        refused_build(router()).contains("PONS-E102"),
        "a router that would install a document lying about its own routes must refuse to be built"
    );
}

#[test]
fn a_path_parameter_is_not_the_operation_beyond_its_own_segment() {
    fn router() -> Router<()> {
        Router::<()>::new(info())
            .route("/assets/{id}", post(vnd_note))
            .route("/assets/{id}/raw", post(plain_item))
    }
    let document = router().into_document().unwrap();
    let published: BTreeMap<String, Option<pons_ir::MediaDescriptor>> = document
        .operations
        .iter()
        .map(|operation| {
            let path = operation
                .path
                .iter()
                .map(|segment| match segment {
                    PathSegment::Literal { lit } => lit.clone(),
                    PathSegment::Param { param, .. } => format!("{{{param}}}"),
                })
                .collect::<Vec<_>>()
                .join("/");
            (
                path,
                operation.body.as_ref().and_then(|body| body.media.clone()),
            )
        })
        .collect();
    assert_eq!(
        published.get("assets/{id}"),
        Some(&Some(pons_ir::MediaDescriptor::from_declared(
            "application/vnd.note+json"
        )))
    );
    assert_eq!(
        published.get("assets/{id}/raw"),
        Some(&None),
        "a body that names no media type answers the content kind's, application/json"
    );

    let mut app = built(router());
    accepted(
        &mut app,
        request("/assets/1", "application/vnd.note+json", r#"{"text":"hi"}"#),
    );
    accepted(
        &mut app,
        request("/assets/1/raw", "application/json", r#"{"text":"hi"}"#),
    );
    rejected(
        &mut app,
        request(
            "/assets/1/raw",
            "application/vnd.note+json",
            r#"{"text":"hi"}"#,
        ),
        &[": expected application/json, found application/vnd.note+json"],
    );
}

#[test]
fn both_array_spellings_read_one_element_the_same_way() {
    fn form(body: &str) -> Request<Body> {
        request("/labels-form", "application/x-www-form-urlencoded", body)
    }
    let mut app = built(carriers());

    let numbers: &[&str] = &[
        "/labels/0: expected a string, found a number",
        "/labels/1: expected a string, found a number",
    ];
    rejected(&mut app, form("labels=%5B1%2C2%5D"), numbers);
    rejected(&mut app, form("labels=%5B1%2C2%5D&labels=3"), numbers);
    rejected(
        &mut app,
        form("labels=%5B1%2C2%5D&labels=%5B3%2C4%5D"),
        &[
            "/labels/0: expected a string, found a number",
            "/labels/1: expected a string, found a number",
            "/labels/2: expected a string, found a number",
            "/labels/3: expected a string, found a number",
        ],
    );
    for body in ["labels=a&labels=b", "labels=%5B%22a%22%2C%22b%22%5D"] {
        accepted(&mut app, form(body));
    }
}

#[test]
fn a_string_representation_enum_is_read_as_the_array_item_it_is() {
    let router = || {
        Router::<()>::new(info())
            .route("/tags-form", post(tagged_form))
            .route("/levels-form", post(levels))
    };
    let mut app = built(router());
    let form = |path: &str, body: &str| request(path, "application/x-www-form-urlencoded", body);

    for body in [
        "levels=info",
        "levels=info&levels=warn",
        "levels=%5B%22info%22%5D",
    ] {
        accepted(&mut app, form("/levels-form", body));
    }
    let (status, body) = send(&mut app, form("/levels-form", "levels=info&levels=nope"));
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    let report: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(
        report["violations"][0]["reason"], "unknown enum variant nope",
        "an item a string enum cannot name is reported against the item"
    );
}

#[test]
fn a_repeated_key_reads_an_empty_value_as_the_item_it_spells() {
    let form = |body: &str| request("/tags-form", "application/x-www-form-urlencoded", body);
    let mut app = built(carriers());

    for (body, path) in [
        ("tags=", "/tags/0"),
        ("tags=&tags=1", "/tags/0"),
        ("tags=1&tags=", "/tags/1"),
    ] {
        let (status, text) = send(&mut app, form(body));
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}: {text}");
        let report: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(
            report["violations"][0]["path"], path,
            "{body} spells one empty item, which the integer item type refuses"
        );
    }
    for body in ["tags=%5B%5D", "tags=1&tags=2"] {
        accepted(&mut app, form(body));
    }
}
