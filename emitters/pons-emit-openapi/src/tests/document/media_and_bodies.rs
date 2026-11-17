use pons_ir::{ByteEncoding, ContentKind, InlineType, NamedRef, StreamKind, TypeKind, TypeRef};
use serde_json::{Value, json};

use super::super::{fixture, named_type, string};

#[test]
fn rejects_mismatched_byte_stream_media_type_override() {
    let mut document = fixture();
    {
        let operation = document
            .operations
            .iter_mut()
            .find(|operation| operation.stream.is_some())
            .unwrap();
        operation.stream.as_mut().unwrap().kind = StreamKind::Bytes {
            content_type: "application/pdf".into(),
        };
        operation.responses[0].media =
            Some(pons_ir::MediaDescriptor::from_declared("application/json"));
    }
    assert!(
        crate::emit(&document)
            .unwrap_err()
            .0
            .contains("does not match stream kind media type")
    );

    document
        .operations
        .iter_mut()
        .find(|operation| operation.stream.is_some())
        .unwrap()
        .responses[0]
        .media = Some(pons_ir::MediaDescriptor::from_declared("application/pdf"));
    assert!(crate::emit(&document).is_ok());

    {
        let operation = document
            .operations
            .iter_mut()
            .find(|operation| operation.stream.is_some())
            .unwrap();
        operation.stream.as_mut().unwrap().kind = StreamKind::Bytes {
            content_type: "Application/PDF ; Version=1".into(),
        };
        operation.responses[0].media = Some(pons_ir::MediaDescriptor::from_declared(
            "application/pdf; version=1",
        ));
    }
    assert!(crate::emit(&document).is_ok());
}

#[test]
fn rejects_sse_and_ndjson_media_type_overrides() {
    let mut document = fixture();
    for (kind, expected) in [
        (StreamKind::Sse { events: vec![] }, "text/event-stream"),
        (
            StreamKind::Ndjson {
                item: TypeRef::Inline(InlineType::Bool),
            },
            "application/x-ndjson",
        ),
    ] {
        for (media_type, accepted) in [
            (expected.to_owned(), true),
            (format!("{expected}; charset=utf-8"), true),
            (format!("{expected} ; CharSet=UTF-8"), true),
            ("application/json".into(), false),
            ("application/octet-stream".into(), false),
        ] {
            let operation = document
                .operations
                .iter_mut()
                .find(|operation| operation.stream.is_some())
                .unwrap();
            operation.stream.as_mut().unwrap().kind = kind.clone();
            operation.responses[0].media =
                Some(pons_ir::MediaDescriptor::from_declared(media_type.clone()));
            assert_eq!(
                crate::emit(&document).is_ok(),
                accepted,
                "{media_type:?} on an {expected} stream"
            );
        }

        let operation = document
            .operations
            .iter_mut()
            .find(|operation| operation.stream.is_some())
            .unwrap();
        operation.stream.as_mut().unwrap().kind = kind;
        operation.responses[0].media = None;
        let emitted: Value = serde_json::from_str(&crate::emit(&document).unwrap()).unwrap();
        assert!(emitted.to_string().contains(expected));
    }
}

#[test]
fn emits_optional_request_bodies() {
    let mut document = fixture();
    let login = document
        .operations
        .iter_mut()
        .find(|operation| operation.id[0][0] == "auth")
        .unwrap();
    login.body.as_mut().unwrap().required = false;
    let output: Value = serde_json::from_str(&crate::emit(&document).unwrap()).unwrap();
    assert_eq!(
        output["paths"]["/auth/login"]["post"]["requestBody"]["required"],
        false
    );
}

#[test]
fn emits_exact_custom_media_types() {
    let mut document = fixture();
    let login = document
        .operations
        .iter_mut()
        .find(|operation| operation.id[0][0] == "auth")
        .unwrap();
    login.body.as_mut().unwrap().media = Some(pons_ir::MediaDescriptor::from_declared(
        "application/vnd.acme+json",
    ));
    login.responses[0].media = Some(pons_ir::MediaDescriptor::from_declared(
        "application/vnd.acme.session+json",
    ));
    login.errors[0].media = Some(pons_ir::MediaDescriptor::from_declared(
        "application/vnd.acme.error+json",
    ));
    let output: Value = serde_json::from_str(&crate::emit(&document).unwrap()).unwrap();
    let login = &output["paths"]["/auth/login"]["post"];
    assert!(login["requestBody"]["content"]["application/vnd.acme+json"].is_object());
    assert!(login["responses"]["200"]["content"]["application/vnd.acme.session+json"].is_object());
    assert!(login["responses"]["4XX"]["content"]["application/vnd.acme.error+json"].is_object());
}

