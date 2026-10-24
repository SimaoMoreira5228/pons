use pons_ir::{
    ContentKind, Document, IR_VERSION, InlineType, ObjectOpenness, PathSegment, StreamKind,
    TypeKind, TypeRef,
};
use serde_json::json;

#[test]
fn rejects_structs_flattened_with_maps() {
    let mut raw: serde_json::Value =
        serde_json::from_str(include_str!("../../../../spec/fixtures/login.json")).unwrap();
    raw["types"]["LoginRequest"]["flattened"] = json!([{
        "kind": "map",
        "key": {"kind": "string"},
        "value": {"kind": "string"}
    }]);
    let document: Document = serde_json::from_value(raw).unwrap();
    assert!(crate::emit(&document).unwrap_err().0.contains(
        "is not a struct, because a flattened member contributes its fields to one object"
    ));
}

#[test]
fn rejects_query_types_without_a_wire_name() {
    let scalar = || {
        TypeRef::Inline(InlineType::String {
            format: None,
            min_length: None,
            max_length: None,
            pattern: None,
        })
    };
    let mut document: Document =
        serde_json::from_str(include_str!("../../../../spec/fixtures/login.json")).unwrap();
    document.operations[1].query = Some(scalar());
    let error = crate::emit(&document).unwrap_err();
    assert!(
        error.0.contains("[[\"events\"], [\"watch\"]]"),
        "{}",
        error.0
    );
    assert!(error.0.contains("structs"), "{}", error.0);

    let mut raw: serde_json::Value =
        serde_json::from_str(include_str!("../../../../spec/fixtures/login.json")).unwrap();
    raw["types"]["QueryAlias"] = json!({
        "kind": "newtype",
        "name": ["query", "alias"],
        "direction": "input",
        "inner": {"kind": "string"}
    });
    let document: Document = serde_json::from_value(raw.clone()).unwrap();
    let mut document = document;
    document.operations[1].query = Some(TypeRef::Named(pons_ir::NamedRef {
        id: "QueryAlias".into(),
        variant: None,
    }));
    assert!(
        crate::emit(&document)
            .unwrap_err()
            .0
            .contains("[[\"events\"], [\"watch\"]]")
    );

    raw["types"]["QueryAlias"] = json!({
        "kind": "newtype",
        "name": ["query", "alias"],
        "direction": "input",
        "inner": {"ref": "EventQuery"}
    });
    let mut document: Document = serde_json::from_value(raw).unwrap();
    document.operations[1].query = Some(TypeRef::Named(pons_ir::NamedRef {
        id: "QueryAlias".into(),
        variant: None,
    }));
    assert!(crate::emit(&document).is_ok());
}

#[test]
fn rejects_nullable_flatten_before_emitting_a_contract() {
    let mut raw: serde_json::Value =
        serde_json::from_str(include_str!("../../../../spec/fixtures/login.json")).unwrap();
    raw["types"]["LoginRequest"]["flattened"] = json!([{
        "kind": "nullable",
        "inner": {"ref": "User"}
    }]);
    let document: Document = serde_json::from_value(raw).unwrap();
    for mode in [crate::EmitMode::Strict, crate::EmitMode::Lossy] {
        assert!(
            crate::emit_with_mode(&document, mode)
                .unwrap_err()
                .0
                .contains("nullable flattened")
        );
    }
}

#[test]
fn rejects_streaming_request_bodies() {
    let mut document: Document =
        serde_json::from_str(include_str!("../../../../spec/fixtures/login.json")).unwrap();
    document.operations[0].body.as_mut().unwrap().content = ContentKind::Stream;
    for mode in [crate::EmitMode::Strict, crate::EmitMode::Lossy] {
        let error = crate::emit_with_mode(&document, mode).unwrap_err();
        assert!(
            error.0.contains(
                "a request body cannot be stream content, because no built-in target reads one"
            ),
            "{}",
            error.0
        );
    }
}

#[test]
fn rejects_required_nullable_request_headers_and_path_parameters() {
    let mut document: Document =
        serde_json::from_str(include_str!("../../../../spec/fixtures/login.json")).unwrap();
    let watch = &mut document.operations[1];
    watch.headers[0].presence = pons_ir::Presence::Required;
    watch.headers[0].ty = TypeRef::Inline(InlineType::Nullable {
        inner: Box::new(TypeRef::Inline(InlineType::String {
            format: None,
            min_length: None,
            max_length: None,
            pattern: None,
        })),
    });
    let error = crate::emit(&document).unwrap_err();
    assert!(
        error
            .0
            .contains("required request header X-Request-ID cannot be nullable"),
        "{}",
        error.0
    );

    let mut document: Document =
        serde_json::from_str(include_str!("../../../../spec/fixtures/login.json")).unwrap();
    document.operations[1].path[1] = PathSegment::Param {
        param: "cursor".into(),
        ty: TypeRef::Inline(InlineType::Nullable {
            inner: Box::new(TypeRef::Inline(InlineType::String {
                format: None,
                min_length: None,
                max_length: None,
                pattern: None,
            })),
        }),
    };
    let error = crate::emit(&document).unwrap_err();
    assert!(
        error
            .0
            .contains("required path parameter cursor cannot be nullable"),
        "{}",
        error.0
    );
}

