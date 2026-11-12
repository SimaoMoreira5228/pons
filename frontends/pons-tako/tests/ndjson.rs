#![cfg(feature = "ndjson")]

use std::collections::BTreeMap;

use futures_util::stream;
use pons_core::Schema;
use pons_ir::{ContentKind, ResponseStatus, StreamKind, TypeKind, TypeRef};
use pons_tako::{Info, PonsNdjson, Router};
use serde::Serialize;

#[derive(Serialize, Schema)]
struct Event {
    seq: u64,
}

async fn events() -> PonsNdjson<Event, impl futures_util::Stream<Item = Event> + Send> {
    PonsNdjson::new(stream::iter([Event { seq: 1 }]))
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
fn describes_the_typed_ndjson_stream() {
    let mut router = Router::new(info());
    router.get("/events", events);
    let document = router.into_document().unwrap();
    document.validate().unwrap();

    let operation = &document.operations[0];
    let Some(pons_ir::StreamDescription {
        kind: StreamKind::Ndjson { item },
    }) = &operation.stream
    else {
        panic!("the stream is not NDJSON")
    };
    let TypeRef::Named(event) = item else {
        panic!("{item:?} is not a named type")
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
    assert!(operation.responses[0].body.is_none());
}

#[test]
fn one_item_becomes_one_line() {
    use futures_util::FutureExt;
    use http_body_util::BodyExt;
    use tako_rs_core::responder::Responder;

    let response =
        PonsNdjson::new(stream::iter([Event { seq: 1 }, Event { seq: 2 }])).into_response();
    assert_eq!(response.headers()["content-type"], "application/x-ndjson");
    let body = response
        .into_body()
        .collect()
        .now_or_never()
        .unwrap()
        .unwrap()
        .to_bytes();
    assert_eq!(
        std::str::from_utf8(&body).unwrap(),
        "{\"seq\":1}\n{\"seq\":2}\n"
    );
}
