#![cfg(feature = "typed-header")]

use std::collections::BTreeMap;
use std::time::Duration;

use headers::{ETag, RetryAfter, UserAgent};
use pons_core::{ErrorSpec, Schema, TypeCx};
use pons_ir::{Direction, ErrorContent, ErrorResponse, ErrorStatus, Presence};
use pons_tako::{
    Info, MediaType, Router, WithErrorHeader, WithHeader, WithMedia, WithOptionalHeader, WithStatus,
};
use serde::Serialize;
use tako::extractors::json::Json as TakoJson;
use tako::responder::Responder;

#[derive(Serialize, Schema)]
struct User {
    id: u64,
}

#[derive(Serialize, Schema)]
struct RateLimited {
    retry_after_secs: u32,
}

impl ErrorSpec for RateLimited {
    fn errors(cx: &mut TypeCx) -> Vec<ErrorResponse> {
        vec![ErrorResponse {
            status: ErrorStatus::Exact(429),
            name: vec!["rate".into(), "limited".into()],
            content: ErrorContent::Json,
            media: None,
            body: Some(<Self as Schema>::schema(cx, Direction::Output)),
            headers: Vec::new(),
            retryable: Some(true),
            doc: None,
            ext: Default::default(),
        }]
    }
}

impl Responder for RateLimited {
    fn into_response(self) -> tako::types::Response {
        (tako::StatusCode::TOO_MANY_REQUESTS, TakoJson(self)).into_response()
    }
}

struct VendorReport;

impl MediaType for VendorReport {
    const VALUE: &'static str = "application/vnd.acme.user.v1+json";
}

fn etag() -> ETag {
    "\"abc\"".parse().expect("a quoted entity tag")
}

async fn both() -> WithHeader<UserAgent, WithOptionalHeader<ETag, TakoJson<User>>> {
    WithHeader::new(
        WithOptionalHeader::new(TakoJson(User { id: 7 }), Some(etag())),
        UserAgent::from_static("pons-test"),
    )
}

async fn omitted() -> WithHeader<UserAgent, WithOptionalHeader<ETag, TakoJson<User>>> {
    WithHeader::new(
        WithOptionalHeader::new(TakoJson(User { id: 8 }), None),
        UserAgent::from_static("pons-test"),
    )
}

async fn created() -> WithStatus<201, WithMedia<WithHeader<UserAgent, TakoJson<User>>, VendorReport>>
{
    WithStatus::new(WithMedia::new(WithHeader::new(
        TakoJson(User { id: 9 }),
        UserAgent::from_static("pons-test"),
    )))
}

async fn limited() -> Result<TakoJson<User>, WithErrorHeader<RetryAfter, RateLimited>> {
    Err(WithErrorHeader::new(
        RateLimited {
            retry_after_secs: 3,
        },
        RetryAfter::delay(Duration::from_secs(3)),
    ))
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

fn router() -> Router {
    let mut router = Router::new(info());
    router.get("/both", both);
    router.get("/omitted", omitted);
    router.get("/created", created);
    router.get("/limited", limited);
    router
}

fn document() -> pons_ir::Document {
    let document = router().into_document().unwrap();
    document.validate().unwrap();
    document
}

fn operation<'a>(document: &'a pons_ir::Document, path: &str) -> &'a pons_ir::Operation {
    document
        .operations
        .iter()
        .find(|operation| {
            operation.path.len() == 1
                && operation.path[0] == pons_ir::PathSegment::Literal { lit: path.into() }
        })
        .expect("the operation")
}

#[test]
fn a_success_response_publishes_required_and_optional_headers() {
    let document = document();
    let response = &operation(&document, "both").responses[0];
    assert_eq!(response.headers.len(), 2);

    let user_agent = response
        .headers
        .iter()
        .find(|header| header.wire == "user-agent")
        .expect("the required header");
    assert_eq!(user_agent.presence, Presence::Required);
    assert!(matches!(
        user_agent.ty,
        pons_ir::TypeRef::Inline(pons_ir::InlineType::Opaque { .. })
    ));

    let etag = response
        .headers
        .iter()
        .find(|header| header.wire == "etag")
        .expect("the optional header");
    assert_eq!(etag.presence, Presence::Optional);
}

#[test]
fn an_omitted_optional_header_stays_optional() {
    let document = document();
    let response = &operation(&document, "omitted").responses[0];
    let etag = response
        .headers
        .iter()
        .find(|header| header.wire == "etag")
        .expect("the optional header");
    assert_eq!(etag.presence, Presence::Optional);
}

#[test]
fn the_wrapper_composes_with_status_and_media() {
    let document = document();
    let response = &operation(&document, "created").responses[0];
    assert_eq!(response.status, pons_ir::ResponseStatus::Exact(201));
    assert_eq!(
        response.media.as_ref().map(ToString::to_string).as_deref(),
        Some(VendorReport::VALUE)
    );
    assert_eq!(response.headers.len(), 1);
    assert_eq!(response.headers[0].wire, "user-agent");
}

#[tokio::test]
async fn the_served_response_carries_the_values() {
    let response = both().await.into_response();
    assert_eq!(response.headers()["user-agent"], "pons-test");
    assert_eq!(response.headers()["etag"], "\"abc\"");
}

#[tokio::test]
async fn an_omitted_optional_header_is_not_served() {
    let response = omitted().await.into_response();
    assert_eq!(response.headers()["user-agent"], "pons-test");
    assert!(!response.headers().contains_key("etag"));
}

#[tokio::test]
async fn the_wrapper_replaces_an_existing_header_value() {
    let mut response = tako::types::Response::new(tako::body::TakoBody::empty());
    response
        .headers_mut()
        .insert("etag", http::HeaderValue::from_static("\"old\""));
    let response = WithHeader::new(response, etag()).into_response();
    assert_eq!(response.headers()["etag"], "\"abc\"");
    assert_eq!(response.headers().get_all("etag").iter().count(), 1);
}

#[tokio::test]
async fn status_and_media_keep_their_behavior_around_a_header() {
    let response = created().await.into_response();
    assert_eq!(response.status(), tako::StatusCode::CREATED);
    assert_eq!(response.headers()["content-type"], VendorReport::VALUE);
    assert_eq!(response.headers()["user-agent"], "pons-test");
}

#[test]
fn an_error_response_publishes_the_wrapped_header() {
    let document = document();
    let error = &operation(&document, "limited").errors[0];
    let header = error
        .headers
        .iter()
        .find(|header| header.wire == "retry-after")
        .expect("the required error header");
    assert_eq!(header.presence, Presence::Required);
}

#[tokio::test]
async fn the_served_error_carries_the_wrapped_header() {
    let Err(error) = limited().await else {
        panic!("the handler answers an error");
    };
    let response = error.into_response();
    assert_eq!(response.status(), tako::StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(response.headers()["retry-after"], "3");
}
