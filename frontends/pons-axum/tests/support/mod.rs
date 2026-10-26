#![allow(dead_code)]

pub mod router;
pub mod validation;

use axum::body::Body;
use pons_axum::http::{Request, StatusCode};
use pons_axum::{HeaderSpec, Info, Router};
use pons_ir::{ErrorResponse, Operation, PathSegment, TypeDef};
use serde_json::Value;
use std::collections::BTreeMap;
use std::future::Future;
use tower_service::Service;

pub fn block_on<F: Future>(future: F) -> F::Output {
    struct ThreadWaker(std::thread::Thread);

    impl std::task::Wake for ThreadWaker {
        fn wake(self: std::sync::Arc<Self>) {
            self.0.unpark();
        }
    }

    let waker = std::task::Waker::from(std::sync::Arc::new(ThreadWaker(std::thread::current())));
    let mut context = std::task::Context::from_waker(&waker);
    let mut future = std::pin::pin!(future);
    loop {
        match future.as_mut().poll(&mut context) {
            std::task::Poll::Ready(value) => return value,
            std::task::Poll::Pending => std::thread::park(),
        }
    }
}

pub fn built(router: pons_axum::Router<()>) -> axum::Router<()> {
    router
        .into_axum()
        .expect("the fixture router installs a document that validates")
}

pub fn info() -> Info {
    Info {
        title: "Users".into(),
        version: "1".into(),
        description: None,
        servers: Vec::new(),
        ext: BTreeMap::new(),
    }
}

pub fn operation<'a>(document: &'a pons_ir::Document, literal: &str) -> &'a Operation {
    document
        .operations
        .iter()
        .find(|operation| {
            operation
                .path
                .iter()
                .any(|segment| matches!(segment, PathSegment::Literal { lit } if lit == literal))
        })
        .unwrap()
}

pub fn literal(lit: &str) -> PathSegment {
    PathSegment::Literal { lit: lit.into() }
}

pub fn get_status(app: &mut axum::Router<()>, path: &str, authorized: bool) -> StatusCode {
    let header = authorized.then_some(("authorization", "Bearer token"));
    request_status(app, path, header)
}

pub fn request_status(
    app: &mut axum::Router<()>,
    path: &str,
    header: Option<(&str, &str)>,
) -> StatusCode {
    let mut builder = pons_axum::http::Request::builder().uri(path);
    if let Some((name, value)) = header {
        builder = builder.header(name, value);
    }
    let request: pons_axum::extract::Request = builder.body(axum::body::Body::empty()).unwrap();
    block_on(app.call(request)).unwrap().status()
}

pub fn request(path: &str, content_type: &str, body: &str) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(path)
        .header("content-type", content_type)
        .body(Body::from(body.to_owned()))
        .unwrap()
}

pub fn json_request(path: &str, body: &str) -> Request<Body> {
    request(path, "application/json", body)
}

pub fn get_request(path: &str) -> Request<Body> {
    Request::builder().uri(path).body(Body::empty()).unwrap()
}

pub fn header_request(path: &str, name: &str, value: &str) -> Request<Body> {
    Request::builder()
        .uri(path)
        .header(name, value)
        .body(Body::empty())
        .unwrap()
}

pub fn send(app: &mut axum::Router<()>, request: Request<Body>) -> (StatusCode, String) {
    let response = block_on(app.call(request)).unwrap();
    let status = response.status();
    let bytes = block_on(axum::body::to_bytes(response.into_body(), usize::MAX)).unwrap();
    (status, String::from_utf8(bytes.to_vec()).unwrap())
}

pub fn accepted(app: &mut axum::Router<()>, request: Request<Body>) {
    let (status, body) = send(app, request);
    assert_eq!(status, StatusCode::OK, "{body}");
}

pub fn rejected(app: &mut axum::Router<()>, request: Request<Body>, expected: &[&str]) {
    let (status, body) = send(app, request);
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    let report: Value = serde_json::from_str(&body).unwrap();
    let found: Vec<String> = report["violations"]
        .as_array()
        .unwrap_or_else(|| panic!("{body} is not a violation report"))
        .iter()
        .map(|violation| {
            format!(
                "{}: {}",
                violation["path"].as_str().unwrap(),
                violation["reason"].as_str().unwrap()
            )
        })
        .collect();
    assert_eq!(found, expected, "{body}");
}

pub fn declared_rejection(document: &pons_ir::Document, literal: &str) -> Option<ErrorResponse> {
    operation(document, literal)
        .errors
        .iter()
        .find(|error| error.name == ["request", "rejected"])
        .cloned()
}

pub fn body_type<'a>(document: &'a pons_ir::Document, id: &str) -> &'a TypeDef {
    document
        .types
        .get(id)
        .unwrap_or_else(|| panic!("{id} is unpublished"))
}

pub fn refused(router: Router<()>) -> String {
    let errors = router.into_document().unwrap_err();
    errors
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn refused_build(router: Router<()>) -> String {
    let errors = router
        .into_axum()
        .expect_err("a router that installs a document which does not validate must not be built");
    errors
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("\n")
}

pub struct RequestId;

impl HeaderSpec for RequestId {
    type Value = String;
    const NAME: &'static str = "X-Request-ID";
}
