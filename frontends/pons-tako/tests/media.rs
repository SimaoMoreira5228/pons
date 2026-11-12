use std::collections::BTreeMap;

use pons_core::Schema;
use pons_ir::{ContentKind, TypeRef};
use pons_tako::{Info, MediaType, Router, WithMedia};
use serde::Serialize;
use tako::extractors::json::Json;

#[derive(Serialize, Schema)]
struct Report {
    id: u64,
}

struct VendorReport;

impl MediaType for VendorReport {
    const VALUE: &'static str = "application/vnd.acme.report.v1+json; version=1";
}

struct BrokenMediaType;

impl MediaType for BrokenMediaType {
    const VALUE: &'static str = "application/json\n";
}

struct GenericProblemJson;

impl MediaType for GenericProblemJson {
    const VALUE: &'static str = "application/problem+json";
}

async fn report() -> WithMedia<Json<Report>, VendorReport> {
    WithMedia::new(Json(Report { id: 9 }))
}

async fn broken() -> WithMedia<Json<Report>, BrokenMediaType> {
    WithMedia::new(Json(Report { id: 9 }))
}

async fn problem() -> WithMedia<Json<Report>, GenericProblemJson> {
    WithMedia::new(Json(Report { id: 9 }))
}

fn info() -> Info {
    Info {
        title: "Reports".into(),
        version: "1".into(),
        description: None,
        servers: Vec::new(),
        ext: BTreeMap::new(),
    }
}

fn errors(router: Router) -> String {
    router
        .into_document()
        .unwrap_err()
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn a_custom_media_response_publishes_the_exact_descriptor() {
    let mut router = Router::new(info());
    router.get("/report", report);
    let document = router.into_document().unwrap();
    document.validate().unwrap();

    let response = &document.operations[0].responses[0];
    assert_eq!(response.content, ContentKind::Json);
    assert!(matches!(response.body, Some(TypeRef::Named(_))));
    let media = response
        .media
        .as_ref()
        .expect("the media type is published");
    assert_eq!(media.media_type, "application");
    assert_eq!(media.subtype, "vnd.acme.report.v1+json");
    assert_eq!(
        media.parameters.get("version").map(String::as_str),
        Some("1")
    );
    assert_eq!(
        media.to_string(),
        "application/vnd.acme.report.v1+json; version=1"
    );
}

#[tokio::test]
async fn the_wrapper_serves_the_declared_content_type() {
    use http_body_util::BodyExt;
    use tako_rs_core::responder::Responder;

    let response = WithMedia::<_, VendorReport>::new(Json(Report { id: 9 })).into_response();
    assert_eq!(
        response.headers()["content-type"],
        "application/vnd.acme.report.v1+json; version=1"
    );
    let body = response.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(&body[..], b"{\"id\":9}");
}

#[test]
fn an_invalid_media_type_fails_the_description() {
    let mut router = Router::new(info());
    router.get("/broken", broken);
    let message = errors(router);
    assert!(message.contains("PONS-E102"), "{message}");
    assert!(message.contains("not a valid media type"), "{message}");
}

#[test]
fn problem_json_on_a_json_response_is_refused() {
    let mut router = Router::new(info());
    router.get("/problem", problem);
    let message = errors(router);
    assert!(message.contains("PONS-E102"), "{message}");
    assert!(message.contains("problem JSON"), "{message}");
}
