#![allow(dead_code)]

use std::{collections::BTreeMap, fs, path::Path, process::Command};

use pons_emit_ts::{
    EmitMode, ErrorStrategy, GeneratedFile, Runtime, TsOptions, ValidationBackend,
    ValidationOptions, emit_with_options,
};
use pons_ir::{
    ByteEncoding, Direction, EnumRepr, InlineType, IntegerWire, NamedRef, OpaqueWire, PathSegment,
    Presence, TypeKind, TypeRef, Variant, VariantPayload,
};

pub fn fixture() -> serde_json::Value {
    serde_json::from_str(include_str!("../../../../spec/fixtures/login.json")).unwrap()
}

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

pub fn kitchen_sink() -> pons_ir::Document {
    let mut raw = fixture();
    raw["types"] = serde_json::json!({
        "Scalars": {
            "kind": "struct", "name": ["scalars"], "direction": "both", "openness": "reject",
            "fields": [
                {"wire": "small", "ident": ["small"], "ty": {"kind": "int", "bits": 8, "signed": true, "wire": "number", "minimum": -5, "maximum": 5, "multiple_of": 5}, "presence": "required"},
                {"wire": "wide", "ident": ["wide"], "ty": {"kind": "int", "bits": 64, "signed": true, "wire": "number"}, "presence": "required"},
                {"wire": "text", "ident": ["text"], "ty": {"kind": "int", "bits": 32, "signed": false, "wire": "string"}, "presence": "required"},
                {"wire": "single", "ident": ["single"], "ty": {"kind": "float", "bits": 32}, "presence": "required"},
                {"wire": "double", "ident": ["double"], "ty": {"kind": "float", "bits": 64}, "presence": "required"},
                {"wire": "money", "ident": ["money"], "ty": {"kind": "decimal", "wire": "number"}, "presence": "required"},
                {"wire": "exact", "ident": ["exact"], "ty": {"kind": "decimal", "wire": "string"}, "presence": "required"},
                {"wire": "uuid", "ident": ["uuid"], "ty": {"kind": "string", "format": "uuid"}, "presence": "required"},
                {"wire": "day", "ident": ["day"], "ty": {"kind": "string", "format": "date"}, "presence": "required"},
                {"wire": "clock", "ident": ["clock"], "ty": {"kind": "string", "format": "time"}, "presence": "required"},
                {"wire": "span", "ident": ["span"], "ty": {"kind": "string", "format": "duration"}, "presence": "required"},
                {"wire": "link", "ident": ["link"], "ty": {"kind": "string", "format": "uri"}, "presence": "required"},
                {"wire": "who", "ident": ["who"], "ty": {"kind": "string", "format": "email"}, "presence": "required"},
                {"wire": "v4", "ident": ["v4"], "ty": {"kind": "string", "format": "ipv4"}, "presence": "required"},
                {"wire": "v6", "ident": ["v6"], "ty": {"kind": "string", "format": "ipv6"}, "presence": "required"},
                {"wire": "moment", "ident": ["moment"], "ty": {"kind": "string", "format": "date-time"}, "presence": "required"},
                {"wire": "sized", "ident": ["sized"], "ty": {"kind": "string", "min_length": 2, "max_length": 4}, "presence": "required"},
                {"wire": "shaped", "ident": ["shaped"], "ty": {"kind": "string", "pattern": "^[a-z]+$"}, "presence": "required"},
                {"wire": "blob", "ident": ["blob"], "ty": {"kind": "bytes", "encoding": "base64"}, "presence": "required"},
                {"wire": "nothing", "ident": ["nothing"], "ty": {"kind": "null"}, "presence": "required"},
                {"wire": "whatever", "ident": ["whatever"], "ty": {"kind": "any"}, "presence": "required"},
                {"wire": "secret", "ident": ["secret"], "ty": {"kind": "opaque", "codec": "c", "wire": "json"}, "presence": "required"},
                {"wire": "maybe", "ident": ["maybe"], "ty": {"kind": "nullable", "inner": {"kind": "int", "bits": 32, "signed": false, "wire": "number"}}, "presence": "required"},
                {"wire": "many", "ident": ["many"], "ty": {"kind": "array", "item": {"kind": "int", "bits": 32, "signed": false, "wire": "number"}, "unique": true}, "presence": "required"},
                {"wire": "table", "ident": ["table"], "ty": {"kind": "map", "key": {"kind": "string"}, "value": {"kind": "int", "bits": 32, "signed": false, "wire": "number"}}, "presence": "required"},
                {"wire": "pair", "ident": ["pair"], "ty": {"kind": "tuple", "items": [{"kind": "string"}, {"kind": "bool"}]}, "presence": "required"},
                {"wire": "maybe_missing", "ident": ["maybe", "missing"], "ty": {"kind": "string"}, "presence": "optional"}
            ]
        },
        "Choices": {
            "kind": "enum", "name": ["choices"], "direction": "both", "repr": {"kind": "string"},
            "variants": [
                {"wire": "one", "ident": ["one"], "payload": {"kind": "unit"}},
                {"wire": "two", "ident": ["two"], "payload": {"kind": "unit"}}
            ]
        },
        "Spreads": {
            "kind": "enum", "name": ["spreads"], "direction": "both", "repr": {"kind": "external"},
            "variants": [
                {"wire": "bare", "ident": ["bare"], "payload": {"kind": "unit"}},
                {"wire": "held", "ident": ["held"], "payload": {"kind": "newtype", "ty": {"kind": "int", "bits": 32, "signed": false, "wire": "number"}}}
            ]
        },
        "Marked": {
            "kind": "enum", "name": ["marked"], "direction": "both", "repr": {"kind": "internal", "tag": "kind"},
            "variants": [
                {"wire": "none", "ident": ["none"], "payload": {"kind": "unit"}},
                {"wire": "some", "ident": ["some"], "payload": {"kind": "struct", "fields": [
                    {"wire": "at", "ident": ["at"], "ty": {"kind": "int", "bits": 32, "signed": false, "wire": "number"}, "presence": "required"}
                ]}}
            ]
        },
        "Beside": {
            "kind": "enum", "name": ["beside"], "direction": "both", "repr": {"kind": "adjacent", "tag": "kind", "content": "payload"},
            "variants": [
                {"wire": "none", "ident": ["none"], "payload": {"kind": "unit"}},
                {"wire": "some", "ident": ["some"], "payload": {"kind": "newtype", "ty": {"kind": "bool"}}}
            ]
        },
        "Shapes": {
            "kind": "enum", "name": ["shapes"], "direction": "both", "repr": {"kind": "untagged"},
            "variants": [
                {"wire": "nothing", "ident": ["nothing"], "payload": {"kind": "unit"}},
                {"wire": "count", "ident": ["count"], "payload": {"kind": "newtype", "ty": {"kind": "int", "bits": 32, "signed": false, "wire": "number"}}},
                {"wire": "named", "ident": ["named"], "payload": {"kind": "struct", "fields": [
                    {"wire": "name", "ident": ["name"], "ty": {"kind": "string"}, "presence": "required"}
                ]}},
                {"wire": "full", "ident": ["full"], "payload": {"kind": "struct", "fields": [
                    {"wire": "name", "ident": ["name"], "ty": {"kind": "string"}, "presence": "required"},
                    {"wire": "extra", "ident": ["extra"], "ty": {"kind": "string"}, "presence": "required"}
                ]}}
            ]
        },
        "Loose": {
            "kind": "struct", "name": ["loose"], "direction": "both", "openness": "preserve_opaque",
            "fields": [{"wire": "a", "ident": ["a"], "ty": {"kind": "string"}, "presence": "required"}]
        },
        "Typed": {
            "kind": "struct", "name": ["typed"], "direction": "both",
            "openness": {"preserve_typed": {"value": {"kind": "int", "bits": 32, "signed": false, "wire": "number"}}},
            "fields": [{"wire": "a", "ident": ["a"], "ty": {"kind": "int", "bits": 32, "signed": false, "wire": "number"}, "presence": "required"}]
        },
        "Sealed": {
            "kind": "struct", "name": ["sealed"], "direction": "both", "openness": "reject",
            "fields": [{"wire": "a", "ident": ["a"], "ty": {"kind": "string"}, "presence": "required"}]
        },
        "Part": {
            "kind": "struct", "name": ["part"], "direction": "both", "openness": "reject",
            "fields": [{"wire": "b", "ident": ["b"], "ty": {"kind": "bool"}, "presence": "required"}]
        },
        "Whole": {
            "kind": "struct", "name": ["whole"], "direction": "both", "openness": "reject",
            "fields": [{"wire": "a", "ident": ["a"], "ty": {"kind": "string"}, "presence": "required"}],
            "flattened": [{"ref": "Part"}]
        },
        "Alias": {"kind": "newtype", "name": ["alias"], "direction": "both", "inner": {"ref": "Deeper"}},
        "Deeper": {"kind": "newtype", "name": ["deeper"], "direction": "both", "inner": {"kind": "int", "bits": 32, "signed": false, "wire": "number"}},
        "Reading": {
            "kind": "struct", "name": ["reading"], "direction": "input", "openness": "reject",
            "fields": [
                {"wire": "in", "ident": ["in"], "ty": {"kind": "string"}, "presence": "required"},
                {"wire": "secret", "ident": ["secret"], "ty": {"kind": "string"}, "presence": "required", "access": "write_only"}
            ]
        },
        "Written": {
            "kind": "struct", "name": ["written"], "direction": "output", "openness": "reject",
            "fields": [
                {"wire": "out", "ident": ["out"], "ty": {"kind": "string"}, "presence": "required"},
                {"wire": "server", "ident": ["server"], "ty": {"kind": "string"}, "presence": "required", "access": "read_only"}
            ]
        },
        "Tagged": {
            "kind": "struct", "name": ["tagged"], "direction": "output", "openness": "reject",
            "fields": [{"wire": "id", "ident": ["id"], "ty": {"kind": "string"}, "presence": "required", "access": "read_only"}]
        },
        "Gathered": {
            "kind": "struct", "name": ["gathered"], "direction": "both", "openness": "reject",
            "fields": [{"wire": "own", "ident": ["own"], "ty": {"kind": "string"}, "presence": "required"}],
            "flattened": [{"ref": "Tagged"}]
        },
        "Tree": {
            "kind": "struct", "name": ["tree"], "direction": "both", "openness": "reject",
            "fields": [
                {"wire": "value", "ident": ["value"], "ty": {"kind": "int", "bits": 32, "signed": false, "wire": "number"}, "presence": "required"},
                {"wire": "next", "ident": ["next"], "ty": {"kind": "nullable", "inner": {"ref": "Tree"}}, "presence": "required"}
            ]
        },
        "Opaque": {"kind": "newtype", "name": ["opaque"], "direction": "both", "inner": {"kind": "opaque", "codec": "c", "wire": "json"}},
        "SealedOpaque": {
            "kind": "struct", "name": ["sealed", "opaque"], "direction": "both", "openness": "reject",
            "fields": [{"wire": "a", "ident": ["a"], "ty": {"kind": "string"}, "presence": "required"}],
            "flattened": [{"ref": "Opaque"}]
        }
    });
    raw["operations"] = serde_json::json!([{
        "id": [["kitchen"], ["sink"]],
        "method": "GET",
        "path": [{"lit": "kitchen"}],
        "auth": {"alternatives": [], "optional": false},
        "semantics": {"safe": true, "idempotent": true},
        "responses": [{"status": 200, "content": "json", "body": {"ref": "Scalars"}}],
        "errors": [{"status": 400, "name": ["kitchen"], "content": "json", "body": {"ref": "Shapes", "variant": "count"}}]
    }]);
    serde_json::from_value(raw).unwrap()
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

pub fn emitted_files(document: &pons_ir::Document, options: TsOptions) -> Vec<GeneratedFile> {
    emit_with_options(document, options)
        .expect("the TypeScript emitter must accept the document")
        .files
}

pub fn generated_file<'a>(files: &'a [GeneratedFile], path: &str) -> &'a str {
    &files
        .iter()
        .find(|file| file.path == path)
        .unwrap_or_else(|| panic!("the emitter wrote no {path}"))
        .content
}

