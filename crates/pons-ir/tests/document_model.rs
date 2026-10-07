use pons_ir::{
    ByteEncoding, ContentKind, Diagnostic, DiagnosticSeverity, Direction, Document, ErrorStatus,
    Field, FieldDefault, InlineType, NamedRef, ObjectOpenness, OpaqueWire, PathSegment, Presence,
    ResponseStatus, StreamKind, TypeDef, TypeKind, TypeRef, ValidationError,
};
use serde_json::Value;
use std::collections::BTreeMap;

#[test]
fn rich_document_round_trips_and_validates() {
    let document: Document =
        serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
    document.validate().unwrap();
    let encoded = serde_json::to_value(&document).unwrap();
    assert_eq!(encoded["ext"]["test.example.source/v1"]["kept"], true);
    assert_eq!(
        encoded["types"]["User"]["ext"]["test.example.source/v1"],
        "kept"
    );
    assert_eq!(encoded["auth_schemes"]["tenant"]["in"], "header");
    let decoded: Document = serde_json::from_value(encoded).unwrap();
    assert_eq!(document, decoded);
    assert_eq!(
        document.operations[0].errors[0].status,
        ErrorStatus::Class(4)
    );
}

#[test]
fn opaque_values_require_a_codec_and_can_be_scanned() {
    let mut document: Document =
        serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
    let user = document.types.get_mut("User").unwrap();
    let TypeKind::Struct { fields, .. } = &mut user.definition else {
        panic!()
    };
    fields[0].ty = TypeRef::Inline(InlineType::Opaque {
        codec: String::new(),
        wire: OpaqueWire::Unknown,
        ext: Default::default(),
    });
    let errors = document.validate().unwrap_err();
    assert!(
        errors
            .iter()
            .any(|error| error.to_string().contains("opaque codec cannot be empty"))
    );
    assert!(document.has_opaque());
    let TypeKind::Struct { fields, .. } = &mut document.types.get_mut("User").unwrap().definition
    else {
        panic!()
    };
    fields[0].ty = TypeRef::Inline(InlineType::Opaque {
        codec: "example".into(),
        wire: OpaqueWire::Json,
        ext: Default::default(),
    });
    document.validate().unwrap();
}

#[test]
fn common_errors_only_root_opaque_when_an_operation_inherits_them() {
    let mut document: Document =
        serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
    let mut common = document.operations[0].errors[0].clone();
    common.name = vec!["common".into(), "error".into()];
    common.body = Some(TypeRef::Inline(InlineType::Opaque {
        codec: "example".into(),
        wire: OpaqueWire::Json,
        ext: Default::default(),
    }));
    document.common_errors.push(common);
    document.operations[0].errors.clear();
    document
        .operations
        .iter_mut()
        .for_each(|operation| operation.errors_inherit = false);
    document.validate().unwrap();
    assert!(document.has_opaque());
    assert!(!document.has_reachable_opaque());

    document.operations[0].errors_inherit = true;
    assert!(document.has_reachable_opaque());
}

#[test]
fn rejects_nullable_flattened_types() {
    let mut document: Document =
        serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
    let TypeKind::Struct { flattened, .. } =
        &mut document.types.get_mut("User").unwrap().definition
    else {
        panic!()
    };
    flattened.push(TypeRef::Inline(InlineType::Nullable {
        inner: Box::new(TypeRef::Inline(InlineType::Bool)),
    }));
    assert!(
        document
            .validate()
            .unwrap_err()
            .iter()
            .any(|error| error.to_string().contains("nullable flattened"))
    );
}

