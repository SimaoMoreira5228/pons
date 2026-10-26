mod support;

use axum::response::IntoResponse;
use pons_axum::http::StatusCode;
use pons_axum::routing::{head, post};
use pons_axum::{Header, Json, NoContent, Router};
use pons_core::{Schema, TypeCx};
use pons_ir::{ContentKind, Direction, ErrorContent, ErrorStatus, InlineType, TypeKind, TypeRef};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use support::validation::*;
use support::*;

#[derive(Debug, Serialize, Schema, pons_core::ApiError)]
enum DeclaredRejection {
    #[pons(status = 422)]
    Rejected,
}

impl IntoResponse for DeclaredRejection {
    fn into_response(self) -> axum::response::Response {
        StatusCode::UNPROCESSABLE_ENTITY.into_response()
    }
}

async fn declares_its_own_rejection(
    Json(_body): Json<OpenUser>,
) -> Result<axum::Json<User>, DeclaredRejection> {
    Err(DeclaredRejection::Rejected)
}

#[derive(Deserialize)]
struct Unpublished;

impl Schema for Unpublished {
    fn schema(_: &mut TypeCx, _: Direction) -> TypeRef {
        TypeRef::Named(pons_ir::NamedRef {
            id: "unpublished::Type#input".into(),
            variant: None,
        })
    }
}

async fn unchecked_create_user(Json(_body): Json<CreateUser>) -> axum::Json<User> {
    axum::Json(User { id: 1 })
}

async fn unpublished(Json(_body): Json<Unpublished>) -> axum::Json<User> {
    axum::Json(User { id: 1 })
}

fn unpublished_router() -> Router<()> {
    Router::new(info()).route("/unpublished", post(unpublished))
}

fn declared_router() -> Router<()> {
    Router::new(info()).route("/declared", post(declares_its_own_rejection))
}

async fn head_search(pons_axum::Query(_query): pons_axum::Query<Search>) -> NoContent {
    NoContent
}

async fn head_count(Header(_header): Header<RetriesHeader>) -> NoContent {
    NoContent
}

fn head_router() -> Router<()> {
    Router::new(info())
        .route("/head-search", head(head_search))
        .route("/head-count", head(head_count))
}

#[test]
fn every_validated_operation_declares_the_rejection_it_can_serve() {
    let document = router().into_document().unwrap();
    for literal in [
        "users",
        "search",
        "form",
        "retried",
        "flagged",
        "open-users",
        "typed-open",
    ] {
        let rejection = declared_rejection(&document, literal)
            .unwrap_or_else(|| panic!("/{literal} must declare the request rejection"));
        assert_eq!(rejection.status, ErrorStatus::Exact(422));
        assert_eq!(rejection.content, ErrorContent::Json);
        let Some(pons_ir::TypeRef::Named(report)) = &rejection.body else {
            panic!("/{literal} must declare the report body");
        };
        let published = body_type(&document, &report.id);
        assert_eq!(published.direction, Direction::Output);
        let TypeKind::Struct { fields, .. } = &published.definition else {
            panic!("/{literal} must declare a report object");
        };
        assert_eq!(fields[0].wire, "violations");
        let TypeRef::Inline(InlineType::Array { item, .. }) = &fields[0].ty else {
            panic!("/{literal} must declare one violation per entry");
        };
        let TypeRef::Named(violation) = &**item else {
            panic!("/{literal} must publish the shape of one violation");
        };
        let TypeKind::Struct { fields, .. } = &body_type(&document, &violation.id).definition
        else {
            panic!("/{literal} must publish a violation object");
        };
        assert_eq!(fields[0].wire, "path");
        assert_eq!(fields[1].wire, "reason");
    }
}

#[test]
fn an_operation_that_validates_nothing_declares_no_rejection() {
    let document = router().into_document().unwrap();
    for literal in ["note", "upload"] {
        assert!(
            declared_rejection(&document, literal).is_none(),
            "/{literal} validates nothing, so the document must not claim that it does"
        );
    }
    assert_eq!(
        operation(&document, "note").body.as_ref().unwrap().content,
        ContentKind::Text
    );
    assert_eq!(
        operation(&document, "upload")
            .body
            .as_ref()
            .unwrap()
            .content,
        ContentKind::Multipart
    );
}

