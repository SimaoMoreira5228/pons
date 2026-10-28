#![allow(dead_code)]

use std::{collections::BTreeMap, fs, process::Command};

use pons_emit_ts::{
    ErrorStrategy, GeneratedFile, TsOptions, ValidationBackend, ValidationOptions,
    emit_with_options,
};
use pons_ir::{
    ByteEncoding, Direction, EnumRepr, InlineType, IntegerWire, NamedRef, OpaqueWire, PathSegment,
    Presence, TypeKind, TypeRef, Variant, VariantPayload,
};

pub fn constrained_integer(wire: IntegerWire, bits: u16) -> TypeRef {
    TypeRef::Inline(InlineType::Int {
        bits,
        signed: false,
        wire,
        minimum: Some(serde_json::json!(10)),
        maximum: Some(serde_json::json!(250)),
        multiple_of: Some(serde_json::json!(5)),
    })
}

pub fn wide_number_document() -> pons_ir::Document {
    let mut document: pons_ir::Document =
        serde_json::from_str(include_str!("../../../../spec/fixtures/login.json")).unwrap();
    let user = document.types.get_mut("User").unwrap();
    let TypeKind::Struct { fields, .. } = &mut user.definition else {
        unreachable!()
    };
    fields[0].ty = TypeRef::Inline(InlineType::Int {
        bits: 64,
        signed: false,
        wire: IntegerWire::Number,
        minimum: None,
        maximum: None,
        multiple_of: None,
    });
    document
}

pub fn opaque_document() -> pons_ir::Document {
    let mut document: pons_ir::Document =
        serde_json::from_str(include_str!("../../../../spec/fixtures/login.json")).unwrap();
    let user = document.types.get_mut("User").unwrap();
    let TypeKind::Struct { fields, .. } = &mut user.definition else {
        unreachable!()
    };
    fields[0].ty = TypeRef::Inline(InlineType::Opaque {
        codec: "example.codec".into(),
        wire: OpaqueWire::Json,
        ext: Default::default(),
    });
    let login = document.types.get_mut("LoginRequest").unwrap();
    let TypeKind::Struct { fields, .. } = &mut login.definition else {
        unreachable!()
    };
    fields[0].ty = TypeRef::Inline(InlineType::Opaque {
        codec: "example.codec".into(),
        wire: OpaqueWire::Json,
        ext: Default::default(),
    });
    document
}

pub fn flattened_opaque_document() -> pons_ir::Document {
    let mut document: pons_ir::Document =
        serde_json::from_str(include_str!("../../../../spec/fixtures/login.json")).unwrap();
    for id in ["LoginRequest", "User"] {
        let definition = document.types.get_mut(id).unwrap();
        let TypeKind::Struct { fields, .. } = &definition.definition else {
            unreachable!()
        };
        definition.definition = TypeKind::Struct {
            fields: fields.clone(),
            flattened: vec![TypeRef::Inline(InlineType::Opaque {
                codec: "flatten.codec".into(),
                wire: OpaqueWire::Json,
                ext: Default::default(),
            })],
            openness: pons_ir::ObjectOpenness::Reject,
        };
    }
    document.operations[0].provides = None;
    document
}

pub fn flattened_named_opaque_document() -> pons_ir::Document {
    let mut raw: serde_json::Value =
        serde_json::from_str(include_str!("../../../../spec/fixtures/login.json")).unwrap();
    raw["types"]["OpaqueInner"] = serde_json::json!({
        "kind": "newtype",
        "name": ["opaque", "inner"],
        "direction": "both",
        "inner": {"kind": "opaque", "codec": "flatten.codec", "wire": "json"}
    });
    raw["types"]["OpaqueAlias"] = serde_json::json!({
        "kind": "newtype",
        "name": ["opaque", "alias"],
        "direction": "both",
        "inner": {"ref": "OpaqueInner"}
    });
    for id in ["LoginRequest", "User"] {
        raw["types"][id]["flattened"] = serde_json::json!([{"ref": "OpaqueAlias"}]);
    }
    serde_json::from_value(raw).unwrap()
}

pub fn nested_flattened_opaque_document() -> pons_ir::Document {
    let mut raw: serde_json::Value =
        serde_json::from_str(include_str!("../../../../spec/fixtures/login.json")).unwrap();
    raw["types"]["OpaqueAlias"] = serde_json::json!({
        "kind": "newtype",
        "name": ["opaque", "alias"],
        "direction": "both",
        "inner": {"kind": "opaque", "codec": "flatten.codec", "wire": "json"}
    });
    raw["types"]["OpaqueEnvelope"] = serde_json::json!({
        "kind": "struct",
        "name": ["opaque", "envelope"],
        "direction": "both",
        "openness": "reject",
        "fields": [
            {"wire": "tag", "ident": ["tag"], "ty": {"kind": "string"}, "presence": "required"}
        ],
        "flattened": [{"ref": "OpaqueAlias"}]
    });
    for id in ["LoginRequest", "User"] {
        raw["types"][id]["flattened"] = serde_json::json!([{"ref": "OpaqueEnvelope"}]);
    }
    let mut document: pons_ir::Document = serde_json::from_value(raw).unwrap();
    document.operations[0].provides = None;
    document
}

pub fn flattened_with_sibling_opaque_document() -> pons_ir::Document {
    let mut document = flattened_opaque_document();
    for id in ["LoginRequest", "User"] {
        let definition = document.types.get_mut(id).unwrap();
        let TypeKind::Struct { fields, .. } = &mut definition.definition else {
            unreachable!()
        };
        fields[0].ty = TypeRef::Inline(InlineType::Opaque {
            codec: "sibling.codec".into(),
            wire: OpaqueWire::Json,
            ext: Default::default(),
        });
    }
    document
}