#[test]
fn rejects_required_nullable_headers_and_path_parameters() {
    fn fixture() -> Document {
        serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap()
    }
    fn nullable() -> TypeRef {
        TypeRef::Inline(InlineType::Nullable {
            inner: Box::new(TypeRef::Inline(InlineType::Bool)),
        })
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

    let mut document = fixture();
    let watch = &mut document.operations[1];
    watch.headers[0].presence = Presence::Required;
    watch.headers[0].ty = nullable();
    rejects(
        &document,
        "required request header X-Request-ID cannot be nullable",
    );

    let mut document = fixture();
    document.types.insert(
        "MaybeHeader".into(),
        TypeDef {
            name: vec!["maybe".into(), "header".into()],
            direction: Direction::Input,
            definition: TypeKind::Newtype { inner: nullable() },
            origin: None,
            doc: None,
            diagnostics: Vec::new(),
            ext: BTreeMap::new(),
        },
    );
    let watch = &mut document.operations[1];
    watch.headers[0].presence = Presence::Required;
    watch.headers[0].ty = TypeRef::Named(NamedRef {
        id: "MaybeHeader".into(),
        variant: None,
    });
    rejects(
        &document,
        "required request header X-Request-ID cannot be nullable",
    );

    let mut document = fixture();
    document.operations[1].path[1] = PathSegment::Param {
        param: "cursor".into(),
        ty: nullable(),
    };
    rejects(
        &document,
        "required path parameter cursor cannot be nullable",
    );

    let mut document = fixture();
    document.operations[1].headers[0].ty = nullable();
    document.validate().unwrap();
}

#[test]
fn auth_scopes_validate_scheme_and_uniqueness() {
    let mut document: Document =
        serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
    document.operations[1]
        .auth
        .scopes
        .insert("missing".into(), vec!["read".into(), "read".into()]);
    let errors = document.validate().unwrap_err();
    assert!(
        errors
            .iter()
            .any(|error| error.to_string().contains("missing"))
    );
    assert!(
        errors
            .iter()
            .any(|error| error.to_string().contains("repeats scope read"))
    );
}

#[test]
fn media_types_validate_and_empty_payloads_reject_them() {
    let mut document: Document =
        serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
    document.operations[0]
        .body
        .as_mut()
        .unwrap()
        .media_type
        .clone_from(&Some("application/vnd.acme+json; charset=utf-8".into()));
    document.validate().unwrap();
    document.operations[0].body.as_mut().unwrap().media_type = Some("not-a-media-type".into());
    document.operations[0].responses[0].content = ContentKind::Empty;
    document.operations[0].responses[0].body = None;
    document.operations[0].responses[0].media_type = Some("application/json".into());
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
        serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
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
    document.operations[operation_index].responses[0].media_type =
        Some("application/pdf; version=2".into());
    assert!(document.validate().unwrap_err().iter().any(|error| {
        error
            .to_string()
            .contains("does not match stream kind media type")
    }));
    document.operations[operation_index].responses[0].media_type =
        Some("application/pdf; version=1; extra=value".into());
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
        serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
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
            document.operations[operation_index].responses[0].media_type = Some(accepted);
            document.validate().unwrap();
        }
        for rejected in ["application/json", "application/octet-stream", "text/plain"] {
            document.operations[operation_index].responses[0].media_type = Some(rejected.into());
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
fn object_openness_is_explicit_and_typed_references_are_validated() {
    let mut document: Document =
        serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
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
            value: TypeRef::Named(NamedRef {
                id: "Missing".into(),
                variant: None,
            }),
        },
    };
    let errors = document.validate().unwrap_err();
    assert!(
        errors
            .iter()
            .any(|error| error.to_string().contains("Missing"))
    );
    let encoded = serde_json::to_value(&document).unwrap();
    assert_eq!(
        encoded["types"]["User"]["openness"]["preserve_typed"]["value"]["ref"],
        "Missing"
    );
}

#[test]
fn response_status_variants_round_trip_and_validate() {
    let mut document: Document =
        serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
    document.operations[0].responses[0].status = ResponseStatus::Class(2);
    document.validate().unwrap();
    let encoded = serde_json::to_value(&document).unwrap();
    assert_eq!(encoded["operations"][0]["responses"][0]["status"], "2xx");
    document.operations[0].responses[0].status = ResponseStatus::Unknown;
    document.validate().unwrap();
    let encoded = serde_json::to_value(&document).unwrap();
    assert_eq!(
        encoded["operations"][0]["responses"][0]["status"],
        "unknown"
    );
    document.operations[0].responses[0].status = ResponseStatus::Default;
    document.validate().unwrap();
    let encoded = serde_json::to_value(&document).unwrap();
    assert_eq!(
        encoded["operations"][0]["responses"][0]["status"],
        "default"
    );
    document.operations[0].responses[0].status = ResponseStatus::Class(6);
    assert!(document.validate().is_err());
}

