mod support;

use pons_ir::{
    ByteEncoding, ContentKind, Direction, Document, InlineType, NamedRef, OpaqueWire, StreamKind,
    TypeDef, TypeKind, TypeRef,
};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use support::{contract, field, fixture, refuses, request_body, string, struct_type};

#[test]
fn a_declared_media_type_must_name_the_body_the_client_writes() {
    let body = |content: &str, media_type: &str, ty: Value| {
        let mut document = request_body(json!({
            "Envelope": struct_type("envelope", vec![field("email", string(), "required")], vec![], json!("reject")),
            "Blob": {"kind": "newtype", "name": ["blob"], "direction": "input",
                "inner": {"kind": "bytes", "encoding": "base64"}}
        }));
        let body = &mut document.operations[0].body.as_mut().unwrap();
        body.content = serde_json::from_value(json!(content)).unwrap();
        body.media = Some(pons_ir::MediaDescriptor::from_declared(media_type));
        body.ty = serde_json::from_value(ty).unwrap();
        document
    };
    let envelope = json!({"ref": "Envelope"});
    let blob = json!({"ref": "Blob"});

    for (content, media_type) in [
        ("form", "application/json"),
        ("multipart", "application/json"),
        ("text", "application/json"),
        ("text", "application/x-www-form-urlencoded"),
        ("bytes", "application/json"),
        ("bytes", "application/vnd.acme+json"),
        ("bytes", "multipart/form-data"),
    ] {
        refuses(
            &body(
                content,
                media_type,
                if content == "bytes" {
                    blob.clone()
                } else {
                    envelope.clone()
                },
            ),
            &format!("has media type {media_type}, which does not name the"),
        );
    }
    for (content, media_type, ty) in [
        ("json", "application/json", envelope.clone()),
        (
            "json",
            "application/vnd.acme+json; charset=utf-8",
            envelope.clone(),
        ),
        ("problem_json", "application/problem+json", envelope.clone()),
        (
            "form",
            "application/x-www-form-urlencoded",
            envelope.clone(),
        ),
        ("multipart", "multipart/form-data", envelope.clone()),
        ("text", "text/plain; charset=utf-8", envelope.clone()),
        ("text", "text/csv", envelope.clone()),
        ("bytes", "application/octet-stream", blob.clone()),
        ("bytes", "application/pdf; version=1", blob.clone()),
    ] {
        body(content, media_type, ty).validate().unwrap();
    }

    let mut response = fixture();
    response.operations[0].responses[0].content = pons_ir::ContentKind::Text;
    response.operations[0].responses[0].media =
        Some(pons_ir::MediaDescriptor::from_declared("application/json"));
    refuses(
        &response,
        "response 200 body has media type application/json, which does not name the text body the client writes",
    );
    let mut error = fixture();
    error.operations[0].errors[0].content = pons_ir::ErrorContent::Text;
    error.operations[0].errors[0].media =
        Some(pons_ir::MediaDescriptor::from_declared("application/json"));
    refuses(
        &error,
        "error body has media type application/json, which does not name the text body the client writes",
    );
}

#[test]
fn a_stream_request_body_is_refused() {
    let mut document = fixture();
    document.operations[0].body.as_mut().unwrap().content = pons_ir::ContentKind::Stream;
    refuses(
        &document,
        "a request body cannot be stream content, because no built-in target reads one",
    );
}

#[test]
fn a_websocket_message_needs_the_direction_the_operation_uses() {
    let socket = |client: &str, server: &str| {
        let mut document = contract(json!({
            "Sent": {"kind": "struct", "name": ["sent"], "direction": "input", "openness": "reject",
                "fields": [field("a", string(), "required")]},
            "Received": {"kind": "struct", "name": ["received"], "direction": "output", "openness": "reject",
                "fields": [field("b", string(), "required")]}
        }));
        document.operations.push(
            serde_json::from_value(json!({
                "id": [["socket"]],
                "method": "GET",
                "path": [{"lit": "socket"}],
                "responses": [{"status": 200, "content": "stream"}],
                "stream": {"kind": "websocket", "client": {"ref": client}, "server": {"ref": server}},
                "auth": {"alternatives": [], "optional": false},
                "semantics": {"safe": true, "idempotent": true}
            }))
            .unwrap(),
        );
        document
    };

    socket("Sent", "Received").validate().unwrap();
    refuses(
        &socket("Received", "Received"),
        "the websocket client message needs a type of the input or both direction, but Received is declared output",
    );
    refuses(
        &socket("Sent", "Sent"),
        "the websocket server message needs a type of the output or both direction, but Sent is declared input",
    );
}