#[test]
fn a_stream_response_is_never_described_as_validated() {
    let document = router().into_document().unwrap();
    let events = document
        .operations
        .iter()
        .find(|operation| operation.stream.is_some())
        .unwrap();
    assert_eq!(events.errors.len(), 1);
    assert_eq!(events.errors[0].name, ["request", "rejected"]);
    assert_eq!(events.responses.len(), 1);
    assert_eq!(events.responses[0].content, ContentKind::Stream);
    assert!(events.responses[0].body.is_none());
}

#[test]
fn a_type_the_document_does_not_publish_is_refused_instead_of_served_unvalidated() {
    let errors = unpublished_router().into_document().unwrap_err();
    assert!(
        errors
            .iter()
            .any(|error| error.to_string().contains("unpublished::Type#input")),
        "{errors:?} must name the type the handler asks for and never publishes"
    );
    assert!(
        refused_build(unpublished_router()).contains("unpublished::Type#input"),
        "into_axum returns the errors instead of panicking at startup on a document that does not validate"
    );
}

#[test]
fn an_operation_that_declares_the_rejection_status_reuses_that_slot() {
    let document = declared_router().into_document().unwrap();
    let declared = operation(&document, "declared");
    let rejected: Vec<_> = declared
        .errors
        .iter()
        .filter(|error| error.status == ErrorStatus::Exact(422))
        .collect();
    assert_eq!(
        rejected.len(),
        1,
        "{:?} declares the status once",
        declared.errors
    );
    assert_eq!(
        rejected[0].name,
        ["rejected"],
        "the slot the operation declares is kept"
    );
    assert!(
        declared.diagnostics.iter().any(|diagnostic| {
            diagnostic.code == "PONS-W101" && diagnostic.message.contains("\"rejected\"")
        }),
        "{:?} must name the error a request is now rejected as",
        declared.diagnostics
    );
    assert!(document.diagnostics.is_empty());
}

#[test]
fn a_head_operation_declares_the_rejection_without_a_body() {
    let document = head_router().into_document().unwrap();
    for literal in ["head-search", "head-count"] {
        let rejection = declared_rejection(&document, literal)
            .unwrap_or_else(|| panic!("/{literal} must declare the request rejection"));
        assert_eq!(rejection.status, ErrorStatus::Exact(422));
        assert_eq!(rejection.content, ErrorContent::Empty);
        assert!(
            rejection.body.is_none(),
            "/{literal} cannot carry a rejection body"
        );
        assert!(rejection.media.is_none());
    }
}

#[test]
fn every_header_rejection_answers_the_status_the_operation_declares() {
    let document = headers().into_document().unwrap();
    assert_eq!(
        declared_rejection(&document, "retried").map(|rejection| rejection.status),
        Some(ErrorStatus::Exact(422)),
        "a typed header declares the rejection it can serve"
    );

    let mut app = built(headers());

    rejected(
        &mut app,
        get_request("/retried"),
        &[": expected a request carrying the required header, found no header"],
    );

    let mut repeated = get_request("/retried");
    repeated
        .headers_mut()
        .append("x-retries", "1".parse().unwrap());
    repeated
        .headers_mut()
        .append("x-retries", "2".parse().unwrap());
    rejected(
        &mut app,
        repeated,
        &[": expected a header spelled once, found a repeated header"],
    );

    rejected(
        &mut app,
        header_request("/retried", "x-retries", "9"),
        &[": integer is above its maximum of 5"],
    );

    rejected(
        &mut app,
        header_request("/token", "x-token", "Ada"),
        &[": expected a header value the type reads, found a value it cannot read"],
    );
    accepted(&mut app, header_request("/token", "x-token", "ada"));
}

#[test]
fn a_validated_extractor_driven_without_the_document_is_refused() {
    let mut app =
        axum::Router::new().route("/undocumented", axum::routing::post(unchecked_create_user));
    let (status, body) = send(
        &mut app,
        json_request(
            "/undocumented",
            r#"{"slug":"ada","retries":2,"tags":["x"]}"#,
        ),
    );
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    let report: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(
        report["violations"][0]["reason"],
        "expected a request carrying the document Router::into_axum installs, found a request carrying none"
    );
}