#[test]
fn error_diagnostics_block_document_validation() {
    let mut document: Document =
        serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
    document.operations[0].diagnostics.push(Diagnostic {
        severity: DiagnosticSeverity::Error,
        code: "PONS-E999".into(),
        message: "unsupported boundary".into(),
        origin: None,
        ext: BTreeMap::new(),
    });
    assert!(
        document
            .validate()
            .unwrap_err()
            .iter()
            .any(|error| error.to_string().contains("PONS-E999"))
    );
}

#[test]
fn validation_finds_nested_unknown_types_and_bad_contract_links() {
    let json = r#"{
        "ir_version": "0.2",
        "info": {"title": "Example", "version": "1"},
        "types": {"Items": {"kind": "struct", "openness": "reject", "name": ["items"], "direction": "both", "fields": [
            {"wire": "items", "ident": ["items"], "ty": {"kind": "array", "item": {"ref": "Missing"}}, "presence": "required"}
        ]}},
        "operations": [{
            "id": [["items"], ["get"]], "method": "GET", "path": [],
            "responses": [{"status": 200, "content": "empty", "headers": [
                {"wire": "X-Missing", "ty": {"ref": "MissingHeader"}, "presence": "required"}
            ]}],
            "auth": {"alternatives": [["missing_scheme"]], "optional": false},
            "semantics": {"safe": true, "idempotent": true}
        }, {
            "id": [["items"], ["get"]], "method": "GET", "path": [],
            "auth": {"alternatives": [], "optional": false},
            "semantics": {"safe": true, "idempotent": true}
        }]
    }"#;

    let document: Document = serde_json::from_str(json).unwrap();
    let errors = document.validate().unwrap_err();
    assert!(errors.iter().any(|error| matches!(error, ValidationError::UnknownType { reference, .. } if reference == "Missing")));
    assert!(errors.iter().any(|error| matches!(error, ValidationError::UnknownType { reference, .. } if reference == "MissingHeader")));
    assert!(errors.iter().any(|error| matches!(error, ValidationError::UnknownAuthScheme { scheme, .. } if scheme == "missing_scheme")));
    assert!(
        errors
            .iter()
            .any(|error| matches!(error, ValidationError::DuplicateOperation { .. }))
    );
}

#[test]
fn validation_rejects_inconsistent_payload_header_and_numeric_contracts() {
    let json = r#"{
        "ir_version": "0.2",
        "info": {"title": "Example", "version": "1"},
        "types": {
            "Bad": {
                "kind": "struct",
                "openness": "reject",
                "name": ["bad"],
                "direction": "both",
                "fields": [{
                    "wire": "value",
                    "ident": ["value"],
                    "ty": {
                        "kind": "int",
                        "bits": 32,
                        "signed": true,
                        "wire": "number",
                        "minimum": "zero",
                        "maximum": null,
                        "multiple_of": 0
                    },
                    "presence": "required"
                }, {
                    "wire": "bounded",
                    "ident": ["bounded"],
                    "ty": {
                        "kind": "int",
                        "bits": 8,
                        "signed": false,
                        "wire": "number",
                        "minimum": -1,
                        "maximum": 300
                    },
                    "presence": "required"
                }]
            },
            "Empty": {
                "kind": "enum",
                "name": ["empty"],
                "direction": "both",
                "repr": {"kind": "string"},
                "variants": []
            }
        },
        "operations": [{
            "id": [["test"]],
            "method": "POST",
            "path": [],
            "headers": [
                {"wire": "X-Trace", "ident": ["trace"], "ty": {"kind": "string"}, "presence": "optional"},
                {"wire": "x-trace", "ident": ["trace"], "ty": {"kind": "string"}, "presence": "optional"},
                {"wire": "bad header", "ident": ["bad"], "ty": {"kind": "string"}, "presence": "optional"}
            ],
            "body": {"ty": {"kind": "string"}, "content": "empty", "required": true},
            "responses": [
                {"status": 200, "content": "json"},
                {"status": 201, "content": "empty", "body": {"kind": "string"}},
                {"status": 202, "content": "stream"}
            ],
            "errors": [{
                "status": 400,
                "name": ["bad"],
                "content": "empty",
                "body": {"kind": "string"}
            }],
            "auth": {"alternatives": [], "optional": false},
            "semantics": {"safe": false, "idempotent": false}
        }]
    }"#;

    let document: Document = serde_json::from_str(json).unwrap();
    let errors = document.validate().unwrap_err();
    let messages = errors
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(messages.contains("integer minimum must be a number"));
    assert!(messages.contains("integer multiple_of must be greater than zero"));
    assert!(messages.contains("integer minimum is below its native range"));
    assert!(messages.contains("integer maximum is above its native range"));
    assert!(messages.contains("enum needs at least one variant"));
    assert!(messages.contains("request body content Empty"));
    assert!(messages.contains("duplicate request header x-trace"));
    assert!(messages.contains("not a valid HTTP field name"));
    assert!(messages.contains("response 200 content Json"));
    assert!(messages.contains("response 201 content Empty"));
    assert!(messages.contains("0 stream descriptions and 1 stream responses"));
    assert!(messages.contains("error content Empty"));
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
        serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
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

