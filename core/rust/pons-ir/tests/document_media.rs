mod support;

use serde_json::{Value, json};
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