#[test]
fn distinguishes_raw_byte_bodies_from_encoded_values() {
    let mut document = fixture();
    let user = document.types.get_mut("User").unwrap();
    let TypeKind::Struct { fields, .. } = &mut user.definition else {
        unreachable!()
    };
    fields.push(pons_ir::Field {
        wire: "avatar".into(),
        ident: vec!["avatar".into()],
        ty: TypeRef::Inline(InlineType::Bytes {
            encoding: ByteEncoding::Base64url,
        }),
        presence: pons_ir::Presence::Optional,
        default: Default::default(),
        access: None,
        doc: None,
        diagnostics: Vec::new(),
        ext: Default::default(),
    });
    let login = document
        .operations
        .iter_mut()
        .find(|operation| operation.id[0][0] == "auth")
        .unwrap();
    login.body.as_mut().unwrap().content = ContentKind::Bytes;
    login.body.as_mut().unwrap().ty = TypeRef::Inline(InlineType::Bytes {
        encoding: ByteEncoding::Base64,
    });
    login.responses[0].content = ContentKind::Bytes;
    login.responses[0].body = Some(TypeRef::Inline(InlineType::Bytes {
        encoding: ByteEncoding::Hex,
    }));
    let events = document
        .operations
        .iter_mut()
        .find(|operation| operation.stream.is_some())
        .unwrap();
    events.stream.as_mut().unwrap().kind = StreamKind::Bytes {
        content_type: "application/pdf".into(),
    };

    let output: Value = serde_json::from_str(&crate::emit(&document).unwrap()).unwrap();
    let login = &output["paths"]["/auth/login"]["post"];
    assert_eq!(
        login["requestBody"]["content"]["application/octet-stream"]["schema"],
        json!({})
    );
    assert_eq!(login["requestBody"]["x-pons"]["ty"]["kind"], "bytes");
    assert_eq!(
        login["responses"]["200"]["content"]["application/octet-stream"]["schema"],
        json!({})
    );
    assert_eq!(login["responses"]["200"]["x-pons"]["body"]["kind"], "bytes");
    assert_eq!(
        output["paths"]["/events/{cursor}"]["get"]["responses"]["200"]["content"]["application/pdf"]
            ["schema"],
        json!({"x-pons-stream":{"kind":"bytes","content_type":"application/pdf"}})
    );
    assert_eq!(
        output["components"]["schemas"]["User"]["properties"]["avatar"]["contentEncoding"],
        "base64url"
    );

    let mut wrapped = document.clone();
    let mut byte_alias = named_type("rawBytes");
    byte_alias.definition = TypeKind::Newtype {
        inner: TypeRef::Inline(InlineType::Bytes {
            encoding: ByteEncoding::Base64,
        }),
    };
    wrapped.types.insert("RawBytes".into(), byte_alias);
    wrapped
        .operations
        .iter_mut()
        .find(|operation| operation.id[0][0] == "auth")
        .unwrap()
        .body
        .as_mut()
        .unwrap()
        .ty = TypeRef::Named(NamedRef {
        id: "RawBytes".into(),
        variant: None,
    });
    crate::emit(&wrapped).unwrap();

    let mut invalid = document.clone();
    invalid
        .operations
        .iter_mut()
        .find(|operation| operation.id[0][0] == "auth")
        .unwrap()
        .body
        .as_mut()
        .unwrap()
        .ty = string();
    assert!(
        crate::emit(&invalid)
            .unwrap_err()
            .0
            .contains("raw byte bodies must use a byte type")
    );

    let mut invalid = document;
    invalid
        .operations
        .iter_mut()
        .find(|operation| operation.stream.is_some())
        .unwrap()
        .stream
        .as_mut()
        .unwrap()
        .kind = StreamKind::Bytes {
        content_type: String::new(),
    };
    assert!(
        crate::emit(&invalid)
            .unwrap_err()
            .0
            .contains("invalid byte stream content type")
    );
}