fn string() -> TypeRef {
    TypeRef::Inline(InlineType::String {
        format: None,
        min_length: None,
        max_length: None,
        pattern: None,
    })
}

fn array(item: TypeRef) -> TypeRef {
    TypeRef::Inline(InlineType::Array {
        item: Box::new(item),
        unique: None,
    })
}

fn nullable(inner: TypeRef) -> TypeRef {
    TypeRef::Inline(InlineType::Nullable {
        inner: Box::new(inner),
    })
}

fn map(value: TypeRef) -> TypeRef {
    TypeRef::Inline(InlineType::Map {
        key: Box::new(string()),
        value: Box::new(value),
    })
}

fn named(id: &str) -> TypeRef {
    TypeRef::Named(NamedRef {
        id: id.into(),
        variant: None,
    })
}

fn named_type(id: &'static str, definition: TypeKind) -> (&'static str, TypeDef) {
    (
        id,
        TypeDef {
            name: vec![id.into()],
            direction: Direction::Both,
            definition,
            origin: None,
            doc: None,
            diagnostics: Vec::new(),
            ext: BTreeMap::new(),
        },
    )
}

fn profile() -> (&'static str, TypeDef) {
    named_type(
        "Profile",
        TypeKind::Struct {
            fields: Vec::new(),
            flattened: Vec::new(),
            openness: ObjectOpenness::Reject,
        },
    )
}

fn form_document(fields: Vec<(&str, TypeRef)>, extra: Vec<(&str, TypeDef)>) -> Document {
    form_document_with(Presence::Required, fields, extra)
}

fn form_document_with(
    presence: Presence,
    fields: Vec<(&str, TypeRef)>,
    extra: Vec<(&str, TypeDef)>,
) -> Document {
    let mut document: Document =
        serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
    for (id, definition) in extra {
        document.types.insert(id.into(), definition);
    }
    document.types.insert(
        "FormBody".into(),
        TypeDef {
            name: vec!["form".into(), "body".into()],
            direction: Direction::Input,
            definition: TypeKind::Struct {
                fields: fields
                    .into_iter()
                    .map(|(wire, ty)| Field {
                        wire: wire.into(),
                        ident: vec![wire.into()],
                        ty,
                        presence,
                        default: FieldDefault::Unspecified,
                        access: None,
                        doc: None,
                        diagnostics: Vec::new(),
                        ext: BTreeMap::new(),
                    })
                    .collect(),
                flattened: Vec::new(),
                openness: ObjectOpenness::Reject,
            },
            origin: None,
            doc: None,
            diagnostics: Vec::new(),
            ext: BTreeMap::new(),
        },
    );
    let body = document.operations[0].body.as_mut().unwrap();
    body.content = ContentKind::Form;
    body.ty = named("FormBody");
    document
}

fn form_errors(document: &Document) -> Vec<String> {
    document
        .validate()
        .unwrap_err()
        .iter()
        .map(ToString::to_string)
        .collect()
}

fn refuses_form_field(document: &Document, wire: &str) {
    let errors = form_errors(document);
    let blamed = format!("form body field {wire:?}");
    assert!(
        errors.iter().any(|error| error.contains(&blamed)),
        "a form body value the carrier cannot spell is blamed by its own field: {errors:?}"
    );
}

