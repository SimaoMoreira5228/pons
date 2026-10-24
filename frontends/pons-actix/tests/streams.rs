use std::collections::BTreeMap;

use actix_web::http::StatusCode;
use actix_web::test;
use actix_web::web::Bytes;
use futures_util::stream;
use pons_actix::{Info, Ndjson, RawBytes, Router, Sse, SseEvent, routing::get};
use pons_core::Schema;
use pons_ir::StreamKind;
use serde::Serialize;

#[derive(Serialize, Schema)]
struct UserEvent {
    id: u64,
}

impl SseEvent for UserEvent {
    const NAME: &'static str = "user";
}

async fn events() -> Sse<UserEvent, impl futures_util::Stream<Item = UserEvent>> {
    Sse::new(stream::iter([UserEvent { id: 1 }]))
}

async fn lines() -> Ndjson<UserEvent, impl futures_util::Stream<Item = UserEvent>> {
    Ndjson::new(stream::iter([UserEvent { id: 1 }]))
}

async fn bytes() -> RawBytes<impl futures_util::Stream<Item = Bytes>> {
    RawBytes::new(stream::iter([Bytes::from_static(b"pdf")]))
}

fn info() -> Info {
    Info {
        title: "Streams".into(),
        version: "1".into(),
        description: None,
        servers: Vec::new(),
        ext: BTreeMap::new(),
    }
}

#[actix_web::test]
async fn sse_publishes_the_event_and_streams_it() {
    let router = Router::<()>::new(info()).route("/events", get(events));
    let document = router.document();
    assert!(matches!(
        document.operations[0].stream.as_ref().unwrap().kind,
        StreamKind::Sse { .. }
    ));

    let app = test::init_service(router.into_actix().unwrap()).await;
    let response =
        test::call_service(&app, test::TestRequest::get().uri("/events").to_request()).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers().get("content-type").unwrap(),
        "text/event-stream"
    );
    let body = test::read_body(response).await;
    assert_eq!(
        std::str::from_utf8(&body).unwrap(),
        "event: user\ndata: {\"id\":1}\n\n"
    );
}

#[actix_web::test]
async fn ndjson_publishes_the_item_and_streams_lines() {
    let router = Router::<()>::new(info()).route("/lines", get(lines));
    let document = router.document();
    assert!(matches!(
        document.operations[0].stream.as_ref().unwrap().kind,
        StreamKind::Ndjson { .. }
    ));

    let app = test::init_service(router.into_actix().unwrap()).await;
    let response =
        test::call_service(&app, test::TestRequest::get().uri("/lines").to_request()).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers().get("content-type").unwrap(),
        "application/x-ndjson"
    );
    let body = test::read_body(response).await;
    assert_eq!(std::str::from_utf8(&body).unwrap(), "{\"id\":1}\n");
}

#[actix_web::test]
async fn raw_bytes_publish_the_content_type_and_stream_the_chunks() {
    let router = Router::<()>::new(info()).route("/bytes", get(bytes));
    let document = router.document();
    assert_eq!(
        document.operations[0].stream.as_ref().unwrap().kind,
        StreamKind::Bytes {
            content_type: "application/octet-stream".into()
        }
    );

    let app = test::init_service(router.into_actix().unwrap()).await;
    let response =
        test::call_service(&app, test::TestRequest::get().uri("/bytes").to_request()).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers().get("content-type").unwrap(),
        "application/octet-stream"
    );
    let body = test::read_body(response).await;
    assert_eq!(&body[..], b"pdf");
}
