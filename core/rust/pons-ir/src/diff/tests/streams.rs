use super::*;

#[test]
fn stream_events_are_classified_per_event() {
    let old: Document = serde_json::from_value(fixture()).unwrap();

    let mut added = fixture();
    added["operations"][1]["stream"]["events"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::json!({"event": "audit", "data": {"ref": "User"}}));
    let new: Document = serde_json::from_value(added).unwrap();
    let change = change_at(&old, &new, "operations.events.watch.stream.events.audit");
    assert_eq!(change.level, ChangeLevel::NonBreaking);
    assert_eq!(change.detail, "stream event added");

    let mut renamed = fixture();
    renamed["operations"][1]["stream"]["events"][0]["event"] = serde_json::json!("person");
    let new: Document = serde_json::from_value(renamed).unwrap();
    let removed = change_at(&old, &new, "operations.events.watch.stream.events.user");
    assert_eq!(removed.level, ChangeLevel::Breaking);
    assert_eq!(removed.detail, "stream event removed");
    assert_eq!(
        change_at(&old, &new, "operations.events.watch.stream.events.person").detail,
        "stream event added"
    );

    let mut retyped = fixture();
    retyped["operations"][1]["stream"]["events"][0]["data"] = serde_json::json!({"kind": "string"});
    let new: Document = serde_json::from_value(retyped).unwrap();
    let change = change_at(&old, &new, "operations.events.watch.stream.events.user");
    assert_eq!(change.level, ChangeLevel::Breaking);
    assert_eq!(change.detail, "stream event data type changed");
}

#[test]
fn non_event_stream_kinds_keep_their_own_paths() {
    let old: Document = serde_json::from_value(fixture()).unwrap();
    let reskinned = |kind: crate::StreamKind| {
        let mut document = old.clone();
        document.operations[1].stream.as_mut().unwrap().kind = kind;
        document.validate().unwrap();
        document
    };
    let kind_changed = reskinned(crate::StreamKind::Ndjson {
        item: TypeRef::Inline(InlineType::Bool),
    });
    let change = change_at(&old, &kind_changed, "operations.events.watch.stream");
    assert_eq!(change.level, ChangeLevel::Breaking);
    assert_eq!(change.detail, "stream kind changed");

    for (from, to, path, detail) in [
        (
            crate::StreamKind::Ndjson {
                item: TypeRef::Inline(InlineType::Bool),
            },
            crate::StreamKind::Ndjson {
                item: TypeRef::Inline(InlineType::Null),
            },
            "operations.events.watch.stream.item",
            "stream item type changed",
        ),
        (
            crate::StreamKind::Bytes {
                content_type: "application/octet-stream".into(),
            },
            crate::StreamKind::Bytes {
                content_type: "application/x-ndjson".into(),
            },
            "operations.events.watch.stream.content_type",
            "stream content type changed",
        ),
        (
            crate::StreamKind::Websocket {
                client: TypeRef::Inline(InlineType::Bool),
                server: TypeRef::Inline(InlineType::Bool),
            },
            crate::StreamKind::Websocket {
                client: TypeRef::Inline(InlineType::Null),
                server: TypeRef::Inline(InlineType::Bool),
            },
            "operations.events.watch.stream.client",
            "stream client message type changed",
        ),
    ] {
        let retyped = reskinned(to);
        let change = change_at(&reskinned(from), &retyped, path);
        assert_eq!(
            (change.level, change.detail.as_str()),
            (ChangeLevel::Breaking, detail)
        );
    }
}

#[test]
fn an_unchanged_stream_of_every_kind_has_no_diff() {
    let sse: Document = serde_json::from_value(fixture()).unwrap();
    let reskinned = |kind: crate::StreamKind| {
        let mut document = sse.clone();
        document.operations[1].stream.as_mut().unwrap().kind = kind;
        document.validate().unwrap();
        document
    };

    for document in [
        sse.clone(),
        reskinned(crate::StreamKind::Ndjson {
            item: TypeRef::Named(crate::NamedRef {
                id: "User".into(),
                variant: None,
            }),
        }),
        reskinned(crate::StreamKind::Bytes {
            content_type: "application/octet-stream".into(),
        }),
        reskinned(crate::StreamKind::Websocket {
            client: TypeRef::Inline(InlineType::Bool),
            server: TypeRef::Named(crate::NamedRef {
                id: "User".into(),
                variant: None,
            }),
        }),
    ] {
        assert_eq!(document.diff(&document), Diff::default());
    }
}