fn refuses_form(document: &Document, message: &str) {
    let errors = form_errors(document);
    assert!(
        errors.iter().any(|error| error.contains(message)),
        "{message:?} missing from {errors:?}"
    );
}

#[test]
fn a_form_body_is_a_record_of_flat_values_and_nothing_else() {
    for (fields, extra, body) in [
        (vec![("email", string())], vec![], None),
        (
            vec![("email", string())],
            vec![named_type(
                "Wrapped",
                TypeKind::Newtype {
                    inner: named("FormBody"),
                },
            )],
            Some(named("Wrapped")),
        ),
    ] {
        let mut document = form_document(fields, extra);
        if let Some(body) = body {
            document.operations[0].body.as_mut().unwrap().ty = body;
        }
        document.validate().unwrap();
    }

    let mut opaque = form_document(vec![("email", string())], vec![]);
    opaque.operations[0].body.as_mut().unwrap().ty = TypeRef::Inline(InlineType::Opaque {
        codec: "axum.form".into(),
        wire: OpaqueWire::Unknown,
        ext: Default::default(),
    });
    refuses_form(
        &opaque,
        "a form body must be a struct or a transparent newtype over a struct, because a urlencoded form writes one key per field",
    );

    for (wire, fields, extra) in [
        (
            "profile",
            vec![("profile", named("Profile"))],
            vec![profile()],
        ),
        ("profile", vec![("profile", map(string()))], vec![]),
        (
            "profile",
            vec![("profile", named("WrappedProfile"))],
            vec![
                profile(),
                named_type(
                    "WrappedProfile",
                    TypeKind::Newtype {
                        inner: named("Profile"),
                    },
                ),
            ],
        ),
    ] {
        refuses_form_field(&form_document(fields, extra), wire);
    }

    refuses_form(
        &form_document(vec![("email", nullable(string()))], vec![]),
        "form body field \"email\" is required and nullable; a urlencoded form field has no encoding for a present null",
    );
    form_document_with(
        Presence::Optional,
        vec![("email", nullable(string()))],
        vec![],
    )
    .validate()
    .unwrap();
}

#[test]
fn a_form_body_carries_one_array_level_and_no_more() {
    form_document(vec![("tags", array(string()))], vec![])
        .validate()
        .unwrap();
    form_document_with(
        Presence::Optional,
        vec![("tags", nullable(array(string())))],
        vec![],
    )
    .validate()
    .unwrap();

    for (wire, fields, extra) in [
        (
            "matrix",
            vec![("matrix", array(array(string())))],
            Vec::new(),
        ),
        (
            "profiles",
            vec![("profiles", array(named("Profile")))],
            vec![profile()],
        ),
    ] {
        refuses_form_field(&form_document(fields, extra), wire);
    }
    refuses_form(
        &form_document(vec![("tags", nullable(array(string())))], vec![]),
        "form body field \"tags\" is required and nullable",
    );
}

#[test]
fn status_defaults_and_classes_keep_their_wire_form() {
    let status: ErrorStatus = serde_json::from_str("\"default\"").unwrap();
    assert_eq!(serde_json::to_string(&status).unwrap(), "\"default\"");
    let status: ErrorStatus = serde_json::from_str("\"5xx\"").unwrap();
    assert_eq!(serde_json::to_string(&status).unwrap(), "\"5xx\"");
}

#[test]
fn checked_in_schema_matches_the_rust_model() {
    let generated = serde_json::to_value(schemars::schema_for!(Document)).unwrap();
    let checked_in: Value = serde_json::from_str(include_str!("../schema/ir.schema.json")).unwrap();
    assert_eq!(generated, checked_in);
}

#[test]
fn canonical_json_ignores_order_of_unordered_contract_members() {
    let document: Document =
        serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
    let mut shuffled = document.clone();
    shuffled.operations.reverse();
    if let TypeKind::Struct { fields, .. } = &mut shuffled.types.get_mut("User").unwrap().definition
    {
        fields.reverse();
    }

    assert_ne!(
        serde_json::to_string(&document).unwrap(),
        serde_json::to_string(&shuffled).unwrap()
    );
    assert_eq!(
        document.to_canonical_json().unwrap(),
        shuffled.to_canonical_json().unwrap()
    );
    assert_eq!(shuffled.operations[0].id[0], ["events"]);
}