pub fn request_input_document(required: bool) -> pons_ir::Document {
    let mut document: pons_ir::Document =
        serde_json::from_str(include_str!("../../../../spec/fixtures/login.json")).unwrap();
    let operation = &mut document.operations[1];
    for segment in &mut operation.path {
        if let PathSegment::Param { ty, .. } = segment {
            *ty = constrained_integer(IntegerWire::Number, 8);
            break;
        }
    }
    operation.headers[0].ty = constrained_integer(IntegerWire::Number, 8);
    operation.headers[0].presence = if required {
        Presence::Required
    } else {
        Presence::Optional
    };
    let query = document.types.get_mut("EventQuery").unwrap();
    let TypeKind::Struct { fields, .. } = &mut query.definition else {
        unreachable!()
    };
    fields[0].ty = constrained_integer(IntegerWire::Number, 8);
    fields[0].presence = if required {
        Presence::Required
    } else {
        Presence::Optional
    };
    document
}

pub fn untagged_bytes_document() -> pons_ir::Document {
    let mut document: pons_ir::Document =
        serde_json::from_str(include_str!("../../../../spec/fixtures/login.json")).unwrap();
    let mut blob = document.types.get("LoginError").unwrap().clone();
    blob.name = vec!["blob".into()];
    blob.direction = Direction::Both;
    blob.definition = TypeKind::Enum {
        repr: EnumRepr::Untagged,
        variants: vec![Variant {
            wire: "bytes".into(),
            ident: vec!["bytes".into()],
            payload: VariantPayload::Newtype {
                ty: TypeRef::Inline(InlineType::Bytes {
                    encoding: ByteEncoding::Base64,
                }),
            },
            doc: None,
            diagnostics: Vec::new(),
            ext: BTreeMap::new(),
        }],
    };
    document.types.insert("Blob".into(), blob);
    let blob_type = TypeRef::Named(NamedRef {
        id: "Blob".into(),
        variant: None,
    });
    let user = document.types.get_mut("User").unwrap();
    let TypeKind::Struct { fields, .. } = &mut user.definition else {
        unreachable!()
    };
    fields[0].ty = blob_type.clone();
    let login = document.types.get_mut("LoginRequest").unwrap();
    let TypeKind::Struct { fields, .. } = &mut login.definition else {
        unreachable!()
    };
    fields[0].ty = blob_type;
    document
}

pub fn validated_files(document: &pons_ir::Document) -> Vec<GeneratedFile> {
    emit_with_options(
        document,
        TsOptions {
            validation: ValidationOptions::new(ValidationBackend::Builtin),
            errors: ErrorStrategy::Throw,
            ..TsOptions::default()
        },
    )
    .unwrap()
    .files
}

pub fn paginated_document(pagination: serde_json::Value) -> pons_ir::Document {
    let document: pons_ir::Document = serde_json::from_value(serde_json::json!({
        "ir_version": "0.2",
        "info": {"title": "Pages", "version": "1"},
        "types": {
            "EventQuery": {
                "kind": "struct",
                "name": ["event", "query"],
                "direction": "input",
                "openness": "reject",
                "fields": [
                    {"wire": "cursor", "ident": ["cursor"], "ty": {"kind": "string"}, "presence": "optional"},
                    {"wire": "offset", "ident": ["offset"], "ty": {"kind": "int", "bits": 32, "signed": false, "wire": "number"}, "presence": "optional"},
                    {"wire": "limit", "ident": ["limit"], "ty": {"kind": "int", "bits": 32, "signed": false, "wire": "number"}, "presence": "optional"}
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
            "EventPage": {
                "kind": "struct",
                "name": ["event", "page"],
                "direction": "output",
                "openness": "reject",
                "fields": [
                    {"wire": "items", "ident": ["items"], "ty": {"kind": "array", "item": {"ref": "Event"}}, "presence": "required"},
                    {"wire": "next_cursor", "ident": ["next", "cursor"], "ty": {"kind": "nullable", "inner": {"kind": "string"}}, "presence": "optional"}
                ]
            }
        },
        "operations": [{
            "id": [["events"], ["list"]],
            "method": "GET",
            "path": [{"lit": "events"}],
            "query": {"ref": "EventQuery"},
            "responses": [{"status": 200, "content": "json", "body": {"ref": "EventPage"}}],
            "errors": [{"status": 404, "name": ["missing"], "content": "json", "body": {"kind": "string"}}],
            "auth": {"alternatives": [], "optional": false},
            "semantics": {"safe": true, "idempotent": true, "pagination": pagination}
        }]
    }))
    .unwrap();
    document.validate().unwrap();
    document
}

pub fn run_bun(files: &[GeneratedFile], harness: &str) {
    let version = Command::new("bun")
        .arg("--version")
        .output()
        .expect("Bun is required for generated TypeScript runtime tests");
    assert!(version.status.success());
    let directory = tempfile::tempdir().unwrap();
    for file in files {
        let destination = directory.path().join(&file.path);
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(destination, &file.content).unwrap();
    }
    let harness_path = directory.path().join("validation.ts");
    fs::write(&harness_path, harness).unwrap();
    let output = Command::new("bun")
        .arg("run")
        .arg(&harness_path)
        .current_dir(directory.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