#[test]
fn media_types_validate_and_empty_payloads_reject_them() {
    let mut document: Document =
        serde_json::from_str(include_str!("../../../../spec/fixtures/login.json")).unwrap();
    document.operations[0]
        .body
        .as_mut()
        .unwrap()
        .media
        .clone_from(&Some(pons_ir::MediaDescriptor::from_declared(
            "application/vnd.acme+json; charset=utf-8",
        )));
    document.validate().unwrap();
    document.operations[0].body.as_mut().unwrap().media =
        Some(pons_ir::MediaDescriptor::from_declared("not-a-media-type"));
    document.operations[0].responses[0].content = ContentKind::Empty;
    document.operations[0].responses[0].body = None;
    document.operations[0].responses[0].media =
        Some(pons_ir::MediaDescriptor::from_declared("application/json"));
    let errors = document.validate().unwrap_err();
    assert!(
        errors
            .iter()
            .any(|error| error.to_string().contains("invalid media type"))
    );
    assert!(
        errors
            .iter()
            .any(|error| error.to_string().contains("empty response"))
    );
}

#[test]
fn byte_stream_media_types_are_consistent_and_validated() {
    let mut document: Document =
        serde_json::from_str(include_str!("../../../../spec/fixtures/login.json")).unwrap();
    let operation_index = document
        .operations
        .iter()
        .position(|operation| operation.stream.is_some())
        .unwrap();
    document.operations[operation_index]
        .stream
        .as_mut()
        .unwrap()
        .kind = StreamKind::Bytes {
        content_type: "Application/PDF ; Version=1".into(),
    };
    document.validate().unwrap();
    document.operations[operation_index].responses[0].media = Some(
        pons_ir::MediaDescriptor::from_declared("application/pdf; version=2"),
    );
    assert!(document.validate().unwrap_err().iter().any(|error| {
        error
            .to_string()
            .contains("does not match stream kind media type")
    }));
    document.operations[operation_index].responses[0].media = Some(
        pons_ir::MediaDescriptor::from_declared("application/pdf; version=1; extra=value"),
    );
    document.validate().unwrap();
    document.operations[operation_index]
        .stream
        .as_mut()
        .unwrap()
        .kind = StreamKind::Bytes {
        content_type: "not-a-media-type".into(),
    };
    assert!(document.validate().unwrap_err().iter().any(|error| {
        error
            .to_string()
            .contains("invalid byte stream content type")
    }));
}

#[test]
fn sse_and_ndjson_media_types_are_checked_against_their_kind() {
    let mut document: Document =
        serde_json::from_str(include_str!("../../../../spec/fixtures/login.json")).unwrap();
    let operation_index = document
        .operations
        .iter()
        .position(|operation| operation.stream.is_some())
        .unwrap();
    for (kind, expected) in [
        (StreamKind::Sse { events: vec![] }, "text/event-stream"),
        (
            StreamKind::Ndjson {
                item: TypeRef::Inline(InlineType::Bool),
            },
            "application/x-ndjson",
        ),
    ] {
        document.operations[operation_index]
            .stream
            .as_mut()
            .unwrap()
            .kind = kind;

        for accepted in [
            expected.to_owned(),
            format!("{expected}; charset=utf-8"),
            format!("{expected} ; CharSet=UTF-8"),
        ] {
            document.operations[operation_index].responses[0].media =
                Some(pons_ir::MediaDescriptor::from_declared(accepted));
            document.validate().unwrap();
        }
        for rejected in ["application/json", "application/octet-stream", "text/plain"] {
            document.operations[operation_index].responses[0].media =
                Some(pons_ir::MediaDescriptor::from_declared(rejected));
            assert!(
                document.validate().unwrap_err().iter().any(|error| error
                    .to_string()
                    .contains("does not match stream kind media type")),
                "{rejected:?} must not serve an {expected} stream"
            );
        }
    }
}