pub fn zod_files(document: &pons_ir::Document, options: TsOptions) -> Vec<GeneratedFile> {
    emitted_files(
        document,
        TsOptions {
            validation: ValidationOptions::new(ValidationBackend::Zod),
            errors: ErrorStrategy::Throw,
            ..options
        },
    )
}

pub fn lossy_options() -> TsOptions {
    TsOptions {
        mode: EmitMode::Lossy,
        ..TsOptions::default()
    }
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

fn link(source: &Path, destination: &Path) {
    #[cfg(unix)]
    std::os::unix::fs::symlink(source, destination).unwrap();
    #[cfg(windows)]
    std::os::windows::fs::symlink_dir(source, destination).unwrap();
}

fn install_zod(directory: &Path) {
    let scope = directory.join("node_modules");
    fs::create_dir_all(&scope).unwrap();
    link(&super::package("zod"), &scope.join("zod"));
}

fn install_runtime_package(directory: &Path) {
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("runtime");
    let installed = directory.join("runtime");
    copy_runtime(&source, &installed);
    let scope = directory.join("node_modules").join("@pons");
    fs::create_dir_all(&scope).unwrap();
    link(&installed, &scope.join("runtime"));
}

fn copy_runtime(source: &Path, destination: &Path) {
    fs::create_dir_all(destination).unwrap();
    for entry in fs::read_dir(source).unwrap() {
        let entry = entry.unwrap();
        let target = destination.join(entry.file_name());
        if entry.path().is_dir() {
            copy_runtime(&entry.path(), &target);
        } else {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name.ends_with(".ts") || name == "package.json" {
                fs::copy(entry.path(), target).unwrap();
            }
        }
    }
}

pub fn run_zod(files: &[GeneratedFile], harness: &str) {
    run_zod_with_runtime(files, harness, Runtime::Vendor)
}

pub fn run_zod_with_runtime(files: &[GeneratedFile], harness: &str, runtime: Runtime) {
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
    if runtime == Runtime::Package {
        install_runtime_package(directory.path());
    }
    install_zod(directory.path());
    let harness_path = directory.path().join("schema.ts");
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