#[test]
fn refuses_a_use_site_whose_type_cannot_cross_the_boundary() {
    let mut raw: serde_json::Value =
        serde_json::from_str(include_str!("../../../../spec/fixtures/login.json")).unwrap();
    raw["operations"][0]["body"] =
        json!({"ty": {"ref": "User"}, "content": "json", "required": true});
    let document: Document = serde_json::from_value(raw).unwrap();
    let error = crate::emit(&document).unwrap_err();
    assert!(
            error.0.contains(
                "operations[0].body.ty: the request body needs a type of the input or both direction, but User is declared output"
            ),
            "{}",
            error.0
        );
}

#[test]
fn rejects_incompatible_typed_object_openness() {
    let mut document: Document =
        serde_json::from_str(include_str!("../../../../spec/fixtures/login.json")).unwrap();
    let user = document.types.get_mut("User").unwrap();
    let TypeKind::Struct {
        fields, flattened, ..
    } = &user.definition
    else {
        unreachable!()
    };
    user.definition = TypeKind::Struct {
        fields: fields.clone(),
        flattened: flattened.clone(),
        openness: ObjectOpenness::PreserveTyped {
            value: TypeRef::Inline(InlineType::Bytes {
                encoding: pons_ir::ByteEncoding::Base64,
            }),
        },
    };
    assert!(
        crate::emit(&document)
            .unwrap_err()
            .0
            .contains("typed object openness")
    );
}

#[test]
fn rejects_documents_with_an_unsupported_ir_version() {
    let mut document: Document =
        serde_json::from_str(include_str!("../../../../spec/fixtures/login.json")).unwrap();
    document.ir_version = "9.0".into();
    assert!(crate::emit(&document).is_err());
    assert_eq!(document.ir_version, "9.0");
    assert_eq!(IR_VERSION, "0.2");
}

#[test]
fn pagination_is_an_explicit_unsupported_capability() {
    let mut document: Document =
        serde_json::from_str(include_str!("../../../../spec/fixtures/login.json")).unwrap();
    document.operations[1].semantics.pagination = Some(
        serde_json::from_value(json!({
            "request": {"kind": "cursor", "field": "limit"},
            "items": "/items"
        }))
        .unwrap(),
    );
    let error = crate::emit(&document).unwrap_err();
    assert!(error.0.contains("does not generate pagination iterators"));

    let emitted = crate::emit_with_mode(&document, crate::EmitMode::Lossy).unwrap();
    assert!(
        emitted
            .warnings
            .iter()
            .any(|warning| warning.contains("does not generate pagination iterators"))
    );
}

#[test]
fn rejects_form_and_multipart_responses() {
    for content in [ContentKind::Form, ContentKind::Multipart] {
        let mut document: Document =
            serde_json::from_str(include_str!("../../../../spec/fixtures/login.json")).unwrap();
        document.operations[0].responses[0].content = content;
        let error = crate::emit(&document).unwrap_err();
        assert!(error.0.contains("form or multipart responses"));
    }
}

#[test]
fn rejects_a_websocket_request_body() {
    let mut document: Document =
        serde_json::from_str(include_str!("../../../../spec/fixtures/login.json")).unwrap();
    let stream_index = document
        .operations
        .iter()
        .position(|operation| operation.stream.is_some())
        .unwrap();
    document.operations[stream_index]
        .stream
        .as_mut()
        .unwrap()
        .kind = StreamKind::Websocket {
        client: pons_ir::TypeRef::Inline(pons_ir::InlineType::Any),
        server: pons_ir::TypeRef::Inline(pons_ir::InlineType::Any),
    };
    document.operations[stream_index].responses[0].content = pons_ir::ContentKind::Empty;
    document.operations[stream_index].responses[0].status = pons_ir::ResponseStatus::Exact(101);
    document.operations[stream_index].responses[0]
        .headers
        .clear();
    document.operations[stream_index].body = Some(pons_ir::Body {
        ty: pons_ir::TypeRef::Inline(pons_ir::InlineType::Any),
        content: pons_ir::ContentKind::Json,
        required: true,
        media: None,
        ext: Default::default(),
    });
    let error = crate::emit(&document).unwrap_err();
    assert!(
        error.0.contains("cannot declare a request body"),
        "{}",
        error.0
    );
}
