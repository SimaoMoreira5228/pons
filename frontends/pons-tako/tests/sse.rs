#![cfg(feature = "sse")]

use std::collections::BTreeMap;

use futures_util::stream;
use pons_core::Schema;
use pons_ir::{ContentKind, Presence, ResponseStatus, StreamKind, TypeKind, TypeRef};
use pons_tako::{DescribeResponse, Info, OperationCx, OperationMeta, PonsSse, Router};
use serde::{Deserialize, Serialize};
use tako::responder::Responder;
use tako::types::Response;

#[derive(Serialize, Schema)]
struct Event {
    seq: u64,
}

#[derive(Serialize, Schema)]
struct EnvelopeEvent {
    data: String,
}

#[derive(Serialize, Schema)]
struct LaggedEvent {
    dropped: u64,
}

#[derive(Deserialize, Schema)]
#[serde(deny_unknown_fields)]
struct StreamQuery {
    pub channel_id: Option<String>,
}

struct MultipleEvents;

impl Responder for MultipleEvents {
    fn into_response(self) -> Response {
        "unused".into_response()
    }
}

impl DescribeResponse for MultipleEvents {
    fn describe(cx: &mut OperationCx) {
        cx.sse_event::<EnvelopeEvent>("envelope");
        cx.sse_event::<LaggedEvent>("lagged");
    }
}

async fn events() -> PonsSse<Event, impl futures_util::Stream<Item = Event> + Send> {
    PonsSse::new(stream::iter([Event { seq: 1 }]))
}

async fn multiple_events() -> MultipleEvents {
    MultipleEvents
}

fn info() -> Info {
    Info {
        title: "Events".into(),
        version: "1".into(),
        description: None,
        servers: Vec::new(),
        ext: BTreeMap::new(),
    }
}

#[test]
fn describes_the_typed_sse_stream() {
    let mut router = Router::new(info());
    router.get("/events", events);
    let document = router.into_document().unwrap();
    document.validate().unwrap();

    let operation = &document.operations[0];
    let Some(pons_ir::StreamDescription {
        kind: StreamKind::Sse { events },
    }) = &operation.stream
    else {
        panic!("the stream is not SSE")
    };
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].event, "message");
    let TypeRef::Named(event) = &events[0].data else {
        panic!("{:?} is not a named type", events[0].data)
    };
    assert_eq!(
        event.id,
        format!("{}#output", std::any::type_name::<Event>())
    );
    assert!(matches!(
        document.types[&event.id].definition,
        TypeKind::Struct { .. }
    ));

    assert_eq!(operation.responses.len(), 1);
    assert_eq!(operation.responses[0].status, ResponseStatus::Exact(200));
    assert_eq!(operation.responses[0].content, ContentKind::Stream);
}

#[test]
fn one_item_becomes_one_event() {
    use futures_util::FutureExt;
    use http_body_util::BodyExt;
    use tako_rs_core::responder::Responder;

    let response = PonsSse::new(stream::iter([Event { seq: 1 }])).into_response();
    assert_eq!(response.headers()["content-type"], "text/event-stream");
    assert_eq!(
        response.headers()["cache-control"],
        "no-cache, no-store, must-revalidate"
    );
    let body = response
        .into_body()
        .collect()
        .now_or_never()
        .unwrap()
        .unwrap()
        .to_bytes();
    assert_eq!(std::str::from_utf8(&body).unwrap(), "data: {\"seq\":1}\n\n");
}

#[test]
fn a_stream_can_describe_multiple_typed_events() {
    let query = StreamQuery { channel_id: None };
    assert!(query.channel_id.is_none());

    let mut router = Router::new(info());
    router.get_described_with(
        "/events",
        multiple_events,
        OperationMeta::new("events.subscribe"),
        |cx| {
            cx.query_type::<StreamQuery>();
            cx.request_header::<String>("last-event-id", Presence::Optional);
            MultipleEvents::describe(cx);
        },
    );
    let document = router.into_document().unwrap();
    document.validate().unwrap();

    let operation = &document.operations[0];
    let Some(pons_ir::StreamDescription {
        kind: StreamKind::Sse { events },
    }) = &operation.stream
    else {
        panic!("the stream is not SSE")
    };
    let names: Vec<_> = events.iter().map(|event| event.event.as_str()).collect();
    assert_eq!(names, ["envelope", "lagged"]);
    assert!(operation.query.is_some());
    assert_eq!(operation.headers[0].wire, "last-event-id");
    assert_eq!(operation.headers[0].presence, Presence::Optional);
    assert_eq!(operation.responses.len(), 1);
    assert_eq!(operation.responses[0].content, ContentKind::Stream);
}
