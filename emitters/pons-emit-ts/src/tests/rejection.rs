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
    assert_eq!(IR_VERSION, "0.3");
}

#[test]
fn an_items_pointer_that_does_not_reach_an_array_is_lossy() {
    let document = paginated_document(json!({
        "kind": "struct",
        "name": ["event", "page"],
        "direction": "output",
        "openness": "reject",
        "fields": [
            {"wire": "items", "ident": ["items"], "ty": {
                "kind": "map", "key": {"kind": "string"}, "value": {"kind": "string"}
            }, "presence": "required"}
        ]
    }));
    let strict = crate::emit_with_mode(&document, crate::EmitMode::Strict)
        .unwrap_err()
        .0;
    assert!(strict.contains("operations.events.list"), "{strict}");
    assert!(strict.contains("lossy coverage"), "{strict}");

    let emitted = crate::emit_with_mode(&document, crate::EmitMode::Lossy).unwrap();
    assert!(
        emitted.coverage.lossy.contains("operations.events.list"),
        "{:?}",
        emitted.coverage
    );
    let events = emitted
        .files
        .iter()
        .find(|file| file.path == "operations/events.ts")
        .unwrap();
    assert!(!events.content.contains("listAll"), "{}", events.content);
    assert!(!events.content.contains("paginate"), "{}", events.content);
    let warning = emitted
        .warnings
        .iter()
        .find(|warning| warning.contains("operations.events.list"))
        .unwrap();
    assert!(warning.contains("\"/items/values\""), "{warning}");
}

#[test]
fn a_pagination_field_declared_on_both_carriers_is_refused() {
    let mut document = paginated_document(json!({
        "kind": "struct",
        "name": ["event", "page"],
        "direction": "output",
        "openness": "reject",
        "fields": [
            {"wire": "items", "ident": ["items"], "ty": {
                "kind": "array", "item": {"ref": "Event"}
            }, "presence": "required"}
        ]
    }));
    document.operations[0].body = Some(
        serde_json::from_value(json!({
            "ty": {"ref": "EventQuery"},
            "content": "json",
            "required": true
        }))
        .unwrap(),
    );
    document.validate().unwrap();
    let error = crate::emit_with_mode(&document, crate::EmitMode::Lossy)
        .unwrap_err()
        .0;
    assert!(error.contains("both the query and the body"), "{error}");
}

fn paginated_document(page: serde_json::Value) -> Document {
    serde_json::from_value(json!({
        "ir_version": IR_VERSION,
        "info": {"title": "Pages", "version": "1"},
        "types": {
            "EventQuery": {
                "kind": "struct",
                "name": ["event", "query"],
                "direction": "input",
                "openness": "reject",
                "fields": [
                    {"wire": "cursor", "ident": ["cursor"], "ty": {"kind": "string"}, "presence": "optional"}
                ]
            },
            "Event": {
                "kind": "struct",
                "name": ["event"],
                "direction": "output",
                "openness": "reject",
                "fields": [
                    {"wire": "id", "ident": ["id"], "ty": {"kind": "string"}, "presence": "required"}
                ]
            },
            "EventPage": page
        },
        "operations": [{
            "id": [["events"], ["list"]],
            "method": "GET",
            "path": [{"lit": "events"}],
            "query": {"ref": "EventQuery"},
            "responses": [{"status": 200, "content": "json", "body": {"ref": "EventPage"}}],
            "auth": {"alternatives": [], "optional": false},
            "semantics": {
                "safe": true,
                "idempotent": true,
                "pagination": {
                    "request": {"kind": "cursor", "field": "cursor"},
                    "items": "/items/values",
                    "next": "/next_cursor"
                }
            }
        }]
    }))
    .unwrap()
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