#[test]
fn byte_bodies_must_resolve_to_a_byte_value() {
    const REQUEST: &str =
        "raw byte bodies must use a byte type, byte newtype, nullable bytes, or a bytes opaque";
    const RESPONSE: &str = "raw byte response bodies must use a byte type, byte newtype, nullable bytes, or a bytes opaque";

    fn bytes() -> TypeRef {
        TypeRef::Inline(InlineType::Bytes {
            encoding: ByteEncoding::Base64,
        })
    }
    fn nullable(inner: TypeRef) -> TypeRef {
        TypeRef::Inline(InlineType::Nullable {
            inner: Box::new(inner),
        })
    }
    fn opaque(wire: OpaqueWire) -> TypeRef {
        TypeRef::Inline(InlineType::Opaque {
            codec: "Raw".into(),
            wire,
            ext: Default::default(),
        })
    }
    fn string() -> TypeRef {
        TypeRef::Inline(InlineType::String {
            format: None,
            min_length: None,
            max_length: None,
            pattern: None,
        })
    }
    fn named(id: &str) -> TypeRef {
        TypeRef::Named(NamedRef {
            id: id.into(),
            variant: None,
        })
    }
    fn newtype(document: &mut Document, id: &str, inner: TypeRef) -> TypeRef {
        document.types.insert(
            id.into(),
            TypeDef {
                name: vec![id.to_ascii_lowercase()],
                direction: Direction::Both,
                definition: TypeKind::Newtype { inner },
                origin: None,
                doc: None,
                diagnostics: Vec::new(),
                ext: BTreeMap::new(),
            },
        );
        named(id)
    }
    fn rejects(document: &Document, message: &str) {
        let errors = document
            .validate()
            .unwrap_err()
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>();
        assert!(
            errors.iter().any(|error| error.contains(message)),
            "{message:?} missing from {errors:?}"
        );
    }

    let mut document: Document =
        serde_json::from_str(include_str!("../../../../spec/fixtures/login.json")).unwrap();
    let login = &mut document.operations[0];
    login.body.as_mut().unwrap().content = ContentKind::Bytes;
    login.body.as_mut().unwrap().ty = bytes();
    login.responses[0].content = ContentKind::Bytes;
    login.responses[0].body = Some(bytes());
    document.validate().unwrap();

    let mut document = document.clone();
    document.operations[0].body.as_mut().unwrap().ty = string();
    rejects(&document, REQUEST);
    document.operations[0].body.as_mut().unwrap().ty = named("User");
    rejects(&document, REQUEST);
    document.operations[0].body.as_mut().unwrap().ty = newtype(&mut document, "Text", string());
    rejects(&document, REQUEST);

    let mut document = document.clone();
    document.operations[0].responses[0].body = Some(named("User"));
    rejects(&document, RESPONSE);

    let raw = newtype(&mut document, "RawBytes", bytes());
    let nested = newtype(&mut document, "NestedBytes", raw);
    document.operations[0].body.as_mut().unwrap().ty = nested.clone();
    document.operations[0].responses[0].body = Some(nested);
    document.validate().unwrap();

    for accepted in [
        nullable(bytes()),
        newtype(&mut document, "MaybeBytes", nullable(bytes())),
        opaque(OpaqueWire::Bytes),
        nullable(opaque(OpaqueWire::Bytes)),
        newtype(&mut document, "OpaqueBytes", opaque(OpaqueWire::Bytes)),
    ] {
        let mut document = document.clone();
        document.operations[0].body.as_mut().unwrap().ty = accepted.clone();
        document.operations[0].responses[0].body = Some(accepted);
        document.validate().unwrap();
    }

    for rejected in [
        nullable(string()),
        nullable(named("User")),
        opaque(OpaqueWire::Text),
        opaque(OpaqueWire::Json),
        opaque(OpaqueWire::Unknown),
        newtype(&mut document, "OpaqueText", opaque(OpaqueWire::Text)),
    ] {
        let mut document = document.clone();
        document.operations[0].body.as_mut().unwrap().ty = rejected.clone();
        rejects(&document, REQUEST);
        document.operations[0].responses[0].body = Some(rejected);
        rejects(&document, RESPONSE);
    }

    document.operations[0].body.as_mut().unwrap().ty =
        newtype(&mut document, "Loop", named("Loop"));
    rejects(&document, REQUEST);
}
