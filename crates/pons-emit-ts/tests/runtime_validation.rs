use std::{collections::BTreeMap, fs, process::Command};

use pons_emit_ts::{
    EmitMode, ErrorStrategy, GeneratedFile, TsOptions, ValidationBackend, ValidationOptions,
    emit_with_options, emit_with_strategy,
};
use pons_ir::{
    ApiKeyLocation, ByteEncoding, ContentKind, CredentialLocator, Direction, EnumRepr, InlineType,
    IntegerWire, NamedRef, OpaqueWire, PathSegment, Presence, ResponseHeader, TypeKind, TypeRef,
    Variant, VariantPayload,
};

fn constrained_integer(wire: IntegerWire, bits: u16) -> TypeRef {
    TypeRef::Inline(InlineType::Int {
        bits,
        signed: false,
        wire,
        minimum: Some(serde_json::json!(10)),
        maximum: Some(serde_json::json!(250)),
        multiple_of: Some(serde_json::json!(5)),
    })
}

fn wide_number_document() -> pons_ir::Document {
    let mut document: pons_ir::Document =
        serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
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

fn opaque_document() -> pons_ir::Document {
    let mut document: pons_ir::Document =
        serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
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

fn flattened_opaque_document() -> pons_ir::Document {
    let mut document: pons_ir::Document =
        serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
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

fn flattened_named_opaque_document() -> pons_ir::Document {
    let mut raw: serde_json::Value =
        serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
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

fn nested_flattened_opaque_document() -> pons_ir::Document {
    let mut raw: serde_json::Value =
        serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
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

fn flattened_with_sibling_opaque_document() -> pons_ir::Document {
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

fn request_input_document(required: bool) -> pons_ir::Document {
    let mut document: pons_ir::Document =
        serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
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

fn untagged_bytes_document() -> pons_ir::Document {
    let mut document: pons_ir::Document =
        serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
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

fn validated_files(document: &pons_ir::Document) -> Vec<GeneratedFile> {
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

fn run_bun(files: &[GeneratedFile], harness: &str) {
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

#[test]
fn generated_client_rejects_missing_fields_and_unknown_enum_variants() {
    let document: pons_ir::Document =
        serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
    let files = emit_with_strategy(&document, EmitMode::Strict, ErrorStrategy::Throw)
        .unwrap()
        .files;
    let client = files.iter().find(|file| file.path == "client.ts").unwrap();
    assert!(client.content.contains("export function createClient"));
    let wire = files
        .iter()
        .find(|file| file.path == "runtime/wire.ts")
        .unwrap();
    assert!(wire.content.contains("function validateWire"));

    let harness = r#"
import { createClient } from "./client.ts";

const missingRequest = createClient({
  baseUrl: "http://example.test",
  fetch: async () => {
    throw new Error("request should not be sent");
  },
});
try {
  await missingRequest.auth.login({ body: {} });
  throw new Error("missing request was accepted");
} catch (cause) {
  const text = JSON.stringify(cause);
  if (!text.includes("missing required field email")) throw new Error(text);
}

const missing = createClient({
  baseUrl: "http://example.test",
  fetch: async () => new Response("{}", { status: 200, headers: { "content-type": "application/json" } }),
});
try {
  await missing.auth.login({ body: { email: "person@example.test" } });
  throw new Error("missing response was accepted");
} catch (cause) {
  const text = JSON.stringify(cause);
  if (!text.includes("missing required field id")) throw new Error(text);
}

const unknown = createClient({
  baseUrl: "http://example.test",
  fetch: async () => new Response(JSON.stringify({ code: "unknown" }), { status: 400, headers: { "content-type": "application/json" } }),
});
try {
  await unknown.auth.login({ body: { email: "person@example.test" } });
  throw new Error("unknown enum variant was accepted");
} catch (cause) {
  const text = JSON.stringify(cause);
  if (!text.includes("invalid enum variant unknown")) throw new Error(text);
}
"#;
    run_bun(&files, harness);
}

#[test]
fn generated_operation_maps_preserve_proto_keys() {
    let mut document: pons_ir::Document =
        serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
    document.operations[0].id = vec![vec!["__proto__".into()]];
    let files = emit_with_strategy(&document, EmitMode::Strict, ErrorStrategy::Throw)
        .unwrap()
        .files;
    let harness = r#"
import { createClient } from "./client.ts";

const client = createClient({
  baseUrl: "http://example.test",
  fetch: async () => new Response(JSON.stringify({ id: "1" }), {
    status: 200,
    headers: { "content-type": "application/json" },
  }),
});
const operation = (client as any)["__proto__"];
if (!Object.hasOwn(client, "__proto__") || typeof operation !== "function") {
  throw new Error("prototype operation was not preserved");
}
if (Object.getPrototypeOf(client) !== Object.prototype) throw new Error("client prototype was changed");
const value = await operation({ body: { email: "person@example.test" } });
if (value.id !== "1") throw new Error("prototype operation was not callable");
"#;
    run_bun(&files, harness);
}

#[test]
fn runtime_objects_preserve_proto_wire_fields() {
    let mut raw: serde_json::Value =
        serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
    raw["types"]["LoginRequest"]["openness"] = serde_json::json!("reject");
    raw["types"]["LoginRequest"]["fields"][0]["wire"] = serde_json::json!("__proto__");
    raw["types"]["LoginRequest"]["fields"][0]["ty"] = serde_json::json!({ "kind": "string" });
    raw["types"]["User"]["openness"] = serde_json::json!("reject");
    raw["types"]["User"]["fields"][0]["wire"] = serde_json::json!("__proto__");
    raw["types"]["User"]["fields"][0]["ty"] = serde_json::json!({ "kind": "string" });
    raw["operations"][0]["provides"] = serde_json::Value::Null;
    let document: pons_ir::Document = serde_json::from_value(raw).unwrap();
    let files = emit_with_strategy(&document, EmitMode::Strict, ErrorStrategy::Throw)
        .unwrap()
        .files;
    let harness = r#"
import { createClient } from "./client.ts";

const requestBody: any = Object.create(null);
requestBody["__proto__"] = "request";
const client = createClient({
  baseUrl: "http://example.test",
  fetch: async (_url, init) => {
    const body = JSON.parse(String(init?.body));
    if (!Object.hasOwn(body, "__proto__") || body["__proto__"] !== "request") {
      throw new Error("request prototype field was not preserved");
    }
    return new Response('{"__proto__":"response"}', {
      status: 200,
      headers: { "content-type": "application/json" },
    });
  },
});
const value: any = await client.auth.login({ body: requestBody });
if (!Object.hasOwn(value, "__proto__") || value["__proto__"] !== "response") {
  throw new Error("response prototype field was not preserved");
}
if (Object.getPrototypeOf(value) !== null) throw new Error("response prototype was changed");
"#;
    run_bun(&files, harness);
}

#[test]
fn runtime_objects_preserve_proto_enum_tags() {
    let mut raw: serde_json::Value =
        serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
    raw["types"]["Choice"] = serde_json::json!({
        "kind": "enum",
        "name": ["choice"],
        "direction": "output",
        "repr": { "kind": "internal", "tag": "__proto__" },
        "variants": [{ "wire": "kind", "ident": ["kind"], "payload": { "kind": "unit" } }]
    });
    raw["operations"][0]["responses"][0]["body"] = serde_json::json!({ "ref": "Choice" });
    raw["operations"][0]["provides"] = serde_json::Value::Null;
    let document: pons_ir::Document = serde_json::from_value(raw).unwrap();
    let files = emit_with_strategy(&document, EmitMode::Strict, ErrorStrategy::Throw)
        .unwrap()
        .files;
    let harness = r#"
import { createClient } from "./client.ts";

const client = createClient({
  baseUrl: "http://example.test",
  fetch: async () => new Response('{"__proto__":"kind"}', {
    status: 200,
    headers: { "content-type": "application/json" },
  }),
});
const value: any = await client.auth.login({ body: { email: "person@example.test" } });
if (!Object.hasOwn(value, "__proto__") || value["__proto__"] !== "kind") {
  throw new Error("prototype enum tag was not preserved");
}
if (Object.getPrototypeOf(value) !== null) throw new Error("enum prototype was changed");
"#;
    run_bun(&files, harness);
}

#[test]
fn opaque_values_require_a_runtime_codec_in_strict_mode() {
    let document = opaque_document();
    let strict = emit_with_strategy(&document, EmitMode::Strict, ErrorStrategy::Throw).unwrap();
    assert_eq!(
        strict.warnings,
        vec![
            "TypeScript does not understand the extension namespace test.example.source/v1, so the \
             emitted client ignores it"
                .to_owned()
        ],
        "the fixture carries one extension namespace the client cannot read"
    );
    let strict_client = strict
        .files
        .iter()
        .find(|file| file.path == "client.ts")
        .unwrap();
    assert!(
        strict_client
            .content
            .contains("codecs: OpaqueCodecRegistry")
    );
    assert!(strict_client.content.contains("required: true"));
    let strict_codec = strict
        .files
        .iter()
        .find(|file| file.path == "runtime/codec.ts")
        .unwrap();
    assert!(strict_codec.content.contains("transformOpaque"));

    let output = emit_with_strategy(&document, EmitMode::Lossy, ErrorStrategy::Throw).unwrap();
    assert!(
        output
            .warnings
            .iter()
            .any(|warning| warning.contains("opaque values"))
    );
    let types = output
        .files
        .iter()
        .find(|file| file.path == "types.ts")
        .unwrap();
    assert!(types.content.contains("unknown"));
    let client = output
        .files
        .iter()
        .find(|file| file.path == "client.ts")
        .unwrap();
    assert!(client.content.contains("codecs?: OpaqueCodecRegistry"));
    let codec = output
        .files
        .iter()
        .find(|file| file.path == "runtime/codec.ts")
        .unwrap();
    assert!(codec.content.contains("schema.kind === \"opaque\""));
}

#[test]
fn orphan_opaque_type_does_not_require_a_runtime_codec_in_strict_mode() {
    let mut raw: serde_json::Value =
        serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
    raw["types"]["OrphanOpaque"] = serde_json::json!({
        "kind": "newtype",
        "name": ["orphan", "opaque"],
        "direction": "both",
        "inner": { "kind": "opaque", "codec": "example.codec", "wire": "json" }
    });
    let document: pons_ir::Document = serde_json::from_value(raw).unwrap();

    let output = emit_with_strategy(&document, EmitMode::Strict, ErrorStrategy::Throw).unwrap();
    assert!(output.coverage.opaque.contains("types.OrphanOpaque"));
    let client = output
        .files
        .iter()
        .find(|file| file.path == "client.ts")
        .unwrap();
    assert!(client.content.contains("codecs?: OpaqueCodecRegistry"));
    assert!(client.content.contains("required: false"));
}

#[test]
fn unowned_common_error_opaque_does_not_require_a_runtime_codec_in_strict_mode() {
    let mut raw: serde_json::Value =
        serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
    raw["common_errors"] = serde_json::json!([{
        "status": "5xx",
        "name": ["common", "error"],
        "content": "json",
        "body": { "kind": "opaque", "codec": "example.codec", "wire": "json" }
    }]);
    for operation in raw["operations"].as_array_mut().unwrap() {
        operation["errors_inherit"] = serde_json::json!(false);
    }
    let document: pons_ir::Document = serde_json::from_value(raw.clone()).unwrap();

    let output = emit_with_strategy(&document, EmitMode::Strict, ErrorStrategy::Throw).unwrap();
    let client = output
        .files
        .iter()
        .find(|file| file.path == "client.ts")
        .unwrap();
    assert!(client.content.contains("codecs?: OpaqueCodecRegistry"));
    assert!(client.content.contains("required: false"));

    let mut inherited = raw;
    inherited["operations"][0]["errors_inherit"] = serde_json::json!(true);
    let document: pons_ir::Document = serde_json::from_value(inherited).unwrap();
    let output = emit_with_strategy(&document, EmitMode::Strict, ErrorStrategy::Throw).unwrap();
    let client = output
        .files
        .iter()
        .find(|file| file.path == "client.ts")
        .unwrap();
    assert!(client.content.contains("codecs: OpaqueCodecRegistry"));
    assert!(client.content.contains("required: true"));
}

#[test]
fn runtime_codecs_encode_requests_and_decode_responses() {
    let document = opaque_document();
    let files = emit_with_strategy(&document, EmitMode::Strict, ErrorStrategy::Throw)
        .unwrap()
        .files;
    let harness = r#"
import { createClient } from "./client.ts";

const calls: string[] = [];
const codec = {
  encode(value: unknown, wire: string) {
    calls.push(`encode:${wire}`);
    return `encoded:${String(value)}`;
  },
  decode(value: unknown, wire: string) {
    calls.push(`decode:${wire}`);
    return `decoded:${String(value)}`;
  },
};
const client = createClient({
  baseUrl: "http://example.test",
  codecs: { "example.codec": codec },
  fetch: async (_url, init) => {
    const body = JSON.parse(String(init?.body));
    if (body.email !== "encoded:person@example.test") throw new Error("request was not encoded");
    return new Response(JSON.stringify({ id: "server" }), {
      status: 200,
      headers: { "content-type": "application/json" },
    });
  },
});
const result = await client.auth.login({ body: { email: "person@example.test" } });
if (result.id !== "decoded:server") throw new Error("response was not decoded");
if (calls.join(",") !== "encode:json,decode:json") throw new Error(calls.join(","));

let fetched = false;
const missing = createClient({
  baseUrl: "http://example.test",
  fetch: async () => {
    fetched = true;
    throw new Error("fetch should not run");
  },
});
let failure: any;
try {
  await missing.auth.login({ body: { email: "person@example.test" } });
} catch (cause) {
  failure = cause;
}
if (failure?._tag !== "OpaqueCodecError") throw new Error(`unexpected failure ${JSON.stringify(failure)}`);
if (fetched) throw new Error("missing codec reached fetch");
"#;
    run_bun(&files, harness);
}

#[test]
fn runtime_codecs_accept_only_own_proto_codecs() {
    let mut document = opaque_document();
    for id in ["LoginRequest", "User"] {
        let definition = document.types.get_mut(id).unwrap();
        let TypeKind::Struct { fields, .. } = &mut definition.definition else {
            unreachable!()
        };
        fields[0].ty = TypeRef::Inline(InlineType::Opaque {
            codec: "__proto__".into(),
            wire: OpaqueWire::Json,
            ext: Default::default(),
        });
    }
    let files = emit_with_strategy(&document, EmitMode::Strict, ErrorStrategy::Throw)
        .unwrap()
        .files;
    let harness = r#"
import { createClient } from "./client.ts";

const calls: string[] = [];
const codec = {
  encode(value: unknown, wire: string) {
    calls.push(`encode:${wire}`);
    return `encoded:${String(value)}`;
  },
  decode(value: unknown, wire: string) {
    calls.push(`decode:${wire}`);
    return `decoded:${String(value)}`;
  },
};
const codecs: Record<string, typeof codec> = {};
Object.defineProperty(codecs, "__proto__", { value: codec, enumerable: true });
if (!Object.hasOwn(codecs, "__proto__")) throw new Error("test registry is not an own map");

const client = createClient({
  baseUrl: "http://example.test",
  codecs,
  fetch: async (_url, init) => {
    const body = JSON.parse(String(init?.body));
    if (body.email !== "encoded:person@example.test") throw new Error("prototype codec was not used to encode");
    return new Response(JSON.stringify({ id: "server" }), {
      status: 200,
      headers: { "content-type": "application/json" },
    });
  },
});
const value = await client.auth.login({ body: { email: "person@example.test" } });
if (value.id !== "decoded:server") throw new Error("prototype codec was not used to decode");
if (calls.join(",") !== "encode:json,decode:json") throw new Error(calls.join(","));

const poisonedPrototype = {};
Object.defineProperty(poisonedPrototype, "__proto__", {
  get() { throw new Error("inherited prototype codec was read"); },
});
let fetched = false;
const missing = createClient({
  baseUrl: "http://example.test",
  codecs: Object.create(poisonedPrototype),
  fetch: async () => {
    fetched = true;
    throw new Error("request should not be sent");
  },
});
let failure: any;
try {
  await missing.auth.login({ body: { email: "person@example.test" } });
} catch (cause) {
  failure = cause;
}
if (failure?._tag !== "OpaqueCodecError") throw new Error(`unexpected failure ${JSON.stringify(failure)}`);
if (fetched) throw new Error("missing codec reached fetch");
"#;
    run_bun(&files, harness);
}

#[test]
fn runtime_codecs_handle_flattened_opaque_values() {
    let document = flattened_opaque_document();
    let files = emit_with_strategy(&document, EmitMode::Strict, ErrorStrategy::Throw)
        .unwrap()
        .files;
    let harness = r#"
import { createClient } from "./client.ts";

const calls: string[] = [];
const client = createClient({
  baseUrl: "http://example.test",
  codecs: {
    "flatten.codec": {
      encode(value: unknown, wire: string) {
        calls.push(`encode:${wire}`);
        return { ...(value as any), email: "encoded" };
      },
      decode(value: unknown, wire: string) {
        calls.push(`decode:${wire}`);
        if ((value as any).id !== "1") throw new Error("wrong decode input");
        return { id: "decoded", nickname: null };
      },
    },
  },
  fetch: async (_url, init) => {
    const body = JSON.parse(String(init?.body));
    if (body.email !== "encoded") throw new Error("flattened request was not encoded");
    return new Response(JSON.stringify({ id: "1" }), {
      status: 200,
      headers: { "content-type": "application/json" },
    });
  },
});
const value = await client.auth.login({ body: { email: "person@example.test" } });
if (value.id !== "decoded") throw new Error("flattened response was not decoded");
if (calls.join(",") !== "encode:json,decode:json") throw new Error(calls.join(","));

let fetched = false;
const missing = createClient({
  baseUrl: "http://example.test",
  fetch: async () => {
    fetched = true;
    return new Response("{}", { status: 200 });
  },
});
let failure: any;
try {
  await missing.auth.login({ body: { email: "person@example.test" } });
} catch (cause) {
  failure = cause;
}
if (failure?._tag !== "OpaqueCodecError") throw new Error(`unexpected failure ${JSON.stringify(failure)}`);
if (fetched) throw new Error("missing flattened codec reached fetch");
"#;
    run_bun(&files, harness);
}

#[test]
fn runtime_codecs_handle_named_flattened_opaque_aliases() {
    let mut document = flattened_named_opaque_document();
    document.operations[0].provides = None;
    let files = emit_with_strategy(&document, EmitMode::Strict, ErrorStrategy::Throw)
        .unwrap()
        .files;
    let harness = r#"
import { createClient } from "./client.ts";

const calls: string[] = [];
const client = createClient({
  baseUrl: "http://example.test",
  codecs: {
    "flatten.codec": {
      encode(value: unknown, wire: string) {
        calls.push(`encode:${wire}`);
        return { ...(value as any), email: "encoded" };
      },
      decode(value: unknown, wire: string) {
        calls.push(`decode:${wire}`);
        return { ...(value as any), id: "decoded" };
      },
    },
  },
  fetch: async (_url, init) => {
    const body = JSON.parse(String(init?.body));
    if (body.email !== "encoded") throw new Error("named flattened alias was not encoded");
    return new Response(JSON.stringify({ id: "wire", nickname: null }), {
      status: 200,
      headers: { "content-type": "application/json" },
    });
  },
});
const value = await client.auth.login({ body: { email: "person@example.test" } });
if (value.id !== "decoded") throw new Error("named flattened alias was not decoded");
if (calls.join(",") !== "encode:json,decode:json") throw new Error(calls.join(","));

let fetched = false;
const missing = createClient({
  baseUrl: "http://example.test",
  fetch: async () => {
    fetched = true;
    return new Response("{}", { status: 200 });
  },
});
let failure: any;
try {
  await missing.auth.login({ body: { email: "person@example.test" } });
} catch (cause) {
  failure = cause;
}
if (failure?._tag !== "OpaqueCodecError") throw new Error(`unexpected failure ${JSON.stringify(failure)}`);
if (fetched) throw new Error("missing named flattened codec reached fetch");
"#;
    run_bun(&files, harness);
}

#[test]
fn runtime_codecs_handle_nested_flattened_opaque_structs() {
    let document = nested_flattened_opaque_document();
    let files = emit_with_strategy(&document, EmitMode::Strict, ErrorStrategy::Throw)
        .unwrap()
        .files;
    let harness = r#"
import { createClient } from "./client.ts";

const calls: string[] = [];
const client = createClient({
  baseUrl: "http://example.test",
  codecs: {
    "flatten.codec": {
      encode(value: unknown, wire: string) {
        calls.push(`encode:${wire}`);
        return { ...(value as any), email: "encoded" };
      },
      decode(value: unknown, wire: string) {
        calls.push(`decode:${wire}`);
        if ((value as any).tag !== "envelope") throw new Error("nested carrier lost its fields");
        return { ...(value as any), id: "decoded" };
      },
    },
  },
  fetch: async (_url, init) => {
    const body = JSON.parse(String(init?.body));
    if (body.email !== "encoded") throw new Error("nested flattened request was not encoded");
    if (body.tag !== "envelope") throw new Error("nested flattened field was not collected");
    return new Response(JSON.stringify({ id: "wire", nickname: null, tag: "envelope" }), {
      status: 200,
      headers: { "content-type": "application/json" },
    });
  },
});
const value = await client.auth.login({ body: { email: "person@example.test", tag: "envelope" } });
if (value.id !== "decoded") throw new Error("nested flattened response was not decoded");
if (calls.join(",") !== "encode:json,decode:json") throw new Error(calls.join(","));

let fetched = false;
const missing = createClient({
  baseUrl: "http://example.test",
  fetch: async () => {
    fetched = true;
    return new Response("{}", { status: 200 });
  },
});
let failure: any;
try {
  await missing.auth.login({ body: { email: "person@example.test", tag: "envelope" } });
} catch (cause) {
  failure = cause;
}
if (failure?._tag !== "OpaqueCodecError") throw new Error(`unexpected failure ${JSON.stringify(failure)}`);
if (fetched) throw new Error("missing nested flattened codec reached fetch");
"#;
    run_bun(&files, harness);
}

#[test]
fn runtime_codecs_handle_flattened_and_sibling_opaque_values() {
    let document = flattened_with_sibling_opaque_document();
    let files = emit_with_strategy(&document, EmitMode::Strict, ErrorStrategy::Throw)
        .unwrap()
        .files;
    let harness = r#"
import { createClient } from "./client.ts";

const calls: string[] = [];
const client = createClient({
  baseUrl: "http://example.test",
  codecs: {
    "flatten.codec": {
      encode(value: unknown, wire: string) {
        calls.push(`flatten:encode:${wire}`);
        return { ...(value as any) };
      },
      decode(value: unknown, wire: string) {
        calls.push(`flatten:decode:${wire}`);
        return value;
      },
    },
    "sibling.codec": {
      encode(value: unknown, wire: string) {
        calls.push(`sibling:encode:${wire}`);
        return `encoded:${String(value)}`;
      },
      decode(value: unknown, wire: string) {
        calls.push(`sibling:decode:${wire}`);
        return `decoded:${String(value)}`;
      },
    },
  },
  fetch: async (_url, init) => {
    const body = JSON.parse(String(init?.body));
    if (body.email !== "encoded:request") throw new Error("sibling request codec was skipped");
    return new Response(JSON.stringify({ id: "server" }), {
      status: 200,
      headers: { "content-type": "application/json" },
    });
  },
});
const value = await client.auth.login({ body: { email: "request" } });
if (value.id !== "decoded:server") throw new Error("sibling response codec was skipped");
if (calls.join(",") !== "flatten:encode:json,sibling:encode:json,flatten:decode:json,sibling:decode:json") {
  throw new Error(`unexpected codec calls: ${calls.join(",")}`);
}

let fetched = false;
const missing = createClient({
  baseUrl: "http://example.test",
  codecs: {
    "flatten.codec": {
      encode: (value: unknown) => value,
      decode: (value: unknown) => value,
    },
  },
  fetch: async () => {
    fetched = true;
    return new Response("{}", { status: 200 });
  },
});
let failure: any;
try {
  await missing.auth.login({ body: { email: "request" } });
} catch (cause) {
  failure = cause;
}
if (failure?._tag !== "OpaqueCodecError") throw new Error(`unexpected failure ${JSON.stringify(failure)}`);
if (fetched) throw new Error("missing sibling codec reached fetch");
"#;
    run_bun(&files, harness);
}

#[test]
fn captures_body_credentials_before_opaque_decoding() {
    let mut document: pons_ir::Document =
        serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
    let user = document.types.get_mut("User").unwrap();
    let TypeKind::Struct { fields, .. } = &mut user.definition else {
        unreachable!()
    };
    fields[0].ty = TypeRef::Inline(InlineType::Opaque {
        codec: "auth.codec".into(),
        wire: OpaqueWire::Json,
        ext: Default::default(),
    });
    document.operations[0].provides.as_mut().unwrap().value = CredentialLocator::Body {
        pointer: "/id".into(),
    };
    document.operations[1].auth.alternatives = vec![vec!["session".into()]];

    let files = emit_with_strategy(&document, EmitMode::Strict, ErrorStrategy::Throw)
        .unwrap()
        .files;
    let harness = r#"
import { createClient } from "./client.ts";

const sent: string[] = [];
let calls = 0;
const client = createClient({
  baseUrl: "http://example.test",
  codecs: {
    "auth.codec": {
      encode: (value: unknown) => value,
      decode: (value: unknown) => `decoded:${String(value)}`,
    },
  },
  fetch: async (_url, init) => {
    if (calls++ === 0) {
      return new Response(JSON.stringify({ id: "wire-secret" }), {
        status: 200,
        headers: { "content-type": "application/json" },
      });
    }
    sent.push(new Headers(init?.headers).get("authorization") ?? "");
    return new Response(new ReadableStream({ start(controller) { controller.close(); } }), {
      status: 200,
      headers: {
        "content-type": "text/event-stream",
        "X-Trace-ID": "trace",
      },
    });
  },
});
const login = await client.auth.login({ body: { email: "person@example.test" } });
if (login.id !== "decoded:wire-secret") throw new Error("opaque response was not decoded");
await client.events.watch({ path: { cursor: "cursor" }, query: {}, headers: {} });
if (sent[0] !== "Bearer wire-secret") throw new Error(`decoded credential was replayed: ${sent[0]}`);
"#;
    run_bun(&files, harness);
}

#[test]
fn captures_body_credentials_when_opaque_decoding_fails() {
    let mut document: pons_ir::Document =
        serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
    let user = document.types.get_mut("User").unwrap();
    let TypeKind::Struct { fields, .. } = &mut user.definition else {
        unreachable!()
    };
    fields[0].ty = TypeRef::Inline(InlineType::Opaque {
        codec: "missing.codec".into(),
        wire: OpaqueWire::Json,
        ext: Default::default(),
    });
    document.operations[0].provides.as_mut().unwrap().value = CredentialLocator::Body {
        pointer: "/id".into(),
    };
    document.operations[1].auth.alternatives = vec![vec!["session".into()]];

    let files = emit_with_strategy(&document, EmitMode::Strict, ErrorStrategy::Throw)
        .unwrap()
        .files;
    let harness = r#"
import { createClient } from "./client.ts";

let calls = 0;
let sent = "";
const client = createClient({
  baseUrl: "https://example.test",
  fetch: async (_url, init) => {
    calls += 1;
    if (calls === 1) {
      return new Response(JSON.stringify({ id: "wire-secret" }), {
        status: 200,
        headers: { "content-type": "application/json" },
      });
    }
    sent = new Headers(init?.headers).get("authorization") ?? "";
    return new Response(new ReadableStream({ start(controller) { controller.close(); } }), {
      status: 200,
      headers: {
        "content-type": "text/event-stream",
        "X-Trace-ID": "trace",
      },
    });
  },
});
let failure: any;
try {
  await client.auth.login({ body: { email: "person@example.test" } });
} catch (cause) {
  failure = cause;
}
if (failure?._tag !== "OpaqueCodecError") throw new Error(`unexpected failure ${JSON.stringify(failure)}`);
await client.events.watch({ path: { cursor: "cursor" }, query: {}, headers: {} });
if (sent !== "Bearer wire-secret") throw new Error(`credential was not captured: ${sent}`);
"#;
    run_bun(&files, harness);
}

#[test]
fn decodes_opaque_internal_newtype_alias_variants() {
    let mut raw: serde_json::Value =
        serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
    raw["types"]["User"]["fields"][0]["ty"] = serde_json::json!({
        "kind": "opaque",
        "codec": "example.codec",
        "wire": "json"
    });
    raw["types"]["UserAlias"] = serde_json::json!({
        "kind": "newtype",
        "name": ["user", "alias"],
        "direction": "both",
        "inner": { "ref": "User" }
    });
    raw["types"]["LoginError"]["variants"][1]["payload"] = serde_json::json!({
        "kind": "newtype",
        "ty": { "ref": "UserAlias" }
    });
    raw["operations"][0]["responses"][0]["body"] = serde_json::json!({
        "ref": "LoginError",
        "variant": "locked"
    });
    raw["operations"][0]["provides"] = serde_json::Value::Null;
    let document: pons_ir::Document = serde_json::from_value(raw).unwrap();
    let files = emit_with_strategy(&document, EmitMode::Strict, ErrorStrategy::Throw)
        .unwrap()
        .files;
    let harness = r#"
import { createClient } from "./client.ts";

const client = createClient({
  baseUrl: "https://example.test",
  codecs: {
    "example.codec": {
      encode: (value: unknown) => value,
      decode: (value: unknown) => `decoded:${String(value)}`,
    },
  },
  fetch: async () => new Response(JSON.stringify({ code: "locked", id: "wire", nickname: null }), {
    status: 200,
    headers: { "content-type": "application/json" },
  }),
});
const value = await client.auth.login({ body: { email: "person@example.test" } });
if (value.code !== "locked" || value.id !== "decoded:wire") throw new Error(`wrong decoded value: ${JSON.stringify(value)}`);
"#;
    run_bun(&files, harness);
}

#[test]
fn rejects_fractional_integer_constraints() {
    let mut document: pons_ir::Document =
        serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
    let user = document.types.get_mut("User").unwrap();
    let TypeKind::Struct { fields, .. } = &mut user.definition else {
        unreachable!()
    };
    fields[0].ty = TypeRef::Inline(InlineType::Int {
        bits: 8,
        signed: false,
        wire: IntegerWire::Number,
        minimum: Some(serde_json::json!(0)),
        maximum: Some(serde_json::json!(255)),
        multiple_of: Some(serde_json::json!(0.5)),
    });
    let error = emit_with_strategy(&document, EmitMode::Strict, ErrorStrategy::Throw).unwrap_err();
    assert!(
        error
            .0
            .contains("integer multiple_of must be an integral value")
    );
}

#[test]
fn refuses_enum_variants_with_unrepresentable_flatten() {
    let mut raw: serde_json::Value =
        serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
    raw["types"]["LoginError"]["variants"][1]["payload"]["fields"][0]["diagnostics"] = serde_json::json!([{
        "severity": "error",
        "code": "PONS-E101",
        "message": "custom serde flatten in an enum variant payload has no wire key"
    }]);
    let document: pons_ir::Document = serde_json::from_value(raw).unwrap();
    let error = emit_with_strategy(&document, EmitMode::Strict, ErrorStrategy::Throw).unwrap_err();
    assert!(error.0.contains("PONS-E101"));
}

#[test]
fn generated_client_enforces_integer_constraints() {
    let mut document: pons_ir::Document =
        serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
    let user = document.types.get_mut("User").unwrap();
    let TypeKind::Struct { fields, .. } = &mut user.definition else {
        unreachable!()
    };
    fields[0].ty = constrained_integer(IntegerWire::Number, 8);

    let login = document.types.get_mut("LoginRequest").unwrap();
    let TypeKind::Struct { fields, .. } = &mut login.definition else {
        unreachable!()
    };
    fields[0].ty = constrained_integer(IntegerWire::Number, 8);

    let response = &mut document.operations[0].responses[0];
    response.headers.push(ResponseHeader {
        wire: "X-Count".into(),
        ty: constrained_integer(IntegerWire::Number, 8),
        presence: Presence::Required,
        ext: BTreeMap::new(),
    });
    response.headers.push(ResponseHeader {
        wire: "X-Text-Count".into(),
        ty: constrained_integer(IntegerWire::String, 8),
        presence: Presence::Required,
        ext: BTreeMap::new(),
    });
    response.headers.push(ResponseHeader {
        wire: "X-Wide".into(),
        ty: TypeRef::Inline(InlineType::Int {
            bits: 64,
            signed: false,
            wire: IntegerWire::String,
            minimum: Some(serde_json::json!(u64::MAX)),
            maximum: Some(serde_json::json!(u64::MAX)),
            multiple_of: Some(serde_json::json!(1)),
        }),
        presence: Presence::Required,
        ext: BTreeMap::new(),
    });
    response.headers.push(ResponseHeader {
        wire: "X-Signed".into(),
        ty: TypeRef::Inline(InlineType::Int {
            bits: 8,
            signed: true,
            wire: IntegerWire::Number,
            minimum: Some(serde_json::json!(-128)),
            maximum: Some(serde_json::json!(127)),
            multiple_of: Some(serde_json::json!(1)),
        }),
        presence: Presence::Required,
        ext: BTreeMap::new(),
    });

    let files = emit_with_strategy(&document, EmitMode::Strict, ErrorStrategy::Throw)
        .unwrap()
        .files;
    let wire = files
        .iter()
        .find(|file| file.path == "runtime/wire.ts")
        .unwrap();
    assert!(wire.content.contains("function validateInteger"));

    let harness = r#"
import { createClient } from "./client.ts";

const makeResponse = (id, count = "250", textCount = "250", wide = "18446744073709551615", signed = "-128") =>
  new Response(JSON.stringify({ id }), {
    status: 200,
    headers: {
      "content-type": "application/json",
      "X-Count": count,
      "X-Text-Count": textCount,
      "X-Wide": wide,
      "X-Signed": signed,
    },
  });
const expectFailure = async (operation, expected) => {
  try {
    await operation();
    throw new Error("operation was accepted");
  } catch (cause) {
    const text = JSON.stringify(cause);
    if (!text.includes(expected)) throw new Error(text);
  }
};

let sent = 0;
const outbound = createClient({
  baseUrl: "http://example.test",
  fetch: async () => {
    sent += 1;
    return makeResponse(250);
  },
});
await expectFailure(() => outbound.auth.login({ body: { email: 249 } }), "does not match its multiple");
if (sent !== 0) throw new Error("invalid outbound integer was sent");
const valid = await outbound.auth.login({ body: { email: 250 } });
if (valid.value.id !== 250 || valid.headers["X-Wide"] !== "18446744073709551615" || valid.headers["X-Signed"] !== -128) {
  throw new Error("valid integer response was changed");
}

const body = createClient({
  baseUrl: "http://example.test",
  fetch: async () => makeResponse(249),
});
await expectFailure(() => body.auth.login({ body: { email: 250 } }), "does not match its multiple");

const overflow = createClient({
  baseUrl: "http://example.test",
  fetch: async () => makeResponse(256),
});
await expectFailure(() => overflow.auth.login({ body: { email: 250 } }), "outside its native range");

const negative = createClient({
  baseUrl: "http://example.test",
  fetch: async () => makeResponse(-1),
});
await expectFailure(() => negative.auth.login({ body: { email: 250 } }), "outside its native range");

const belowMinimum = createClient({
  baseUrl: "http://example.test",
  fetch: async () => makeResponse(5),
});
await expectFailure(() => belowMinimum.auth.login({ body: { email: 250 } }), "below its minimum");

const header = createClient({
  baseUrl: "http://example.test",
  fetch: async () => makeResponse(250, "249"),
});
await expectFailure(() => header.auth.login({ body: { email: 250 } }), "invalid response header value");

const textHeader = createClient({
  baseUrl: "http://example.test",
  fetch: async () => makeResponse(250, "255", "256"),
});
await expectFailure(() => textHeader.auth.login({ body: { email: 250 } }), "invalid response header value");

const wideHeader = createClient({
  baseUrl: "http://example.test",
  fetch: async () => makeResponse(250, "255", "255", "18446744073709551616"),
});
await expectFailure(() => wideHeader.auth.login({ body: { email: 250 } }), "invalid response header value");

const signedHeader = createClient({
  baseUrl: "http://example.test",
  fetch: async () => makeResponse(250, "255", "255", "18446744073709551615", "128"),
});
await expectFailure(() => signedHeader.auth.login({ body: { email: 250 } }), "invalid response header value");
"#;
    run_bun(&files, harness);
}

#[test]
fn preserves_wide_integer_values_without_touching_strings() {
    let document = wide_number_document();
    let files = emit_with_strategy(&document, EmitMode::Strict, ErrorStrategy::Throw)
        .unwrap()
        .files;
    let harness = r#"
import { createClient } from "./client.ts";

const sentinel = "\u0000pons-bigint:";
const body = `{"id":9007199254740993,"nickname":${JSON.stringify(sentinel + "123")},${JSON.stringify(sentinel + "text")}:"key","quoted":"9007199254740993"}`;
const client = createClient({
  baseUrl: "http://example.test",
  fetch: async () => new Response(body, { status: 200, headers: { "content-type": "application/json" } }),
});
const value = await client.auth.login({ body: { email: "person@example.test" } });
if (value.id !== 9007199254740993n) throw new Error("unsafe number was not preserved");
if (value.nickname !== sentinel + "123") throw new Error("sentinel string was changed");
if (value[sentinel + "text"] !== "key") throw new Error("sentinel key was changed");
if (value.quoted !== "9007199254740993") throw new Error("quoted number was changed");
"#;
    run_bun(&files, harness);
}

#[test]
fn refreshes_expired_cached_credentials() {
    let mut document: pons_ir::Document =
        serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
    let operation = &mut document.operations[0];
    operation.auth.alternatives = vec![vec!["session".into()]];
    let provides = operation.provides.as_mut().unwrap();
    provides.value = CredentialLocator::Header {
        name: "X-Token".into(),
    };
    provides.expires_at = Some(CredentialLocator::Header {
        name: "X-Expires".into(),
    });

    let files = emit_with_strategy(&document, EmitMode::Strict, ErrorStrategy::Throw)
        .unwrap()
        .files;
    let harness = r#"
import { createClient } from "./client.ts";

const sent: string[] = [];
let gets = 0;
let clears = 0;
const store = {
  get: async () => {
    gets += 1;
     return { value: gets === 1 ? "initial" : "fresh" };
  },
  set: async () => {},
  clear: async () => {
    clears += 1;
  },
};
const client = createClient({
  baseUrl: "http://example.test",
  auth: { session: store },
  fetch: async (_url, init) => {
    sent.push(init.headers.get("authorization") ?? "");
    return new Response(JSON.stringify({ id: "1" }), {
      status: 200,
      headers: {
        "content-type": "application/json",
        "X-Token": "expired",
        "X-Expires": "2000-01-01T00:00:00Z",
      },
    });
  },
});
await client.auth.login({ body: { email: "person@example.test" } });
await client.auth.login({ body: { email: "person@example.test" } });
if (sent[0] !== "Bearer initial" || sent[1] !== "Bearer fresh") throw new Error(`unexpected credentials: ${sent.join(",")}`);
if (gets !== 2 || clears !== 1) throw new Error(`unexpected source calls: ${gets},${clears}`);
"#;
    run_bun(&files, harness);
}

#[test]
fn replays_captured_cookie_session_credentials() {
    let mut document: pons_ir::Document =
        serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
    document.auth_schemes.insert(
        "session".into(),
        pons_ir::AuthScheme::CookieSession {
            cookie: "session".into(),
        },
    );
    document.operations[0].provides.as_mut().unwrap().value = CredentialLocator::SetCookie {
        name: "session".into(),
    };
    document.operations[1].auth.alternatives = vec![vec!["session".into()]];

    let files = emit_with_strategy(&document, EmitMode::Strict, ErrorStrategy::Throw)
        .unwrap()
        .files;
    let harness = r#"
import { createClient } from "./client.ts";

const cookies: Array<string | null> = [];
const credentials: Array<RequestCredentials | undefined> = [];
const client = createClient({
  baseUrl: "http://example.test",
  headers: { cookie: "csrf=keep; session=old" },
  fetch: async (_url, init) => {
    cookies.push(new Headers(init?.headers).get("cookie"));
    credentials.push(init?.credentials);
    if (cookies.length === 1) {
      return new Response(JSON.stringify({ id: "1" }), {
        status: 200,
        headers: {
          "content-type": "application/json",
          "set-cookie": "session=captured; Path=/",
        },
      });
    }
    return new Response(new ReadableStream({ start(controller) { controller.close(); } }), {
      status: 200,
      headers: {
        "content-type": "text/event-stream",
        "X-Trace-ID": "trace",
      },
    });
  },
});
await client.auth.login({ body: { email: "person@example.test" } });
await client.events.watch({ path: { cursor: "cursor" }, query: {}, headers: {} });
if (cookies[1] !== "csrf=keep; session=captured") throw new Error(`cookie was not replayed: ${cookies[1]}`);
    if (credentials[1] !== "include") throw new Error(`credentials were not included: ${credentials[1]}`);
    "#;
    run_bun(&files, harness);
}

#[test]
fn uses_the_last_matching_set_cookie() {
    let mut document: pons_ir::Document =
        serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
    document.auth_schemes.insert(
        "session".into(),
        pons_ir::AuthScheme::CookieSession {
            cookie: "session".into(),
        },
    );
    document.operations[0].provides.as_mut().unwrap().value = CredentialLocator::SetCookie {
        name: "session".into(),
    };
    document.operations[1].auth.alternatives = vec![vec!["session".into()]];

    let files = emit_with_strategy(&document, EmitMode::Strict, ErrorStrategy::Throw)
        .unwrap()
        .files;
    let harness = r#"
import { createClient } from "./client.ts";

let calls = 0;
const client = createClient({
  baseUrl: "http://example.test",
  fetch: async (_url, init) => {
    calls += 1;
    if (calls === 1) {
      return new Response(JSON.stringify({ id: "1" }), {
        status: 200,
        headers: [
          ["content-type", "application/json"],
          ["set-cookie", "session=obsolete; Path=/; Max-Age=0"],
          ["set-cookie", "session=current; Path=/; Max-Age=60"],
        ],
      });
    }
    const headers = new Headers(init?.headers);
    if (headers.get("cookie") !== "session=current") throw new Error(`wrong cookie: ${headers.get("cookie")}`);
    return new Response(new ReadableStream({ start(controller) { controller.close(); } }), {
      status: 200,
      headers: {
        "content-type": "text/event-stream",
        "X-Trace-ID": "trace",
      },
    });
  },
});
await client.auth.login({ body: { email: "person@example.test" } });
await client.events.watch({ path: { cursor: "cursor" }, query: {}, headers: {} });
"#;
    run_bun(&files, harness);
}

#[test]
fn persists_cookie_metadata_across_clients() {
    let mut document: pons_ir::Document =
        serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
    document.auth_schemes.insert(
        "session".into(),
        pons_ir::AuthScheme::CookieSession {
            cookie: "session".into(),
        },
    );
    document.operations[0].provides.as_mut().unwrap().value = CredentialLocator::SetCookie {
        name: "session".into(),
    };
    document.operations[1].auth.alternatives = vec![vec!["session".into()]];

    let files = emit_with_strategy(&document, EmitMode::Strict, ErrorStrategy::Throw)
        .unwrap()
        .files;
    let harness = r#"
import { createClient } from "./client.ts";

let stored: any;
let protectedFetches = 0;
const store = {
  get: async () => stored,
  set: async (value: string, metadata: any) => {
    stored = { value, ...metadata };
  },
  clear: async () => {
    stored = undefined;
  },
};
const first = createClient({
  baseUrl: "https://example.test/tenant-a",
  auth: { session: store },
  fetch: async () => new Response(JSON.stringify({ id: "1" }), {
    status: 200,
    headers: {
      "content-type": "application/json",
      "set-cookie": "session=secret; Path=/auth; Secure",
    },
  }),
});
await first.auth.login({ body: { email: "person@example.test" } });
if (stored?.path !== "/auth" || stored?.secure !== true || stored?.host !== "example.test") {
  throw new Error(`cookie metadata was not persisted: ${JSON.stringify(stored)}`);
}
const expectAuthFailure = async (baseUrl: string) => {
  const client = createClient({
    baseUrl,
    auth: { session: store },
    fetch: async () => {
      protectedFetches += 1;
      throw new Error("out-of-scope cookie reached fetch");
    },
  });
  let failure: any;
  try {
    await client.events.watch({ path: { cursor: "cursor" }, query: {}, headers: {} });
  } catch (cause) {
    failure = cause;
  }
  if (failure?._tag !== "AuthUnavailableError") throw new Error(`unexpected failure ${JSON.stringify(failure)}`);
};
await expectAuthFailure("https://example.test");
await expectAuthFailure("https://other.test");
if (protectedFetches !== 0) throw new Error("out-of-scope cookie reached fetch");
"#;
    run_bun(&files, harness);
}

#[test]
fn scopes_captured_non_cookie_credentials_by_origin() {
    let mut document: pons_ir::Document =
        serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
    document.operations[0].provides.as_mut().unwrap().value = CredentialLocator::Header {
        name: "X-Token".into(),
    };
    document.operations[1].auth.alternatives = vec![vec!["session".into()]];
    let files = emit_with_strategy(&document, EmitMode::Strict, ErrorStrategy::Throw)
        .unwrap()
        .files;
    let harness = r#"
import { createClient } from "./client.ts";

let stored: any;
let fetched = false;
const store = {
  get: async () => stored,
  set: async (value: string, metadata: any) => { stored = { value, ...metadata }; },
  clear: async () => { stored = undefined; },
};
const first = createClient({
  baseUrl: "https://example.test:8443",
  auth: { session: store },
  fetch: async () => new Response(JSON.stringify({ id: "1" }), {
    status: 200,
    headers: { "content-type": "application/json", "X-Token": "secret" },
  }),
});
await first.auth.login({ body: { email: "person@example.test" } });
const second = createClient({
  baseUrl: "https://example.test:9443",
  auth: { session: store },
  fetch: async () => {
    fetched = true;
    return new Response(new ReadableStream({ start(controller) { controller.close(); } }), {
      status: 200,
      headers: { "content-type": "text/event-stream", "X-Trace-ID": "trace" },
    });
  },
});
let failure: any;
try {
  await second.events.watch({ path: { cursor: "cursor" }, query: {}, headers: {} });
} catch (cause) {
  failure = cause;
}
if (failure?._tag !== "AuthUnavailableError") throw new Error(`unexpected failure ${JSON.stringify(failure)}`);
if (fetched) throw new Error("credential reached another origin");
"#;
    run_bun(&files, harness);
}

#[test]
fn follows_same_origin_redirects_before_decoding() {
    let document: pons_ir::Document =
        serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
    let files = emit_with_strategy(&document, EmitMode::Strict, ErrorStrategy::Throw)
        .unwrap()
        .files;
    let harness = r#"
import { createClient } from "./client.ts";

let calls = 0;
const client = createClient({
  baseUrl: "https://example.test",
  fetch: async () => {
    calls += 1;
    if (calls === 1) return new Response(null, { status: 302, headers: { location: "/auth/final" } });
    return new Response(JSON.stringify({ id: "1" }), {
      status: 200,
      headers: { "content-type": "application/json" },
    });
  },
});
const result = await client.auth.login({ body: { email: "person@example.test" } });
if (calls !== 2) throw new Error(`unexpected calls ${calls}`);
if (result.id !== "1") throw new Error(`wrong result ${JSON.stringify(result)}`);
"#;
    run_bun(&files, harness);
}

#[test]
fn reattaches_query_credentials_across_same_origin_redirects() {
    let mut document: pons_ir::Document =
        serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
    document.auth_schemes.insert(
        "queryKey".into(),
        pons_ir::AuthScheme::ApiKey {
            location: ApiKeyLocation::Query,
            name: "api_key".into(),
        },
    );
    document.auth_schemes.insert(
        "adapter".into(),
        pons_ir::AuthScheme::Custom {
            description: "query adapter".into(),
            adapter: None,
            ext: Default::default(),
        },
    );
    document.operations[1].auth.alternatives = vec![vec!["queryKey".into(), "adapter".into()]];
    let files = emit_with_strategy(&document, EmitMode::Strict, ErrorStrategy::Throw)
        .unwrap()
        .files;
    let harness = r#"
import { createClient } from "./client.ts";

const requested: string[] = [];
let calls = 0;
const client = createClient({
  baseUrl: "https://example.test",
  auth: { queryKey: "secret" },
  authAdapters: {
    adapter: () => ({ query: { adapter_token: "attached" } }),
  },
  fetch: async (url) => {
    calls += 1;
    requested.push(`${url.pathname}?${url.searchParams.toString()}`);
    if (calls === 1) return new Response(null, { status: 302, headers: { location: "/events/redirected" } });
    return new Response(new ReadableStream({ start(controller) { controller.close(); } }), {
      status: 200,
      headers: { "content-type": "text/event-stream", "X-Trace-ID": "trace" },
    });
  },
});
await client.events.watch({ path: { cursor: "cursor" }, query: {}, headers: {} });
if (calls !== 2) throw new Error(`unexpected calls ${calls}`);
if (!requested[0].includes("api_key=secret") || !requested[0].includes("adapter_token=attached")) {
  throw new Error(`first request lost its query credentials: ${requested[0]}`);
}
if (!requested[1].includes("api_key=secret")) throw new Error(`redirect lost the api key: ${requested[1]}`);
if (!requested[1].includes("adapter_token=attached")) {
  throw new Error(`redirect lost the adapter query credential: ${requested[1]}`);
}
"#;
    run_bun(&files, harness);
}

#[test]
fn rejects_cross_origin_redirects_with_required_credentials() {
    let mut document: pons_ir::Document =
        serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
    document.operations[1].auth.alternatives = vec![vec!["session".into()]];
    let files = emit_with_strategy(&document, EmitMode::Strict, ErrorStrategy::Throw)
        .unwrap()
        .files;
    let harness = r#"
import { createClient } from "./client.ts";

let calls = 0;
const client = createClient({
  baseUrl: "https://a.test",
  auth: { session: "token" },
  fetch: async () => {
    calls += 1;
    return new Response(null, { status: 302, headers: { location: "https://b.test/events" } });
  },
});
let failure: any;
try {
  await client.events.watch({ path: { cursor: "cursor" }, query: {}, headers: {} });
} catch (cause) {
  failure = cause;
}
if (failure?._tag !== "CredentialRedirectError") throw new Error(`unexpected failure ${JSON.stringify(failure)}`);
if (calls !== 1) throw new Error("credential reached the redirect target");
"#;
    run_bun(&files, harness);
}

#[test]
fn replays_cookie_metadata_across_ports_on_one_host() {
    let mut document: pons_ir::Document =
        serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
    document.auth_schemes.insert(
        "session".into(),
        pons_ir::AuthScheme::CookieSession {
            cookie: "session".into(),
        },
    );
    document.operations[0].provides.as_mut().unwrap().value = CredentialLocator::SetCookie {
        name: "session".into(),
    };
    document.operations[1].auth.alternatives = vec![vec!["session".into()]];

    let files = emit_with_strategy(&document, EmitMode::Strict, ErrorStrategy::Throw)
        .unwrap()
        .files;
    let harness = r#"
import { createClient } from "./client.ts";

let stored: any;
let calls = 0;
const store = {
  get: async () => stored,
  set: async (value: string, metadata: any) => {
    stored = { value, ...metadata };
  },
  clear: async () => {
    stored = undefined;
  },
};
const first = createClient({
  baseUrl: "https://example.test:8443",
  auth: { session: store },
  fetch: async () => {
    calls += 1;
    return new Response(JSON.stringify({ id: "1" }), {
      status: 200,
      headers: {
        "content-type": "application/json",
        "set-cookie": "session=secret; Path=/; Secure",
      },
    });
  },
});
await first.auth.login({ body: { email: "person@example.test" } });
const second = createClient({
  baseUrl: "https://example.test:9443",
  auth: { session: store },
  fetch: async (_url, init) => {
    calls += 1;
    if (new Headers(init?.headers).get("cookie") !== "session=secret") throw new Error("same-host cookie was not replayed");
    return new Response(new ReadableStream({ start(controller) { controller.close(); } }), {
      status: 200,
      headers: {
        "content-type": "text/event-stream",
        "X-Trace-ID": "trace",
      },
    });
  },
});
await second.events.watch({ path: { cursor: "cursor" }, query: {}, headers: {} });
if (calls !== 2) throw new Error(`unexpected request count: ${calls}`);
"#;
    run_bun(&files, harness);
}

#[test]
fn rejects_unscoped_cookie_token_store_records() {
    let mut document: pons_ir::Document =
        serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
    document.auth_schemes.insert(
        "session".into(),
        pons_ir::AuthScheme::CookieSession {
            cookie: "session".into(),
        },
    );
    document.operations[1].auth.alternatives = vec![vec!["session".into()]];

    let files = emit_with_strategy(&document, EmitMode::Strict, ErrorStrategy::Throw)
        .unwrap()
        .files;
    let harness = r#"
import { createClient } from "./client.ts";

let fetched = false;
const client = createClient({
  baseUrl: "https://example.test",
  auth: {
    session: {
      get: async () => ({ value: "secret" }),
      set: async () => {},
      clear: async () => {},
    },
  },
  fetch: async () => {
    fetched = true;
    throw new Error("unscoped cookie reached fetch");
  },
});
let failure: any;
try {
  await client.events.watch({ path: { cursor: "cursor" }, query: {}, headers: {} });
} catch (cause) {
  failure = cause;
}
if (failure?._tag !== "AuthSourceError") throw new Error(`unexpected failure ${JSON.stringify(failure)}`);
if (fetched) throw new Error("unscoped cookie reached fetch");
"#;
    run_bun(&files, harness);
}

#[test]
fn rejects_legacy_token_store_records() {
    let mut document: pons_ir::Document =
        serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
    let provides = document.operations[0].provides.as_mut().unwrap();
    provides.value = CredentialLocator::Header {
        name: "X-Token".into(),
    };
    provides.expires_at = Some(CredentialLocator::Header {
        name: "X-Expires".into(),
    });
    document.operations[1].auth.alternatives = vec![vec!["session".into()]];

    let files = emit_with_strategy(&document, EmitMode::Strict, ErrorStrategy::Throw)
        .unwrap()
        .files;
    let harness = r#"
import { createClient } from "./client.ts";

let fetched: string[] = [];
const watch = async (store: any) => {
  const client = createClient({
    baseUrl: "https://example.test",
    auth: { session: store },
    fetch: async (_url, init) => {
      fetched.push(new Headers(init?.headers).get("authorization") ?? "");
      return new Response(new ReadableStream({ start(controller) { controller.close(); } }), {
        status: 200,
        headers: { "content-type": "text/event-stream", "X-Trace-ID": "trace" },
      });
    },
  });
  let failure: any;
  try {
    await client.events.watch({ path: { cursor: "cursor" }, query: {}, headers: {} });
  } catch (cause) {
    failure = cause;
  }
  return failure;
};
const expectSourceFailure = (failure: any, message: string) => {
  if (failure?._tag !== "AuthSourceError") throw new Error(`unexpected failure ${JSON.stringify(failure)}`);
  if (failure?.cause?.message !== message) throw new Error(`unexpected cause ${failure?.cause?.message}`);
};

let legacy: any;
const store = {
  get: async () => legacy,
  set: async (value: string, expiresAt: any) => { legacy = { value, expiresAt }; },
  clear: async () => { legacy = undefined; },
};
const writer = createClient({
  baseUrl: "https://example.test",
  auth: { session: store },
  fetch: async () => new Response(JSON.stringify({ id: "1" }), {
    status: 200,
    headers: {
      "content-type": "application/json",
      "X-Token": "captured",
      "X-Expires": new Date(Date.now() + 3_600_000).toISOString(),
    },
  }),
});
await writer.auth.login({ body: { email: "person@example.test" } });
expectSourceFailure(await watch(store), "token store record expiresAt must be a number");
expectSourceFailure(
  await watch({ get: async () => "legacy", set: async () => {}, clear: async () => {} }),
  "token store must return a credential record, not a string",
);
expectSourceFailure(await watch({ get: async () => ({}), set: async () => {}, clear: async () => {} }), "token store record must have a string value");
expectSourceFailure(
  await watch({ get: async () => ({ value: "x", expiresAt: "2000-01-01T00:00:00Z" }), set: async () => {}, clear: async () => {} }),
  "token store record expiresAt must be a number",
);
if (fetched.length !== 0) throw new Error(`rejected credential reached fetch: ${fetched.join(",")}`);
const accepted = await watch({ get: async () => ({ value: "kept" }), set: async () => {}, clear: async () => {} });
if (accepted !== undefined) throw new Error(`valid record was rejected: ${JSON.stringify(accepted)}`);
if (fetched[0] !== "Bearer kept") throw new Error(`unexpected credential ${fetched[0]}`);
"#;
    run_bun(&files, harness);
}

#[test]
fn does_not_resurrect_a_deleted_cookie_source() {
    let mut document: pons_ir::Document =
        serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
    document.auth_schemes.insert(
        "session".into(),
        pons_ir::AuthScheme::CookieSession {
            cookie: "session".into(),
        },
    );
    document.operations[0].provides.as_mut().unwrap().value = CredentialLocator::SetCookie {
        name: "session".into(),
    };
    document.operations[1].auth.alternatives = vec![vec!["session".into()], vec!["tenant".into()]];

    let files = emit_with_strategy(&document, EmitMode::Strict, ErrorStrategy::Throw)
        .unwrap()
        .files;
    let harness = r#"
import { createClient } from "./client.ts";

let calls = 0;
const client = createClient({
  baseUrl: "https://example.test",
  auth: {
    session: () => "revoked",
    tenant: "tenant",
  },
  fetch: async (_url, init) => {
    calls += 1;
    if (calls === 1) {
      return new Response(JSON.stringify({ id: "1" }), {
        status: 200,
        headers: {
          "content-type": "application/json",
          "set-cookie": "session=revoked; Max-Age=0",
        },
      });
    }
    const headers = new Headers(init?.headers);
    if (headers.get("cookie") !== null) throw new Error("deleted source was replayed");
    if (headers.get("X-Tenant") !== "tenant") throw new Error("fallback was not selected");
    return new Response(new ReadableStream({ start(controller) { controller.close(); } }), {
      status: 200,
      headers: {
        "content-type": "text/event-stream",
        "X-Trace-ID": "trace",
      },
    });
  },
});
await client.auth.login({ body: { email: "person@example.test" } });
await client.events.watch({ path: { cursor: "cursor" }, query: {}, headers: {} });
"#;
    run_bun(&files, harness);
}

#[test]
fn falls_back_after_deleting_an_ambient_cookie() {
    let mut document: pons_ir::Document =
        serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
    document.auth_schemes.insert(
        "session".into(),
        pons_ir::AuthScheme::CookieSession {
            cookie: "session".into(),
        },
    );
    document.operations[0].provides.as_mut().unwrap().value = CredentialLocator::SetCookie {
        name: "session".into(),
    };
    document.operations[1].auth.alternatives = vec![vec!["session".into()], vec!["tenant".into()]];

    let files = emit_with_strategy(&document, EmitMode::Strict, ErrorStrategy::Throw)
        .unwrap()
        .files;
    let harness = r#"
import { createClient } from "./client.ts";

let calls = 0;
const client = createClient({
  baseUrl: "https://example.test",
  auth: { tenant: "tenant" },
  fetch: async (_url, init) => {
    calls += 1;
    if (calls === 1) {
      return new Response(JSON.stringify({ id: "1" }), {
        status: 200,
        headers: {
          "content-type": "application/json",
          "set-cookie": "session=deleted; Max-Age=0",
        },
      });
    }
    const headers = new Headers(init?.headers);
    if (headers.get("cookie") !== null) throw new Error("ambient cookie was replayed");
    if (headers.get("X-Tenant") !== "tenant") throw new Error("fallback was not selected");
    return new Response(new ReadableStream({ start(controller) { controller.close(); } }), {
      status: 200,
      headers: {
        "content-type": "text/event-stream",
        "X-Trace-ID": "trace",
      },
    });
  },
});
await client.auth.login({ body: { email: "person@example.test" } });
await client.events.watch({ path: { cursor: "cursor" }, query: {}, headers: {} });
"#;
    run_bun(&files, harness);
}

#[test]
fn ignores_cookie_deletions_for_another_path() {
    let mut document: pons_ir::Document =
        serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
    document.auth_schemes.insert(
        "session".into(),
        pons_ir::AuthScheme::CookieSession {
            cookie: "session".into(),
        },
    );
    document.operations[0].provides.as_mut().unwrap().value = CredentialLocator::SetCookie {
        name: "session".into(),
    };
    document.operations[1].auth.alternatives = vec![vec!["session".into()]];
    document.operations[1].path = vec![
        PathSegment::Literal { lit: "auth".into() },
        PathSegment::Literal {
            lit: "refresh".into(),
        },
    ];

    let files = emit_with_strategy(&document, EmitMode::Strict, ErrorStrategy::Throw)
        .unwrap()
        .files;
    let harness = r#"
import { createClient } from "./client.ts";

let calls = 0;
const client = createClient({
  baseUrl: "https://example.test",
  fetch: async (_url, init) => {
    calls += 1;
    if (calls === 1) {
      return new Response(JSON.stringify({ id: "1" }), {
        status: 200,
        headers: {
          "content-type": "application/json",
          "set-cookie": "session=live; Path=/auth; Secure",
        },
      });
    }
    if (calls === 2) {
      return new Response(JSON.stringify({ id: "1" }), {
        status: 200,
        headers: {
          "content-type": "application/json",
          "set-cookie": "session=deleted; Path=/other; Max-Age=0",
        },
      });
    }
    const headers = new Headers(init?.headers);
    if (headers.get("cookie") !== "session=live") throw new Error(`wrong cookie: ${headers.get("cookie")}`);
    return new Response(new ReadableStream({ start(controller) { controller.close(); } }), {
      status: 200,
      headers: {
        "content-type": "text/event-stream",
        "X-Trace-ID": "trace",
      },
    });
  },
});
await client.auth.login({ body: { email: "person@example.test" } });
await client.auth.login({ body: { email: "person@example.test" } });
await client.events.watch({ query: {}, headers: {} });
"#;
    run_bun(&files, harness);
}

#[test]
fn keeps_broader_cookies_when_a_narrower_one_arrives() {
    let mut document: pons_ir::Document =
        serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
    document.auth_schemes.insert(
        "session".into(),
        pons_ir::AuthScheme::CookieSession {
            cookie: "session".into(),
        },
    );
    document.operations[0].provides.as_mut().unwrap().value = CredentialLocator::SetCookie {
        name: "session".into(),
    };
    document.operations[1].auth.alternatives = vec![vec!["session".into()]];
    let mut scoped = document.operations[1].clone();
    scoped.id = vec![vec!["admin".into()], vec!["events".into()]];
    scoped.path.insert(
        0,
        PathSegment::Literal {
            lit: "admin".into(),
        },
    );
    document.operations.push(scoped);

    let files = emit_with_strategy(&document, EmitMode::Strict, ErrorStrategy::Throw)
        .unwrap()
        .files;
    let harness = r#"
import { createClient } from "./client.ts";

let calls = 0;
let narrow = true;
const client = createClient({
  baseUrl: "https://example.test",
  fetch: async (_url, init) => {
    calls += 1;
    if (calls === 5) {
      narrow = false;
      return new Response(JSON.stringify({ id: "1" }), {
        status: 200,
        headers: {
          "content-type": "application/json",
          "set-cookie": "session=narrow; Path=/admin; Max-Age=0",
        },
      });
    }
    if (calls <= 2) {
      const cookie = calls === 1 ? "session=broad; Path=/" : "session=narrow; Path=/admin";
      return new Response(JSON.stringify({ id: "1" }), {
        status: 200,
        headers: { "content-type": "application/json", "set-cookie": cookie },
      });
    }
    const scoped = new URL(String(_url)).pathname.startsWith("/admin");
    const expected = narrow && scoped ? "session=narrow" : "session=broad";
    const sent = new Headers(init?.headers).get("cookie");
    if (sent !== expected) throw new Error(`wrong cookie for ${expected}: ${sent}`);
    return new Response(new ReadableStream({ start(controller) { controller.close(); } }), {
      status: 200,
      headers: {
        "content-type": "text/event-stream",
        "X-Trace-ID": "trace",
      },
    });
  },
});
await client.auth.login({ body: { email: "person@example.test" } });
await client.auth.login({ body: { email: "person@example.test" } });
await client.events.watch({ path: { cursor: "cursor" }, query: {}, headers: {} });
await client.admin.events({ path: { cursor: "cursor" }, query: {}, headers: {} });
await client.auth.login({ body: { email: "person@example.test" } });
await client.admin.events({ path: { cursor: "cursor" }, query: {}, headers: {} });
if (calls !== 6) throw new Error(`unexpected calls ${calls}`);
"#;
    run_bun(&files, harness);
}

#[test]
fn does_not_resurrect_an_expired_configured_cookie() {
    let mut document: pons_ir::Document =
        serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
    document.auth_schemes.insert(
        "session".into(),
        pons_ir::AuthScheme::CookieSession {
            cookie: "session".into(),
        },
    );
    document.operations[0].provides.as_mut().unwrap().value = CredentialLocator::SetCookie {
        name: "session".into(),
    };
    document.operations[1].auth.alternatives = vec![vec!["session".into()], vec!["tenant".into()]];

    let files = emit_with_strategy(&document, EmitMode::Strict, ErrorStrategy::Throw)
        .unwrap()
        .files;
    let harness = r#"
import { createClient } from "./client.ts";

let calls = 0;
const client = createClient({
  baseUrl: "https://example.test",
  auth: {
    session: () => "expired",
    tenant: "tenant",
  },
  fetch: async (_url, init) => {
    calls += 1;
    if (calls === 1) {
      return new Response(JSON.stringify({ id: "1" }), {
        status: 200,
        headers: {
          "content-type": "application/json",
          "set-cookie": "session=expired; Max-Age=0.01",
        },
      });
    }
    const headers = new Headers(init?.headers);
    if (headers.get("cookie") !== null) throw new Error("expired source was replayed");
    if (headers.get("X-Tenant") !== "tenant") throw new Error("fallback was not selected");
    return new Response(new ReadableStream({ start(controller) { controller.close(); } }), {
      status: 200,
      headers: {
        "content-type": "text/event-stream",
        "X-Trace-ID": "trace",
      },
    });
  },
});
await client.auth.login({ body: { email: "person@example.test" } });
await new Promise((resolve) => setTimeout(resolve, 20));
await client.events.watch({ path: { cursor: "cursor" }, query: {}, headers: {} });
"#;
    run_bun(&files, harness);
}

#[test]
fn scopes_captured_cookie_credentials_to_the_default_path() {
    let mut document: pons_ir::Document =
        serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
    document.auth_schemes.insert(
        "session".into(),
        pons_ir::AuthScheme::CookieSession {
            cookie: "session".into(),
        },
    );
    document.operations[0].provides.as_mut().unwrap().value = CredentialLocator::SetCookie {
        name: "session".into(),
    };
    document.operations[1].auth.alternatives = vec![vec!["session".into()], vec!["tenant".into()]];

    let files = emit_with_strategy(&document, EmitMode::Strict, ErrorStrategy::Throw)
        .unwrap()
        .files;
    let harness = r#"
import { createClient } from "./client.ts";

let calls = 0;
const client = createClient({
  baseUrl: "http://example.test",
  auth: { tenant: "tenant" },
  fetch: async (_url, init) => {
    calls += 1;
    if (calls === 1) {
      return new Response(JSON.stringify({ id: "1" }), {
        status: 200,
        headers: {
          "content-type": "application/json",
          "set-cookie": "session=secret",
        },
      });
    }
    const headers = new Headers(init?.headers);
    if (headers.get("cookie") !== null) throw new Error("out-of-path cookie was replayed");
    if (headers.get("X-Tenant") !== "tenant") throw new Error("fallback was not selected");
    return new Response(new ReadableStream({ start(controller) { controller.close(); } }), {
      status: 200,
      headers: {
        "content-type": "text/event-stream",
        "X-Trace-ID": "trace",
      },
    });
  },
});
await client.auth.login({ body: { email: "person@example.test" } });
await client.events.watch({ path: { cursor: "cursor" }, query: {}, headers: {} });
"#;
    run_bun(&files, harness);
}

#[test]
fn does_not_replay_secure_cookies_over_http() {
    let mut document: pons_ir::Document =
        serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
    document.auth_schemes.insert(
        "session".into(),
        pons_ir::AuthScheme::CookieSession {
            cookie: "session".into(),
        },
    );
    document.operations[0].provides.as_mut().unwrap().value = CredentialLocator::SetCookie {
        name: "session".into(),
    };
    document.operations[1].auth.alternatives = vec![vec!["session".into()], vec!["tenant".into()]];

    let files = emit_with_strategy(&document, EmitMode::Strict, ErrorStrategy::Throw)
        .unwrap()
        .files;
    let harness = r#"
import { createClient } from "./client.ts";

let calls = 0;
const client = createClient({
  baseUrl: "http://example.test",
  auth: { tenant: "tenant" },
  fetch: async (_url, init) => {
    calls += 1;
    if (calls === 1) {
      return new Response(JSON.stringify({ id: "1" }), {
        status: 200,
        headers: {
          "content-type": "application/json",
          "set-cookie": "session=secret; Secure",
        },
      });
    }
    const headers = new Headers(init?.headers);
    if (headers.get("cookie") !== null) throw new Error("secure cookie was replayed over HTTP");
    if (headers.get("X-Tenant") !== "tenant") throw new Error("fallback was not selected");
    return new Response(new ReadableStream({ start(controller) { controller.close(); } }), {
      status: 200,
      headers: {
        "content-type": "text/event-stream",
        "X-Trace-ID": "trace",
      },
    });
  },
});
await client.auth.login({ body: { email: "person@example.test" } });
await client.events.watch({ path: { cursor: "cursor" }, query: {}, headers: {} });
"#;
    run_bun(&files, harness);
}

#[test]
fn rejects_insecure_secure_prefix_cookies() {
    let mut document: pons_ir::Document =
        serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
    document.auth_schemes.insert(
        "session".into(),
        pons_ir::AuthScheme::CookieSession {
            cookie: "__Secure-session".into(),
        },
    );
    document.operations[0].provides.as_mut().unwrap().value = CredentialLocator::SetCookie {
        name: "__Secure-session".into(),
    };
    document.operations[1].auth.alternatives = vec![vec!["session".into()], vec!["tenant".into()]];

    let files = emit_with_strategy(&document, EmitMode::Strict, ErrorStrategy::Throw)
        .unwrap()
        .files;
    let harness = r#"
import { createClient } from "./client.ts";

let calls = 0;
const client = createClient({
  baseUrl: "https://example.test",
  auth: { tenant: "tenant" },
  fetch: async (_url, init) => {
    calls += 1;
    if (calls === 1) {
      return new Response(JSON.stringify({ id: "1" }), {
        status: 200,
        headers: {
          "content-type": "application/json",
          "set-cookie": "__Secure-session=secret; Path=/",
        },
      });
    }
    const headers = new Headers(init?.headers);
    if (headers.get("cookie") !== null) throw new Error("invalid __Secure- cookie was replayed");
    if (headers.get("X-Tenant") !== "tenant") throw new Error("fallback was not selected");
    return new Response(new ReadableStream({ start(controller) { controller.close(); } }), {
      status: 200,
      headers: {
        "content-type": "text/event-stream",
        "X-Trace-ID": "trace",
      },
    });
  },
});
await client.auth.login({ body: { email: "person@example.test" } });
await client.events.watch({ path: { cursor: "cursor" }, query: {}, headers: {} });
"#;
    run_bun(&files, harness);
}

#[test]
fn validates_prefixes_on_the_effective_cookie_name() {
    let mut document: pons_ir::Document =
        serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
    document.auth_schemes.insert(
        "session".into(),
        pons_ir::AuthScheme::CookieSession {
            cookie: "__Secure-session".into(),
        },
    );
    document.operations[0].provides.as_mut().unwrap().value = CredentialLocator::SetCookie {
        name: "session".into(),
    };
    document.operations[1].auth.alternatives = vec![vec!["session".into()], vec!["tenant".into()]];

    let files = emit_with_strategy(&document, EmitMode::Strict, ErrorStrategy::Throw)
        .unwrap()
        .files;
    let harness = r#"
import { createClient } from "./client.ts";

let calls = 0;
const client = createClient({
  baseUrl: "https://example.test",
  auth: { tenant: "tenant" },
  fetch: async (_url, init) => {
    calls += 1;
    if (calls === 1) {
      return new Response(JSON.stringify({ id: "1" }), {
        status: 200,
        headers: {
          "content-type": "application/json",
          "set-cookie": "session=secret; Path=/",
        },
      });
    }
    const headers = new Headers(init?.headers);
    if (headers.get("cookie") !== null) throw new Error("unsecured source fed a __Secure- cookie");
    if (headers.get("X-Tenant") !== "tenant") throw new Error("fallback was not selected");
    return new Response(new ReadableStream({ start(controller) { controller.close(); } }), {
      status: 200,
      headers: {
        "content-type": "text/event-stream",
        "X-Trace-ID": "trace",
      },
    });
  },
});
await client.auth.login({ body: { email: "person@example.test" } });
await client.events.watch({ path: { cursor: "cursor" }, query: {}, headers: {} });
"#;
    run_bun(&files, harness);
}

#[test]
fn validates_prefixes_for_cookie_api_keys() {
    let mut document: pons_ir::Document =
        serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
    document.auth_schemes.insert(
        "api".into(),
        pons_ir::AuthScheme::ApiKey {
            location: ApiKeyLocation::Cookie,
            name: "__Secure-api".into(),
        },
    );
    document.operations[0].provides.as_mut().unwrap().scheme = "api".into();
    document.operations[0].provides.as_mut().unwrap().value = CredentialLocator::SetCookie {
        name: "session".into(),
    };
    document.operations[1].auth.alternatives = vec![vec!["api".into()], vec!["tenant".into()]];

    let files = emit_with_strategy(&document, EmitMode::Strict, ErrorStrategy::Throw)
        .unwrap()
        .files;
    let harness = r#"
import { createClient } from "./client.ts";

let calls = 0;
const client = createClient({
  baseUrl: "https://example.test",
  auth: { tenant: "tenant" },
  fetch: async (_url, init) => {
    calls += 1;
    if (calls === 1) {
      return new Response(JSON.stringify({ id: "1" }), {
        status: 200,
        headers: {
          "content-type": "application/json",
          "set-cookie": "session=secret; Path=/",
        },
      });
    }
    const headers = new Headers(init?.headers);
    if (headers.get("cookie") !== null) throw new Error("unsecured source fed a __Secure- API key");
    if (headers.get("X-Tenant") !== "tenant") throw new Error("fallback was not selected");
    return new Response(new ReadableStream({ start(controller) { controller.close(); } }), {
      status: 200,
      headers: {
        "content-type": "text/event-stream",
        "X-Trace-ID": "trace",
      },
    });
  },
});
await client.auth.login({ body: { email: "person@example.test" } });
await client.events.watch({ path: { cursor: "cursor" }, query: {}, headers: {} });
"#;
    run_bun(&files, harness);
}

#[test]
fn revalidates_cookie_api_key_origin_after_transport_request_hooks() {
    let mut document: pons_ir::Document =
        serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
    document.auth_schemes.insert(
        "api".into(),
        pons_ir::AuthScheme::ApiKey {
            location: ApiKeyLocation::Cookie,
            name: "__Secure-api".into(),
        },
    );
    document.operations[0].provides.as_mut().unwrap().scheme = "api".into();
    document.operations[0].provides.as_mut().unwrap().value = CredentialLocator::SetCookie {
        name: "session".into(),
    };
    document.operations[1].auth.alternatives = vec![vec!["api".into()]];

    let files = emit_with_strategy(&document, EmitMode::Strict, ErrorStrategy::Throw)
        .unwrap()
        .files;
    let harness = r#"
import { createClient } from "./client.ts";

let calls = 0;
const client = createClient({
  baseUrl: "https://example.test",
  transport: {
    onRequest: async ({ url }) => {
      if (url.pathname === "/auth/login") return;
      url.hostname = "idp.test";
    },
  },
  fetch: async () => {
    calls += 1;
    if (calls === 1) {
      return new Response(JSON.stringify({ id: "1" }), {
        status: 200,
        headers: {
          "content-type": "application/json",
          "set-cookie": "session=secret; Path=/; Secure",
        },
      });
    }
    throw new Error("cookie API key reached a rewritten host");
  },
});
await client.auth.login({ body: { email: "person@example.test" } });
let failure: any;
try {
  await client.events.watch({ path: { cursor: "cursor" }, query: {}, headers: {} });
} catch (cause) {
  failure = cause;
}
if (failure?._tag !== "AuthUnavailableError") throw new Error(`unexpected failure ${JSON.stringify(failure)}`);
if (calls !== 1) throw new Error("cookie API key reached a rewritten host");
"#;
    run_bun(&files, harness);
}

#[test]
fn rejects_non_root_host_prefix_cookies() {
    let mut document: pons_ir::Document =
        serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
    document.auth_schemes.insert(
        "session".into(),
        pons_ir::AuthScheme::CookieSession {
            cookie: "__Host-session".into(),
        },
    );
    document.operations[0].provides.as_mut().unwrap().value = CredentialLocator::SetCookie {
        name: "__Host-session".into(),
    };
    document.operations[1].auth.alternatives = vec![vec!["session".into()], vec!["tenant".into()]];

    let files = emit_with_strategy(&document, EmitMode::Strict, ErrorStrategy::Throw)
        .unwrap()
        .files;
    let harness = r#"
import { createClient } from "./client.ts";

let calls = 0;
const client = createClient({
  baseUrl: "https://example.test",
  auth: { tenant: "tenant" },
  fetch: async (_url, init) => {
    calls += 1;
    if (calls === 1) {
      return new Response(JSON.stringify({ id: "1" }), {
        status: 200,
        headers: {
          "content-type": "application/json",
          "set-cookie": "__Host-session=secret; Path=/auth; Secure",
        },
      });
    }
    const headers = new Headers(init?.headers);
    if (headers.get("cookie") !== null) throw new Error("invalid __Host- cookie was replayed");
    if (headers.get("X-Tenant") !== "tenant") throw new Error("fallback was not selected");
    return new Response(new ReadableStream({ start(controller) { controller.close(); } }), {
      status: 200,
      headers: {
        "content-type": "text/event-stream",
        "X-Trace-ID": "trace",
      },
    });
  },
});
await client.auth.login({ body: { email: "person@example.test" } });
await client.events.watch({ path: { cursor: "cursor" }, query: {}, headers: {} });
"#;
    run_bun(&files, harness);
}

#[test]
fn parses_combined_set_cookie_headers_without_get_set_cookie() {
    let mut document: pons_ir::Document =
        serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
    document.auth_schemes.insert(
        "session".into(),
        pons_ir::AuthScheme::CookieSession {
            cookie: "session".into(),
        },
    );
    document.operations[0].provides.as_mut().unwrap().value = CredentialLocator::SetCookie {
        name: "session".into(),
    };
    document.operations[1].auth.alternatives = vec![vec!["session".into()]];

    let files = emit_with_strategy(&document, EmitMode::Strict, ErrorStrategy::Throw)
        .unwrap()
        .files;
    let harness = r#"
import { createClient } from "./client.ts";

let calls = 0;
const client = createClient({
  baseUrl: "https://example.test",
  fetch: async (_url, init) => {
    calls += 1;
    if (calls === 1) {
      return {
        status: 200,
        url: "",
        headers: {
          get(name: string) {
            if (name.toLowerCase() === "content-type") return "application/json";
            if (name.toLowerCase() === "set-cookie") {
              return "session=old; Expires=Wed, 21 Oct 2015 07:28:00 GMT; Path=/, session=current; Path=/";
            }
            return null;
          },
        },
        text: async () => JSON.stringify({ id: "1" }),
      } as unknown as Response;
    }
    const headers = new Headers(init?.headers);
    if (headers.get("cookie") !== "session=current") throw new Error(`wrong cookie: ${headers.get("cookie")}`);
    return new Response(new ReadableStream({ start(controller) { controller.close(); } }), {
      status: 200,
      headers: {
        "content-type": "text/event-stream",
        "X-Trace-ID": "trace",
      },
    });
  },
});
await client.auth.login({ body: { email: "person@example.test" } });
await client.events.watch({ path: { cursor: "cursor" }, query: {}, headers: {} });
"#;
    run_bun(&files, harness);
}

#[test]
fn normalizes_relative_cookie_paths() {
    let mut document: pons_ir::Document =
        serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
    document.auth_schemes.insert(
        "session".into(),
        pons_ir::AuthScheme::CookieSession {
            cookie: "session".into(),
        },
    );
    document.operations[0].provides.as_mut().unwrap().value = CredentialLocator::SetCookie {
        name: "session".into(),
    };
    document.operations[1].auth.alternatives = vec![vec!["session".into()]];
    document.operations[1].path = vec![
        PathSegment::Literal { lit: "auth".into() },
        PathSegment::Literal {
            lit: "refresh".into(),
        },
    ];

    let files = emit_with_strategy(&document, EmitMode::Strict, ErrorStrategy::Throw)
        .unwrap()
        .files;
    let harness = r#"
import { createClient } from "./client.ts";

let calls = 0;
const client = createClient({
  baseUrl: "https://example.test",
  fetch: async (_url, init) => {
    calls += 1;
    if (calls === 1) {
      return new Response(JSON.stringify({ id: "1" }), {
        status: 200,
        headers: {
          "content-type": "application/json",
          "set-cookie": "session=secret; Path=auth",
        },
      });
    }
    const headers = new Headers(init?.headers);
    if (headers.get("cookie") !== "session=secret") throw new Error(`wrong cookie: ${headers.get("cookie")}`);
    return new Response(new ReadableStream({ start(controller) { controller.close(); } }), {
      status: 200,
      headers: {
        "content-type": "text/event-stream",
        "X-Trace-ID": "trace",
      },
    });
  },
});
await client.auth.login({ body: { email: "person@example.test" } });
await client.events.watch({ query: {}, headers: {} });
"#;
    run_bun(&files, harness);
}

#[test]
fn does_not_replay_a_cross_origin_cookie() {
    let mut document: pons_ir::Document =
        serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
    document.auth_schemes.insert(
        "session".into(),
        pons_ir::AuthScheme::CookieSession {
            cookie: "session".into(),
        },
    );
    document.operations[0].provides.as_mut().unwrap().value = CredentialLocator::SetCookie {
        name: "session".into(),
    };
    document.operations[1].auth.alternatives = vec![vec!["session".into()]];

    let files = emit_with_strategy(&document, EmitMode::Strict, ErrorStrategy::Throw)
        .unwrap()
        .files;
    let harness = r#"
import { createClient } from "./client.ts";

let calls = 0;
const client = createClient({
  baseUrl: "https://example.test",
  fetch: async () => {
    calls += 1;
    if (calls === 1) {
      return {
        status: 200,
        url: "https://idp.test/oauth/callback",
        headers: {
          get(name: string) {
            if (name.toLowerCase() === "content-type") return "application/json";
            if (name.toLowerCase() === "set-cookie") return "session=idp; Path=/; Secure";
            return null;
          },
        },
        text: async () => JSON.stringify({ id: "1" }),
      } as unknown as Response;
    }
    throw new Error("cross-origin cookie reached fetch");
  },
});
await client.auth.login({ body: { email: "person@example.test" } });
let failure: any;
try {
  await client.events.watch({ path: { cursor: "cursor" }, query: {}, headers: {} });
} catch (cause) {
  failure = cause;
}
if (failure?._tag !== "AuthUnavailableError") throw new Error(`unexpected failure ${JSON.stringify(failure)}`);
if (calls !== 1) throw new Error("cross-origin cookie reached fetch");
"#;
    run_bun(&files, harness);
}

#[test]
fn revalidates_captured_bearer_origin_after_transport_request_hooks() {
    let mut document: pons_ir::Document =
        serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
    document.operations[0].provides.as_mut().unwrap().value = CredentialLocator::Header {
        name: "X-Token".into(),
    };
    document.operations[1].auth.alternatives = vec![vec!["session".into()]];

    let files = emit_with_strategy(&document, EmitMode::Strict, ErrorStrategy::Throw)
        .unwrap()
        .files;
    let harness = r#"
import { createClient } from "./client.ts";

let calls = 0;
const client = createClient({
  baseUrl: "https://example.test",
  transport: {
    onRequest: async ({ url }) => {
      if (url.pathname === "/auth/login") return;
      url.hostname = "idp.test";
    },
  },
  fetch: async () => {
    calls += 1;
    if (calls === 1) {
      return new Response(JSON.stringify({ id: "1" }), {
        status: 200,
        headers: {
          "content-type": "application/json",
          "X-Token": "secret",
        },
      });
    }
    throw new Error("captured bearer reached a rewritten host");
  },
});
await client.auth.login({ body: { email: "person@example.test" } });
let failure: any;
try {
  await client.events.watch({ path: { cursor: "cursor" }, query: {}, headers: {} });
} catch (cause) {
  failure = cause;
}
if (failure?._tag !== "AuthUnavailableError") throw new Error(`unexpected failure ${JSON.stringify(failure)}`);
if (calls !== 1) throw new Error("captured bearer reached a rewritten host");
"#;
    run_bun(&files, harness);
}

#[test]
fn revalidates_cookie_scope_after_transport_request_hooks() {
    let mut document: pons_ir::Document =
        serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
    document.auth_schemes.insert(
        "session".into(),
        pons_ir::AuthScheme::CookieSession {
            cookie: "session".into(),
        },
    );
    document.operations[0].provides.as_mut().unwrap().value = CredentialLocator::SetCookie {
        name: "session".into(),
    };
    document.operations[1].auth.alternatives = vec![vec!["session".into()]];
    document.operations[1].path = vec![
        PathSegment::Literal { lit: "auth".into() },
        PathSegment::Literal {
            lit: "refresh".into(),
        },
    ];

    let files = emit_with_strategy(&document, EmitMode::Strict, ErrorStrategy::Throw)
        .unwrap()
        .files;
    let harness = r#"
import { createClient } from "./client.ts";

let calls = 0;
const client = createClient({
  baseUrl: "https://example.test",
  transport: {
    onRequest: async ({ url }) => {
      if (url.pathname === "/auth/login") return;
      url.protocol = "http:";
      url.pathname = "/events/watch";
    },
  },
  fetch: async () => {
    calls += 1;
    if (calls > 1) throw new Error("request should not be sent");
    return new Response(JSON.stringify({ id: "1" }), {
      status: 200,
      headers: {
        "content-type": "application/json",
        "set-cookie": "session=secret; Path=/auth; Secure",
      },
    });
  },
});
let failure: any;
try {
  await client.auth.login({ body: { email: "person@example.test" } });
  await client.events.watch({ query: {}, headers: {} });
} catch (cause) {
  failure = cause;
}
if (failure?._tag !== "AuthUnavailableError") throw new Error(`unexpected failure ${JSON.stringify(failure)}`);
if (calls !== 1) throw new Error("rewritten request reached fetch");
"#;
    run_bun(&files, harness);
}

#[test]
fn revalidates_cookie_origin_after_transport_request_hooks() {
    let mut document: pons_ir::Document =
        serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
    document.auth_schemes.insert(
        "session".into(),
        pons_ir::AuthScheme::CookieSession {
            cookie: "session".into(),
        },
    );
    document.operations[0].provides.as_mut().unwrap().value = CredentialLocator::SetCookie {
        name: "session".into(),
    };
    document.operations[1].auth.alternatives = vec![vec!["session".into()]];

    let files = emit_with_strategy(&document, EmitMode::Strict, ErrorStrategy::Throw)
        .unwrap()
        .files;
    let harness = r#"
import { createClient } from "./client.ts";

let calls = 0;
const client = createClient({
  baseUrl: "https://example.test",
  transport: {
    onRequest: async ({ url }) => {
      if (url.pathname === "/auth/login") return;
      url.hostname = "idp.test";
    },
  },
  fetch: async () => {
    calls += 1;
    if (calls > 1) throw new Error("cross-origin request reached fetch");
    return new Response(JSON.stringify({ id: "1" }), {
      status: 200,
      headers: {
        "content-type": "application/json",
        "set-cookie": "session=secret; Path=/; Secure",
      },
    });
  },
});
let failure: any;
try {
  await client.auth.login({ body: { email: "person@example.test" } });
  await client.events.watch({ path: { cursor: "cursor" }, query: {}, headers: {} });
} catch (cause) {
  failure = cause;
}
if (failure?._tag !== "AuthUnavailableError") throw new Error(`unexpected failure ${JSON.stringify(failure)}`);
if (calls !== 1) throw new Error("cross-origin request reached fetch");
"#;
    run_bun(&files, harness);
}

#[test]
fn snapshots_cookie_scope_before_transport_revalidation() {
    let mut document: pons_ir::Document =
        serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
    document.auth_schemes.insert(
        "session".into(),
        pons_ir::AuthScheme::CookieSession {
            cookie: "session".into(),
        },
    );
    document.operations[0].provides.as_mut().unwrap().value = CredentialLocator::SetCookie {
        name: "session".into(),
    };
    document.operations[1].auth.alternatives = vec![vec!["session".into()]];
    document.operations[1].path = vec![
        PathSegment::Literal { lit: "auth".into() },
        PathSegment::Literal {
            lit: "refresh".into(),
        },
    ];

    let files = emit_with_strategy(&document, EmitMode::Strict, ErrorStrategy::Throw)
        .unwrap()
        .files;
    let harness = r#"
import { createClient } from "./client.ts";

let calls = 0;
let protectedFetched = false;
let release!: () => void;
const gate = new Promise<void>((resolve) => { release = resolve; });
let started!: () => void;
const entered = new Promise<void>((resolve) => { started = resolve; });
const client = createClient({
  baseUrl: "https://example.test",
  transport: {
    onRequest: async ({ url }) => {
      if (url.pathname === "/auth/login") return;
      started();
      await gate;
      url.protocol = "http:";
      url.pathname = "/events/watch";
    },
  },
  fetch: async () => {
    calls += 1;
    if (calls === 1) {
      return new Response(JSON.stringify({ id: "1" }), {
        status: 200,
        headers: {
          "content-type": "application/json",
          "set-cookie": "session=old; Path=/auth; Secure",
        },
      });
    }
    if (calls === 2) {
      return new Response(JSON.stringify({ id: "1" }), {
        status: 200,
        headers: {
          "content-type": "application/json",
          "set-cookie": "session=new; Path=/",
        },
      });
    }
    protectedFetched = true;
    throw new Error("stale cookie reached fetch");
  },
});
await client.auth.login({ body: { email: "person@example.test" } });
const request = client.events.watch({ query: {}, headers: {} }).then(() => undefined, (cause) => cause);
await entered;
await client.auth.login({ body: { email: "person@example.test" } });
release();
const failure: any = await request;
if (failure?._tag !== "AuthUnavailableError") throw new Error(`unexpected failure ${JSON.stringify(failure)}`);
if (protectedFetched) throw new Error("stale cookie reached fetch");
"#;
    run_bun(&files, harness);
}

#[test]
fn snapshots_cookie_scope_across_concurrent_capture() {
    let mut document: pons_ir::Document =
        serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
    document.auth_schemes.insert(
        "session".into(),
        pons_ir::AuthScheme::CookieSession {
            cookie: "session".into(),
        },
    );
    document.auth_schemes.insert(
        "adapter".into(),
        pons_ir::AuthScheme::Custom {
            description: "delayed adapter".into(),
            adapter: None,
            ext: Default::default(),
        },
    );
    document.operations[0].provides.as_mut().unwrap().value = CredentialLocator::SetCookie {
        name: "session".into(),
    };
    document.operations[1].auth.alternatives = vec![vec!["session".into(), "adapter".into()]];
    document.operations[1].path = vec![
        PathSegment::Literal { lit: "auth".into() },
        PathSegment::Literal {
            lit: "refresh".into(),
        },
    ];

    let files = emit_with_strategy(&document, EmitMode::Strict, ErrorStrategy::Throw)
        .unwrap()
        .files;
    let harness = r#"
import { createClient } from "./client.ts";

let calls = 0;
let protectedFetched = false;
let release!: () => void;
const gate = new Promise<void>((resolve) => { release = resolve; });
let started!: () => void;
const entered = new Promise<void>((resolve) => { started = resolve; });
const client = createClient({
  baseUrl: "https://example.test",
  authAdapters: {
    adapter: async () => {
      started();
      await gate;
      return {};
    },
  },
  transport: {
    onRequest: async ({ url }) => {
      if (url.pathname === "/auth/login") return;
      url.protocol = "http:";
      url.pathname = "/events/watch";
    },
  },
  fetch: async () => {
    calls += 1;
    if (calls === 1) {
      return new Response(JSON.stringify({ id: "1" }), {
        status: 200,
        headers: {
          "content-type": "application/json",
          "set-cookie": "session=old; Path=/auth; Secure",
        },
      });
    }
    if (calls === 2) {
      return new Response(JSON.stringify({ id: "1" }), {
        status: 200,
        headers: {
          "content-type": "application/json",
          "set-cookie": "session=new; Path=/",
        },
      });
    }
    protectedFetched = true;
    throw new Error("stale cookie reached fetch");
  },
});
await client.auth.login({ body: { email: "person@example.test" } });
const request = client.events.watch({ query: {}, headers: {} }).then(() => undefined, (cause) => cause);
await entered;
await client.auth.login({ body: { email: "person@example.test" } });
release();
const failure: any = await request;
if (failure?._tag !== "AuthUnavailableError") throw new Error(`unexpected failure ${JSON.stringify(failure)}`);
if (protectedFetched) throw new Error("stale cookie reached fetch");
"#;
    run_bun(&files, harness);
}

#[test]
fn clears_deleted_cookie_credentials_before_auth_fallback() {
    let mut document: pons_ir::Document =
        serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
    document.auth_schemes.insert(
        "session".into(),
        pons_ir::AuthScheme::CookieSession {
            cookie: "session".into(),
        },
    );
    document.operations[0].provides.as_mut().unwrap().value = CredentialLocator::SetCookie {
        name: "session".into(),
    };
    document.operations[1].auth.alternatives = vec![vec!["session".into()], vec!["tenant".into()]];

    let files = emit_with_strategy(&document, EmitMode::Strict, ErrorStrategy::Throw)
        .unwrap()
        .files;
    let harness = r#"
import { createClient } from "./client.ts";

let calls = 0;
let gets = 0;
let clears = 0;
const sets: string[] = [];
const client = createClient({
  baseUrl: "http://example.test",
  auth: {
    session: {
      get: async () => {
        gets += 1;
        return undefined;
      },
      set: async (value: string) => {
        sets.push(value);
      },
      clear: async () => {
        clears += 1;
      },
    },
    tenant: "tenant",
  },
  fetch: async (_url, init) => {
    calls += 1;
    if (calls <= 3) {
      return new Response(JSON.stringify({ id: "1" }), {
        status: 200,
        headers: {
          "content-type": "application/json",
          "set-cookie": calls === 1
            ? "session=live; Path=/"
            : calls === 2
              ? "session=revoked; Path=/; Max-Age=0"
              : "session=expired; Path=/; Expires=Thu, 01 Jan 1970 00:00:00 GMT",
        },
      });
    }
    const headers = new Headers(init?.headers);
    if (headers.get("cookie") !== null) throw new Error("deleted cookie was replayed");
    if (headers.get("X-Tenant") !== "tenant") throw new Error("fallback was not selected");
    return new Response(new ReadableStream({ start(controller) { controller.close(); } }), {
      status: 200,
      headers: {
        "content-type": "text/event-stream",
        "X-Trace-ID": "trace",
      },
    });
  },
});
await client.auth.login({ body: { email: "person@example.test" } });
await client.auth.login({ body: { email: "person@example.test" } });
await client.auth.login({ body: { email: "person@example.test" } });
await client.events.watch({ path: { cursor: "cursor" }, query: {}, headers: {} });
if (sets.join(",") !== "live") throw new Error(`unexpected stored values: ${sets.join(",")}`);
if (clears !== 1) throw new Error(`unexpected clear count: ${clears}`);
    if (gets !== 0) throw new Error(`unexpected source reads: ${gets}`);
    "#;
    run_bun(&files, harness);
}

#[test]
fn expires_captured_cookie_credentials() {
    let mut document: pons_ir::Document =
        serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
    document.auth_schemes.insert(
        "session".into(),
        pons_ir::AuthScheme::CookieSession {
            cookie: "session".into(),
        },
    );
    document.operations[0].provides.as_mut().unwrap().value = CredentialLocator::SetCookie {
        name: "session".into(),
    };
    document.operations[1].auth.alternatives = vec![vec!["session".into()], vec!["tenant".into()]];

    let files = emit_with_strategy(&document, EmitMode::Strict, ErrorStrategy::Throw)
        .unwrap()
        .files;
    let harness = r#"
import { createClient } from "./client.ts";

let calls = 0;
let gets = 0;
let clears = 0;
const sets: string[] = [];
const client = createClient({
  baseUrl: "http://example.test",
  auth: {
    session: {
      get: async () => {
        gets += 1;
        return undefined;
      },
      set: async (value: string) => {
        sets.push(value);
      },
      clear: async () => {
        clears += 1;
      },
    },
    tenant: "tenant",
  },
  fetch: async (_url, init) => {
    calls += 1;
    if (calls === 1) {
      return new Response(JSON.stringify({ id: "1" }), {
        status: 200,
        headers: {
          "content-type": "application/json",
          "set-cookie": "session=live; Max-Age=1",
        },
      });
    }
    const headers = new Headers(init?.headers);
    if (headers.get("cookie") !== null) throw new Error("expired cookie was replayed");
    if (headers.get("X-Tenant") !== "tenant") throw new Error("fallback was not selected");
    return new Response(new ReadableStream({ start(controller) { controller.close(); } }), {
      status: 200,
      headers: {
        "content-type": "text/event-stream",
        "X-Trace-ID": "trace",
      },
    });
  },
});
await client.auth.login({ body: { email: "person@example.test" } });
await new Promise((resolve) => setTimeout(resolve, 1100));
await client.events.watch({ path: { cursor: "cursor" }, query: {}, headers: {} });
if (sets.join(",") !== "live") throw new Error(`unexpected stored values: ${sets.join(",")}`);
if (clears !== 1) throw new Error(`unexpected clear count: ${clears}`);
if (gets !== 1) throw new Error(`unexpected source reads: ${gets}`);
"#;
    run_bun(&files, harness);
}

#[test]
fn falls_back_to_a_later_auth_alternative() {
    let mut document: pons_ir::Document =
        serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
    document
        .auth_schemes
        .insert("empty".into(), pons_ir::AuthScheme::Bearer { format: None });
    document.operations[1].auth.alternatives = vec![vec!["empty".into()], vec!["tenant".into()]];

    let files = emit_with_strategy(&document, EmitMode::Strict, ErrorStrategy::Throw)
        .unwrap()
        .files;
    let harness = r#"
import { createClient } from "./client.ts";

let emptyReads = 0;
const client = createClient({
  baseUrl: "http://example.test",
  auth: {
    empty: () => {
      emptyReads += 1;
      return undefined;
    },
    tenant: "tenant",
  },
  fetch: async (_url, init) => {
    const headers = new Headers(init?.headers);
    if (headers.get("authorization") !== null) throw new Error("empty alternative was attached");
    if (headers.get("X-Tenant") !== "tenant") throw new Error("fallback alternative was not attached");
    return new Response(new ReadableStream({ start(controller) { controller.close(); } }), {
      status: 200,
      headers: {
        "content-type": "text/event-stream",
        "X-Trace-ID": "trace",
      },
    });
  },
});
await client.events.watch({ path: { cursor: "cursor" }, query: {}, headers: {} });
    if (emptyReads !== 1) throw new Error(`unexpected empty source reads: ${emptyReads}`);
    "#;
    run_bun(&files, harness);
}

#[test]
fn falls_back_from_an_empty_cookie_source() {
    let mut document: pons_ir::Document =
        serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
    document.auth_schemes.insert(
        "session".into(),
        pons_ir::AuthScheme::CookieSession {
            cookie: "session".into(),
        },
    );
    document.operations[1].auth.alternatives = vec![vec!["session".into()], vec!["tenant".into()]];

    let files = emit_with_strategy(&document, EmitMode::Strict, ErrorStrategy::Throw)
        .unwrap()
        .files;
    let harness = r#"
import { createClient } from "./client.ts";

const client = createClient({
  baseUrl: "http://example.test",
  auth: {
    session: () => undefined,
    tenant: "tenant",
  },
  fetch: async (_url, init) => {
    const headers = new Headers(init?.headers);
    if (headers.get("cookie") !== null) throw new Error("empty cookie alternative was attached");
    if (headers.get("X-Tenant") !== "tenant") throw new Error("fallback cookie alternative was not selected");
    return new Response(new ReadableStream({ start(controller) { controller.close(); } }), {
      status: 200,
      headers: {
        "content-type": "text/event-stream",
        "X-Trace-ID": "trace",
      },
    });
  },
});
await client.events.watch({ path: { cursor: "cursor" }, query: {}, headers: {} });
"#;
    run_bun(&files, harness);
}

#[test]
fn rejects_cookie_values_that_can_add_cookie_pairs() {
    let mut document: pons_ir::Document =
        serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
    document.auth_schemes.insert(
        "session".into(),
        pons_ir::AuthScheme::CookieSession {
            cookie: "session".into(),
        },
    );
    document.operations[1].auth.alternatives = vec![vec!["session".into()]];

    let files = emit_with_strategy(&document, EmitMode::Strict, ErrorStrategy::Throw)
        .unwrap()
        .files;
    let harness = r#"
import { createClient } from "./client.ts";

let fetched = false;
const client = createClient({
  baseUrl: "http://example.test",
  auth: { session: "token; admin=1" },
  headers: { cookie: "admin=0" },
  fetch: async () => {
    fetched = true;
    throw new Error("request should not be sent");
  },
});
let failure: any;
try {
  await client.events.watch({ path: { cursor: "cursor" }, query: {}, headers: {} });
} catch (cause) {
  failure = cause;
}
if (failure?._tag !== "AuthSourceError") throw new Error(`unexpected failure ${JSON.stringify(failure)}`);
if (fetched) throw new Error("invalid cookie reached fetch");
"#;
    run_bun(&files, harness);
}

#[test]
fn rejects_cookie_names_that_can_add_cookie_pairs() {
    let mut document: pons_ir::Document =
        serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
    document.auth_schemes.insert(
        "session".into(),
        pons_ir::AuthScheme::CookieSession {
            cookie: "session=admin".into(),
        },
    );
    document.operations[1].auth.alternatives = vec![vec!["session".into()]];

    let files = emit_with_strategy(&document, EmitMode::Strict, ErrorStrategy::Throw)
        .unwrap()
        .files;
    let harness = r#"
import { createClient } from "./client.ts";

let fetched = false;
const client = createClient({
  baseUrl: "http://example.test",
  auth: { session: "1" },
  headers: { cookie: "session=real" },
  fetch: async () => {
    fetched = true;
    throw new Error("request should not be sent");
  },
});
let failure: any;
try {
  await client.events.watch({ path: { cursor: "cursor" }, query: {}, headers: {} });
} catch (cause) {
  failure = cause;
}
if (failure?._tag !== "AuthSourceError") throw new Error(`unexpected failure ${JSON.stringify(failure)}`);
if (fetched) throw new Error("invalid cookie name reached fetch");
"#;
    run_bun(&files, harness);
}

#[test]
fn rejects_inherited_object_properties_as_credentials() {
    let mut document: pons_ir::Document =
        serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
    document.auth_schemes.insert(
        "constructor".into(),
        pons_ir::AuthScheme::Bearer { format: None },
    );
    document.operations[1].auth.alternatives = vec![vec!["constructor".into()]];

    let files = emit_with_strategy(&document, EmitMode::Strict, ErrorStrategy::Throw)
        .unwrap()
        .files;
    let harness = r#"
import { createClient } from "./client.ts";

let fetched = false;
const client = createClient({
  baseUrl: "http://example.test",
  fetch: async () => {
    fetched = true;
    throw new Error("request should not be sent");
  },
});
let failure: any;
try {
  await client.events.watch({ path: { cursor: "cursor" }, query: {}, headers: {} });
} catch (cause) {
  failure = cause;
}
if (failure?._tag !== "AuthUnavailableError") throw new Error(`unexpected failure ${JSON.stringify(failure)}`);
if (fetched) throw new Error("missing credential reached fetch");
"#;
    run_bun(&files, harness);
}

#[test]
fn does_not_inherit_custom_auth_scopes() {
    let mut document: pons_ir::Document =
        serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
    document.auth_schemes.insert(
        "toString".into(),
        pons_ir::AuthScheme::Custom {
            description: "scope probe".into(),
            adapter: None,
            ext: Default::default(),
        },
    );
    document.operations[1].auth.alternatives = vec![vec!["toString".into()]];

    let files = emit_with_strategy(&document, EmitMode::Strict, ErrorStrategy::Throw)
        .unwrap()
        .files;
    let harness = r#"
import { createClient } from "./client.ts";

const client = createClient({
  baseUrl: "https://example.test",
  authAdapters: {
    toString: ({ scopes }) => {
      if (scopes.length !== 0) throw new Error(`inherited scopes: ${scopes.join(",")}`);
      return {};
    },
  },
  fetch: async () => new Response(new ReadableStream({ start(controller) { controller.close(); } }), {
    status: 200,
    headers: {
      "content-type": "text/event-stream",
      "X-Trace-ID": "trace",
    },
  }),
});
await client.events.watch({ path: { cursor: "cursor" }, query: {}, headers: {} });
"#;
    run_bun(&files, harness);
}

#[test]
fn a_custom_scheme_is_served_by_the_adapter_id_it_declares() {
    let mut document: pons_ir::Document =
        serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
    document.auth_schemes.insert(
        "tenant".into(),
        pons_ir::AuthScheme::Custom {
            description: "tenant adapter".into(),
            adapter: Some("tenantSignature".into()),
            ext: Default::default(),
        },
    );
    document.operations[1].auth.alternatives = vec![vec!["tenant".into()]];

    let files = emit_with_strategy(&document, EmitMode::Strict, ErrorStrategy::Throw)
        .unwrap()
        .files;
    let client = files.iter().find(|file| file.path == "client.ts").unwrap();
    assert!(
        client.content.contains(
            "authSchemes[\"tenant\"] = { kind: \"custom\", adapter: \"tenantSignature\" };"
        ),
        "{}",
        client.content
    );
    let harness = r#"
import { createClient } from "./client.ts";

const stream = () => new Response(new ReadableStream({ start(controller) { controller.close(); } }), {
  status: 200,
  headers: { "content-type": "text/event-stream", "X-Trace-ID": "trace" },
});

const byScheme = createClient({
  baseUrl: "https://example.test",
  authAdapters: { tenant: () => ({ headers: { "x-tenant": "signed" } }) },
  fetch: async () => stream(),
});
let failure: any;
try {
  await byScheme.events.watch({ path: { cursor: "cursor" }, query: {}, headers: {} });
} catch (cause) {
  failure = cause;
}
if (failure?._tag !== "AuthUnavailableError") throw new Error(`the scheme id served the adapter: ${JSON.stringify(failure)}`);

const declared: string[] = [];
const byAdapter = createClient({
  baseUrl: "https://example.test",
  authAdapters: {
    tenantSignature: ({ scheme }) => {
      declared.push(scheme);
      return { headers: { "x-tenant": "signed" } };
    },
  },
  fetch: async (_url, init) => {
    const headers = new Headers(init?.headers);
    if (headers.get("x-tenant") !== "signed") throw new Error("the declared adapter did not attach its header");
    return stream();
  },
});
await byAdapter.events.watch({ path: { cursor: "cursor" }, query: {}, headers: {} });
if (declared.join(",") !== "tenant") throw new Error(`adapter context named ${declared.join(",")}`);
"#;
    run_bun(&files, harness);
}

#[test]
fn preserves_proto_auth_scheme_and_custom_adapter_scopes() {
    let mut document: pons_ir::Document =
        serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
    document.auth_schemes.insert(
        "__proto__".into(),
        pons_ir::AuthScheme::Custom {
            description: "prototype adapter".into(),
            adapter: None,
            ext: Default::default(),
        },
    );
    let operation = &mut document.operations[1];
    operation.auth.alternatives = vec![vec!["__proto__".into()]];
    operation
        .auth
        .scopes
        .insert("__proto__".into(), vec!["events:read".into()]);

    let files = emit_with_strategy(&document, EmitMode::Strict, ErrorStrategy::Throw)
        .unwrap()
        .files;
    let harness = r#"
import { createClient } from "./client.ts";

const client = createClient({
  baseUrl: "https://example.test",
  authAdapters: {
    ["__proto__"]: ({ scheme, scopes }) => {
      if (scheme !== "__proto__" || scopes.join(",") !== "events:read") {
        throw new Error(`wrong prototype auth context: ${scheme}/${scopes.join(",")}`);
      }
      return { headers: { "X-Auth": "present" } };
    },
  },
  fetch: async (_url, init) => {
    if (new Headers(init?.headers).get("X-Auth") !== "present") {
      throw new Error("prototype auth adapter was not called");
    }
    return new Response(new ReadableStream({ start(controller) { controller.close(); } }), {
      status: 200,
      headers: {
        "content-type": "text/event-stream",
        "X-Trace-ID": "trace",
      },
    });
  },
});
await client.events.watch({ path: { cursor: "cursor" }, query: {}, headers: {} });
"#;
    run_bun(&files, harness);
}

#[test]
fn revalidates_custom_auth_origin_after_transport_request_hooks() {
    let mut document: pons_ir::Document =
        serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
    document.auth_schemes.insert(
        "adapter".into(),
        pons_ir::AuthScheme::Custom {
            description: "origin probe".into(),
            adapter: None,
            ext: Default::default(),
        },
    );
    document.operations[1].auth.alternatives = vec![vec!["adapter".into()]];

    let files = emit_with_strategy(&document, EmitMode::Strict, ErrorStrategy::Throw)
        .unwrap()
        .files;
    let harness = r#"
import { createClient } from "./client.ts";

let fetched = false;
const client = createClient({
  baseUrl: "https://example.test",
  authAdapters: {
    adapter: async () => ({ headers: { "X-Secret": "secret" } }),
  },
  transport: {
    onRequest: async ({ url }) => {
      if (url.pathname === "/auth/login") return;
      url.hostname = "idp.test";
    },
  },
  fetch: async () => {
    fetched = true;
    throw new Error("custom credential reached a rewritten host");
  },
});
let failure: any;
try {
  await client.events.watch({ path: { cursor: "cursor" }, query: {}, headers: {} });
} catch (cause) {
  failure = cause;
}
if (failure?._tag !== "AuthUnavailableError") throw new Error(`unexpected failure ${JSON.stringify(failure)}`);
if (fetched) throw new Error("custom credential reached a rewritten host");
"#;
    run_bun(&files, harness);
}

#[test]
fn returns_non_stream_success_values_for_stream_operations() {
    let mut raw: serde_json::Value =
        serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
    raw["operations"][1]["responses"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::json!({
            "status": 201,
            "content": "json",
            "body": { "kind": "string" }
        }));
    let document: pons_ir::Document = serde_json::from_value(raw).unwrap();
    let files = emit_with_strategy(&document, EmitMode::Strict, ErrorStrategy::Throw)
        .unwrap()
        .files;
    assert!(
        files
            .iter()
            .find(|file| file.path == "operations/events.ts")
            .unwrap()
            .content
            .contains("| string")
    );
    let harness = r#"
import { createClient } from "./client.ts";

const client = createClient({
  baseUrl: "https://example.test",
  auth: { tenant: "tenant" },
  fetch: async () => new Response(JSON.stringify("json"), {
    status: 201,
    headers: { "content-type": "application/json" },
  }),
});
const value = await client.events.watch({ path: { cursor: "cursor" }, query: {}, headers: {} });
if (value !== "json") throw new Error(`non-stream success was not returned: ${String(value)}`);
"#;
    run_bun(&files, harness);
}

#[test]
fn validates_text_response_constraints() {
    let mut raw: serde_json::Value =
        serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
    raw["operations"][0]["responses"][0] = serde_json::json!({
        "status": 200,
        "content": "text",
        "body": { "kind": "string", "min_length": 5, "max_length": 5 }
    });
    raw["operations"][0]["provides"] = serde_json::Value::Null;
    let document: pons_ir::Document = serde_json::from_value(raw).unwrap();
    let files = emit_with_strategy(&document, EmitMode::Strict, ErrorStrategy::Throw)
        .unwrap()
        .files;
    let harness = r#"
import { createClient } from "./client.ts";

const client = createClient({
  baseUrl: "https://example.test",
  fetch: async () => new Response("x", { status: 200, headers: { "content-type": "text/plain" } }),
});
let failure: any;
try {
  await client.auth.login({ body: { email: "person@example.test" } });
} catch (cause) {
  failure = cause;
}
if (failure?._tag !== "DecodeError") throw new Error(`unexpected failure ${JSON.stringify(failure)}`);
"#;
    run_bun(&files, harness);
}

#[test]
fn preserves_floats_beside_wide_integers() {
    let mut raw: serde_json::Value =
        serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
    raw["types"]["FloatBox"] = serde_json::json!({
        "kind": "struct",
        "name": ["float", "box"],
        "direction": "output",
        "openness": "reject",
        "fields": [
            { "wire": "id", "ident": ["id"], "ty": { "kind": "int", "bits": 64, "signed": false, "wire": "number" }, "presence": "required" },
            { "wire": "ratio", "ident": ["ratio"], "ty": { "kind": "float", "bits": 64 }, "presence": "required" }
        ]
    });
    raw["operations"][0]["responses"][0]["body"] = serde_json::json!({ "ref": "FloatBox" });
    raw["operations"][0]["provides"] = serde_json::Value::Null;
    let document: pons_ir::Document = serde_json::from_value(raw).unwrap();
    let files = emit_with_strategy(&document, EmitMode::Strict, ErrorStrategy::Throw)
        .unwrap()
        .files;
    let harness = r#"
import { createClient } from "./client.ts";

const client = createClient({
  baseUrl: "https://example.test",
  fetch: async () => new Response('{"id":9007199254740993,"ratio":9007199254740993}', {
    status: 200,
    headers: { "content-type": "application/json" },
  }),
});
const value = await client.auth.login({ body: { email: "person@example.test" } });
if (value.id !== 9007199254740993n || typeof value.ratio !== "number" || !Number.isFinite(value.ratio)) {
  throw new Error(`wrong numeric values: ${String(value.id)}/${String(value.ratio)}`);
}
"#;
    run_bun(&files, harness);
}

#[test]
fn normalizes_safe_wide_integers_in_responses_and_streams() {
    let document = wide_number_document();
    let files = emit_with_strategy(&document, EmitMode::Strict, ErrorStrategy::Throw)
        .unwrap()
        .files;
    let harness = r#"
import { createClient } from "./client.ts";

const client = createClient({
  baseUrl: "http://example.test",
  fetch: async () => new Response('{"id":42}', { status: 200, headers: { "content-type": "application/json" } }),
});
const value = await client.auth.login({ body: { email: "person@example.test" } });
if (value.id !== 42n) throw new Error("safe response number was not normalized");

const streamBody = new ReadableStream({
  start(controller) {
    controller.enqueue(new TextEncoder().encode('event: user\ndata: {"id":42}\n\n'));
    controller.close();
  },
});
const streamClient = createClient({
  baseUrl: "http://example.test",
  auth: { tenant: "tenant" },
  fetch: async () => new Response(streamBody, { status: 200, headers: { "content-type": "text/event-stream", "X-Trace-ID": "trace" } }),
});
const stream = await streamClient.events.watch({ path: { cursor: "cursor" }, query: {}, headers: {} });
let seen;
for await (const item of stream.value) {
  if (item.isErr()) throw new Error("stream item failed");
  seen = item.value.data.id;
  break;
}
if (seen !== 42n) throw new Error("safe stream number was not normalized");
"#;
    run_bun(&files, harness);
}

#[test]
fn lossy_mode_accepts_unsafe_wide_integer_requests() {
    let mut document = wide_number_document();
    let wide = document
        .types
        .get("User")
        .and_then(|definition| match &definition.definition {
            TypeKind::Struct { fields, .. } => fields.first().map(|field| field.ty.clone()),
            _ => None,
        })
        .unwrap();
    let login = document.types.get_mut("LoginRequest").unwrap();
    let TypeKind::Struct { fields, .. } = &mut login.definition else {
        unreachable!()
    };
    fields[0].ty = wide;
    let files = emit_with_strategy(&document, EmitMode::Lossy, ErrorStrategy::Throw)
        .unwrap()
        .files;
    let harness = r#"
import { createClient } from "./client.ts";

const unsafeNumber = 9007199254740992;
const client = createClient({
  baseUrl: "http://example.test",
  fetch: async (_url, init) => {
    const body = JSON.parse(String(init?.body));
    if (body.email !== unsafeNumber) throw new Error(`wrong request integer: ${body.email}`);
    return new Response('{"id":9007199254740992}', {
      status: 200,
      headers: { "content-type": "application/json" },
    });
  },
});
const value = await client.auth.login({ body: { email: unsafeNumber } });
if (value.id !== unsafeNumber) throw new Error(`wrong response integer: ${value.id}`);
"#;
    run_bun(&files, harness);

    let files = emit_with_strategy(&document, EmitMode::Strict, ErrorStrategy::Throw)
        .unwrap()
        .files;
    let harness = r#"
import { createClient } from "./client.ts";

let fetched = false;
const client = createClient({
  baseUrl: "http://example.test",
  fetch: async () => {
    fetched = true;
    throw new Error("strict request should not be sent");
  },
});
let failure: any;
try {
  await client.auth.login({ body: { email: 9007199254740992 as any } });
} catch (cause) {
  failure = cause;
}
if (!String(failure?.message).includes("expected an exact integer")) {
  throw new Error(`unexpected strict failure: ${String(failure)}`);
}
if (fetched) throw new Error("unsafe strict request reached fetch");
"#;
    run_bun(&files, harness);
}

#[test]
fn lossy_mode_accepts_rounded_wide_integers() {
    let document = wide_number_document();
    let files = emit_with_strategy(&document, EmitMode::Lossy, ErrorStrategy::Throw)
        .unwrap()
        .files;
    let harness = r#"
import { createClient } from "./client.ts";

const client = createClient({
  baseUrl: "http://example.test",
  fetch: async () => new Response('{"id":9007199254740993}', {
    status: 200,
    headers: { "content-type": "application/json" },
  }),
});
const value = await client.auth.login({ body: { email: "person@example.test" } });
if (value.id !== 9007199254740992) throw new Error(`wide integer was not rounded: ${value.id}`);
"#;
    run_bun(&files, harness);
}

#[test]
fn generated_client_validates_required_fields_by_direction_and_access() {
    let mut raw: serde_json::Value =
        serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
    raw["types"]["LoginRequest"] = serde_json::json!({
        "kind": "struct",
        "name": ["login", "request"],
        "direction": "input",
        "openness": "reject",
        "fields": [
            {"wire": "email", "ident": ["email"], "ty": {"kind": "string"}, "presence": "required"},
            {"wire": "secret", "ident": ["secret"], "ty": {"kind": "string"}, "presence": "required", "access": "write_only", "default": "sent"},
            {"wire": "server_id", "ident": ["server", "id"], "ty": {"kind": "string"}, "presence": "required", "access": "read_only", "default": "filled"}
        ]
    });
    raw["types"]["User"] = serde_json::json!({
        "kind": "struct",
        "name": ["user"],
        "direction": "output",
        "openness": "reject",
        "fields": [
            {"wire": "id", "ident": ["id"], "ty": {"kind": "string"}, "presence": "required", "access": "read_only", "default": "filled"},
            {"wire": "secret", "ident": ["secret"], "ty": {"kind": "string"}, "presence": "required", "access": "write_only", "default": "hidden"},
            {"wire": "value", "ident": ["value"], "ty": {"kind": "string"}, "presence": "required"}
        ]
    });
    let directional: pons_ir::Document = serde_json::from_value(raw).unwrap();
    let mut both = directional.clone();
    both.types.get_mut("LoginRequest").unwrap().direction = Direction::Both;
    both.types.get_mut("User").unwrap().direction = Direction::Both;
    let harness = r#"
import { createClient } from "./client.ts";

let response: any = { id: "1", value: "ok" };
let fetched = 0;
let sent: any;
const client = createClient({
  baseUrl: "http://example.test",
  fetch: async (_url, init) => {
    fetched += 1;
    sent = JSON.parse(String(init?.body));
    return new Response(JSON.stringify(response), {
      status: 200,
      headers: { "content-type": "application/json" },
    });
  },
});
const expectFailure = async (operation, expected) => {
  try {
    await operation();
    throw new Error("operation was accepted");
  } catch (cause) {
    const text = JSON.stringify(cause);
    if (!text.includes(expected)) throw new Error(text);
  }
};

await client.auth.login({ body: { email: "person@example.test", secret: "write-only" } });
if (sent.server_id !== undefined) throw new Error("read-only request field was sent");
await expectFailure(
  () => client.auth.login({ body: { email: "person@example.test" } }),
  "missing required field secret",
);
if (fetched !== 1) throw new Error("missing write-only field reached fetch");

const value = await client.auth.login({ body: { email: "person@example.test", secret: "write-only" } });
if (value.id !== "1" || value.value !== "ok" || (value as any).secret !== undefined) {
  throw new Error(`wrong output fields: ${JSON.stringify(value)}`);
}
response = { value: "ok" };
await expectFailure(
  () => client.auth.login({ body: { email: "person@example.test", secret: "write-only" } }),
  "missing required field id",
);
if (fetched !== 3) throw new Error("invalid read-only response was not fetched");
"#;
    for document in [directional, both] {
        let files = emit_with_strategy(&document, EmitMode::Strict, ErrorStrategy::Throw)
            .unwrap()
            .files;
        run_bun(&files, harness);
    }
}

#[test]
fn generated_client_validates_flattened_fields_with_typed_openness() {
    let mut raw: serde_json::Value =
        serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
    raw["types"]["Metadata"] = serde_json::json!({
        "kind": "struct",
        "name": ["metadata"],
        "direction": "both",
        "openness": "reject",
        "fields": [
            {"wire": "flat", "ident": ["flat"], "ty": {"kind": "string"}, "presence": "required"}
        ]
    });
    raw["types"]["QueryMeta"] = serde_json::json!({
        "kind": "struct",
        "name": ["query", "meta"],
        "direction": "both",
        "openness": "reject",
        "fields": [
            {"wire": "extra", "ident": ["extra"], "ty": {"kind": "string"}, "presence": "required"}
        ]
    });
    for id in ["User", "EventQuery"] {
        raw["types"][id]["direction"] = serde_json::json!("both");
        raw["types"][id]["fields"] = serde_json::json!([
            {"wire": "own", "ident": ["own"], "ty": {"kind": "string"}, "presence": "required"}
        ]);
        raw["types"][id]["flattened"] =
            serde_json::json!([{"ref": if id == "User" { "Metadata" } else { "QueryMeta" } }]);
    }

    raw["types"]["User"]["openness"] = serde_json::json!({
        "preserve_typed": {"value": {"kind": "string"}}
    });
    raw["types"]["EventQuery"]["openness"] = serde_json::json!("reject");
    raw["types"]["EventQuery"]["fields"] = serde_json::json!([
        {"wire": "own", "ident": ["own"], "ty": {"kind": "string"}, "presence": "required"},
        {"wire": "flat", "ident": ["flat"], "ty": {"kind": "string"}, "presence": "required"}
    ]);
    let document: pons_ir::Document = serde_json::from_value(raw).unwrap();
    let files = emit_with_strategy(&document, EmitMode::Strict, ErrorStrategy::Throw)
        .unwrap()
        .files;
    let harness = r#"
import { createClient } from "./client.ts";

let response: any = { own: "outer", flat: "metadata", extra: "typed" };
let fetched = 0;
const client = createClient({
  baseUrl: "http://example.test",
  auth: { tenant: "tenant" },
  fetch: async (url) => {
    fetched += 1;
    if (url.pathname === "/auth/login") {
      return new Response(JSON.stringify(response), {
        status: 200,
        headers: { "content-type": "application/json" },
      });
    }
    return new Response(new ReadableStream({ start(controller) { controller.close(); } }), {
      status: 200,
      headers: { "content-type": "text/event-stream", "X-Trace-ID": "trace" },
    });
  },
});
const expectFailure = async (operation, expected) => {
  try {
    await operation();
    throw new Error("operation was accepted");
  } catch (cause) {
    const text = JSON.stringify(cause);
    if (!text.includes(expected)) throw new Error(text);
  }
};

const value = await client.auth.login({ body: { email: "person@example.test" } });
if (value.own !== "outer" || value.flat !== "metadata" || value.extra !== "typed") {
  throw new Error(`wrong flattened response: ${JSON.stringify(value)}`);
}
response = { own: "outer", flat: "metadata", extra: 1 };
await expectFailure(
  () => client.auth.login({ body: { email: "person@example.test" } }),
  "expected string",
);
if (fetched !== 2) throw new Error("invalid typed response was not fetched");

await client.events.watch({
  path: { cursor: "cursor" },
  query: { own: "outer", flat: "metadata", extra: "typed" },
  headers: {},
});
await expectFailure(
  () => client.events.watch({ path: { cursor: "cursor" }, query: { own: "outer", extra: "typed" }, headers: {} }),
  "missing required field flat",
);
await expectFailure(
  () => client.events.watch({ path: { cursor: "cursor" }, query: { own: "outer", flat: "metadata", extra: 1 }, headers: {} }),
  "expected string",
);
if (fetched !== 3) throw new Error("invalid flattened queries reached fetch");
"#;
    run_bun(&files, harness);
}

#[test]
fn generated_client_types_extra_properties_with_the_map_value() {
    let mut base: serde_json::Value =
        serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
    let wide = serde_json::json!({"kind": "int", "bits": 64, "signed": false, "wire": "number"});
    base["types"]["User"]["fields"] =
        serde_json::json!([{"wire": "id", "ident": ["id"], "ty": wide, "presence": "required"}]);
    base["types"]["Counts"] = serde_json::json!({
        "kind": "newtype",
        "name": ["counts"],
        "direction": "both",
        "inner": {"kind": "map", "key": {"kind": "string"}, "value": wide}
    });
    let harness = r#"
import { createClient } from "./client.ts";

let response = `{"id":9007199254740993,"extra":9007199254740995}`;
let fetched = 0;
const client = createClient({
  baseUrl: "http://example.test",
  fetch: async () => {
    fetched += 1;
    return new Response(response, { status: 200, headers: { "content-type": "application/json" } });
  },
});
const value = await client.auth.login({ body: { email: "person@example.test" } });
if (value.id !== 9007199254740993n) throw new Error("declared field lost its wide value");
if ((value as any).extra !== 9007199254740995n) throw new Error("extra property ignored the map value");

response = `{"id":1,"extra":"text"}`;
try {
  await client.auth.login({ body: { email: "person@example.test" } });
  throw new Error("an extra property outside the map value was accepted");
} catch (cause) {
  const text = JSON.stringify(cause);
  if (!text.includes("expected an exact integer")) throw new Error(text);
}
if (fetched !== 2) throw new Error("invalid extra property did not reach fetch");
"#;

    for openness in [
        serde_json::json!({
            "preserve_typed": {
                "value": {"kind": "map", "key": {"kind": "string"}, "value": wide}
            }
        }),
        serde_json::json!({"preserve_typed": {"value": {"ref": "Counts"}}}),
    ] {
        let mut raw = base.clone();
        raw["types"]["User"]["openness"] = openness;
        let document: pons_ir::Document = serde_json::from_value(raw).unwrap();
        let files = emit_with_strategy(&document, EmitMode::Strict, ErrorStrategy::Throw)
            .unwrap()
            .files;
        run_bun(&files, harness);
    }
}

#[test]
fn generated_client_preflights_request_inputs() {
    let optional = request_input_document(false);
    let optional_files = emit_with_strategy(&optional, EmitMode::Strict, ErrorStrategy::Throw)
        .unwrap()
        .files;
    let optional_client = optional_files
        .iter()
        .find(|file| file.path == "runtime/transport.ts")
        .unwrap();
    assert!(optional_client.content.contains("preflightPath"));
    assert!(
        optional_client
            .content
            .contains("missing required request body")
    );
    let preflight = optional_client
        .content
        .find("const encodedPathValues")
        .unwrap();
    let url = optional_client.content.find("const url = new URL").unwrap();
    let fetch = optional_client
        .content
        .find("const response = await fetchWithRetry")
        .unwrap();
    assert!(preflight < url && url < fetch);

    let optional_harness = r#"
import { createClient } from "./client.ts";

const response = () => new Response("", { status: 200, headers: { "content-type": "text/event-stream", "X-Trace-ID": "trace" } });
const expectFailure = async (operation, expected) => {
  try {
    await operation();
    throw new Error("operation was accepted");
  } catch (cause) {
    const text = JSON.stringify(cause);
    if (!text.includes(expected)) throw new Error(text);
  }
};
let sent = 0;
const client = createClient({
  baseUrl: "http://example.test",
  auth: { tenant: "tenant" },
  fetch: async () => {
    sent += 1;
    return response();
  },
});
await client.events.watch({ path: { cursor: 250 }, query: {}, headers: {} });
if (sent !== 1) throw new Error("valid request was not sent");
await expectFailure(() => client.events.watch({ path: { cursor: 249 }, query: {}, headers: {} }), "does not match its multiple");
if (sent !== 1) throw new Error("invalid path was sent");
await expectFailure(() => client.events.watch({ path: { cursor: 250 }, query: { limit: 249 }, headers: {} }), "does not match its multiple");
if (sent !== 1) throw new Error("invalid query was sent");
await expectFailure(() => client.events.watch({ path: { cursor: 250 }, query: {}, headers: { "X-Request-ID": 249 } }), "does not match its multiple");
if (sent !== 1) throw new Error("invalid header was sent");
await expectFailure(() => client.auth.login({}), "missing required request body");
if (sent !== 1) throw new Error("missing body was sent");
await client.events.watch({ path: { cursor: 250 }, query: { limit: undefined }, headers: {} });
if (sent !== 2) throw new Error("optional undefined was rejected");
"#;
    run_bun(&optional_files, optional_harness);

    let required = request_input_document(true);
    let required_files = emit_with_strategy(&required, EmitMode::Strict, ErrorStrategy::Throw)
        .unwrap()
        .files;
    let required_harness = r#"
import { createClient } from "./client.ts";

const response = () => new Response("", { status: 200, headers: { "content-type": "text/event-stream", "X-Trace-ID": "trace" } });
const expectFailure = async (operation, expected) => {
  try {
    await operation();
    throw new Error("operation was accepted");
  } catch (cause) {
    const text = JSON.stringify(cause);
    if (!text.includes(expected)) throw new Error(text);
  }
};
let sent = 0;
const client = createClient({
  baseUrl: "http://example.test",
  auth: { tenant: "tenant" },
  fetch: async () => {
    sent += 1;
    return response();
  },
});
await client.events.watch({ path: { cursor: 250 }, query: { limit: 250 }, headers: { "X-Request-ID": 250 } });
if (sent !== 1) throw new Error("valid required request was not sent");
await expectFailure(() => client.events.watch({ path: { cursor: 250 }, query: {}, headers: { "X-Request-ID": 250 } }), "missing required field limit");
if (sent !== 1) throw new Error("missing query was sent");
await expectFailure(() => client.events.watch({ path: { cursor: 250 }, query: { limit: 250 }, headers: {} }), "missing required request header X-Request-ID");
if (sent !== 1) throw new Error("missing header was sent");
await expectFailure(() => client.events.watch({ path: {}, query: { limit: 250 }, headers: { "X-Request-ID": 250 } }), "missing required path parameter cursor");
if (sent !== 1) throw new Error("missing path was sent");
"#;
    run_bun(&required_files, required_harness);
}

#[test]
fn rejects_multipart_media_type_overrides() {
    let mut document: pons_ir::Document =
        serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
    let body = document.operations[0].body.as_mut().unwrap();
    body.content = ContentKind::Multipart;
    body.media = Some(pons_ir::MediaDescriptor::from_declared("multipart/mixed"));
    let error = emit_with_strategy(&document, EmitMode::Strict, ErrorStrategy::Throw).unwrap_err();
    assert!(error.0.contains("multipart media type overrides"));
}

#[test]
fn resets_sse_field_limit_between_events() {
    let document: pons_ir::Document =
        serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
    let files = emit_with_strategy(&document, EmitMode::Strict, ErrorStrategy::Throw)
        .unwrap()
        .files;
    let harness = r#"
import { createClient } from "./client.ts";

const comments = ":x\n".repeat(50_001);
const event = `event: user\ndata: {"id":"1"}\n${comments}\n`;
const body = new ReadableStream({
  start(controller) {
    controller.enqueue(new TextEncoder().encode(event + event));
    controller.close();
  },
});
const client = createClient({
  baseUrl: "https://example.test",
  auth: { tenant: "tenant" },
  fetch: async () => new Response(body, {
    status: 200,
    headers: { "content-type": "text/event-stream", "X-Trace-ID": "trace" },
  }),
});
const stream = await client.events.watch({ path: { cursor: "cursor" }, query: {}, headers: {} });
let count = 0;
for await (const item of stream.value) {
  if (item.isErr()) throw item.error;
  count += 1;
}
if (count !== 2) throw new Error(`expected 2 events, received ${count}`);
"#;
    run_bun(&files, harness);
}

#[test]
fn accepts_bare_json_when_contract_media_type_has_parameters() {
    let mut document: pons_ir::Document =
        serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
    document.operations[0].responses[0].media = Some(pons_ir::MediaDescriptor::from_declared(
        "application/json; charset=utf-8",
    ));
    let files = emit_with_strategy(&document, EmitMode::Strict, ErrorStrategy::Throw)
        .unwrap()
        .files;
    let harness = r#"
import { createClient } from "./client.ts";

const client = createClient({
  baseUrl: "https://example.test",
  fetch: async () => new Response(JSON.stringify({ id: "1" }), {
    status: 200,
    headers: { "content-type": "application/json" },
  }),
});
const value = await client.auth.login({ body: { email: "person@example.test" } });
if (value.id !== "1") throw new Error("bare JSON response was not decoded");
"#;
    run_bun(&files, harness);
}

#[test]
fn byte_response_media_matching_remains_strict() {
    let mut document: pons_ir::Document =
        serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
    let response = &mut document.operations[0].responses[0];
    response.content = ContentKind::Bytes;
    response.body = Some(TypeRef::Inline(InlineType::Bytes {
        encoding: ByteEncoding::Base64,
    }));
    response.media = Some("application/pdf; version=1".into());
    let files = emit_with_strategy(&document, EmitMode::Strict, ErrorStrategy::Throw)
        .unwrap()
        .files;
    let harness = r#"
import { createClient } from "./client.ts";

const client = createClient({
  baseUrl: "https://example.test",
  fetch: async () => new Response(new Uint8Array([1]), {
    status: 200,
    headers: { "content-type": "application/pdf" },
  }),
});
let failure: any;
try {
  await client.auth.login({ body: { email: "person@example.test" } });
} catch (cause) {
  failure = cause;
}
if (failure?._tag !== "DecodeError") throw new Error(`unexpected failure ${JSON.stringify(failure)}`);
if (!String(failure.message).includes("version=1")) throw new Error("strict media mismatch was lost");
"#;
    run_bun(&files, harness);
}

#[test]
fn a_bytes_response_runs_its_validator_on_the_read_bytes() {
    let document: pons_ir::Document =
        serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
    let files = emit_with_strategy(&document, EmitMode::Strict, ErrorStrategy::Throw)
        .unwrap()
        .files;
    let transport = files
        .iter()
        .find(|file| file.path == "runtime/transport.ts")
        .unwrap();
    assert!(transport.content.contains("responseKind !== \"bytes\""));
    assert!(transport.content.contains("responseKind === \"bytes\""));

    let harness = r#"
import { request } from "./runtime/transport.ts";

const client = () => ({
  baseUrl: "https://example.test",
  auth: {},
  authAdapters: {},
  authRecords: Object.create(null),
  authInvalidated: Object.create(null),
  authSchemes: {},
  opaque: { required: false },
  fetch: async () => new Response(new Uint8Array([1, 2, 3]), {
    status: 200,
    headers: { "content-type": "application/octet-stream" },
  }),
});
const spec = (validate: any) => ({
  method: "GET",
  path: "/bytes",
  inputs: { path: [], headers: [], bodyRequired: false },
  responses: [{ status: 200, content: "bytes", schema: { kind: "bytes", encoding: "base64" }, validate }],
  errors: [],
});

let seen: unknown;
const accepted = await request(client(), spec((value: unknown) => {
  seen = value;
  return null;
}));
if (accepted.isErr()) throw new Error(`bytes response was rejected: ${JSON.stringify(accepted.error)}`);
if (!(seen instanceof Uint8Array) || seen.length !== 3) {
  throw new Error("the bytes validator did not receive the read bytes");
}

const rejected = await request(client(), spec(() => ({ path: ["body"], message: "expected bytes" })));
if (!rejected.isErr()) throw new Error("a bytes validator issue was ignored");
if (rejected.error._tag !== "DecodeError") throw new Error(`unexpected failure ${JSON.stringify(rejected.error)}`);
if (!String(rejected.error.message).includes("body: expected bytes")) {
  throw new Error("the bytes validator issue was lost");
}

for (const malformed of [undefined, "not an issue", { message: "missing path" }, { path: [], message: 42 }]) {
  const result = await request(client(), spec(() => malformed));
  if (!result.isErr() || result.error._tag !== "DecodeError") {
    throw new Error(`a malformed validator result escaped: ${JSON.stringify(result)}`);
  }
  if (!String(result.error.message).includes("invalid issue")) {
    throw new Error("a malformed validator result was not a protocol failure");
  }
}

const thrown = await request(client(), spec(() => { throw new Error("boom"); }));
if (!thrown.isErr() || thrown.error._tag !== "DecodeError") {
  throw new Error(`a thrown validator became ${JSON.stringify(thrown)}`);
}
if (!String(thrown.error.message).includes("validator threw")) {
  throw new Error("a thrown validator was not reported as a contract failure");
}
"#;
    run_bun(&files, harness);
}

#[test]
fn maps_unsupported_response_charset_to_protocol_failure() {
    let mut document: pons_ir::Document =
        serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
    let response = &mut document.operations[0].responses[0];
    response.content = ContentKind::Text;
    response.body = Some(TypeRef::Inline(InlineType::String {
        format: None,
        min_length: None,
        max_length: None,
        pattern: None,
    }));
    document.operations[0].provides = None;
    let files = emit_with_strategy(&document, EmitMode::Strict, ErrorStrategy::Throw)
        .unwrap()
        .files;
    let harness = r#"
import { createClient } from "./client.ts";

const client = createClient({
  baseUrl: "https://example.test",
  fetch: async () => new Response("value", {
    status: 200,
    headers: { "content-type": "text/plain; charset=x-unsupported-pons" },
  }),
});
let failure: any;
try {
  await client.auth.login({ body: { email: "person@example.test" } });
} catch (cause) {
  failure = cause;
}
if (failure?._tag !== "DecodeError") throw new Error(`unexpected failure ${JSON.stringify(failure)}`);
if (!String(failure.message).includes("unsupported response charset")) {
  throw new Error(`charset error was lost: ${String(failure.message)}`);
}
"#;
    run_bun(&files, harness);
}

#[test]
fn generated_client_transforms_untagged_byte_payloads() {
    let document = untagged_bytes_document();
    let files = emit_with_strategy(&document, EmitMode::Strict, ErrorStrategy::Throw)
        .unwrap()
        .files;
    let wire = files
        .iter()
        .find(|file| file.path == "runtime/wire.ts")
        .unwrap();
    assert!(wire.content.contains("selectUntaggedVariant"));

    let harness = r#"
import { createClient } from "./client.ts";

let sentBody;
const client = createClient({
  baseUrl: "http://example.test",
  fetch: async (_url, init) => {
    sentBody = init.body;
    return new Response(JSON.stringify({ id: "AQI=" }), { status: 200, headers: { "content-type": "application/json" } });
  },
});
const value = await client.auth.login({ body: { email: new Uint8Array([1, 2]) } });
if (sentBody !== JSON.stringify({ email: "AQI=" })) throw new Error(`unexpected body: ${sentBody}`);
if (!(value.id instanceof Uint8Array) || value.id.length !== 2 || value.id[0] !== 1 || value.id[1] !== 2) {
  throw new Error("untagged bytes were not decoded");
}

let invalidSent = 0;
const invalid = createClient({
  baseUrl: "http://example.test",
  fetch: async () => {
    invalidSent += 1;
    throw new Error("invalid request was sent");
  },
});
try {
  await invalid.auth.login({ body: { email: 1 } });
  throw new Error("invalid untagged value was accepted");
} catch (cause) {
  const text = JSON.stringify(cause);
  if (!text.includes("value does not match an enum variant")) throw new Error(text);
}
if (invalidSent !== 0) throw new Error("invalid untagged value reached fetch");
"#;
    run_bun(&files, harness);
}

fn narrowed_untagged_document() -> pons_ir::Document {
    let mut raw: serde_json::Value =
        serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
    raw["types"]["Choice"] = serde_json::json!({
        "kind": "enum", "name": ["choice"], "direction": "output", "repr": {"kind": "untagged"},
        "variants": [
            {"wire": "label", "ident": ["label"], "payload": {"kind": "newtype", "ty": {"kind": "string"}}},
            {"wire": "count", "ident": ["count"], "payload": {"kind": "newtype", "ty": {"kind": "int", "bits": 16, "signed": false, "wire": "number"}}}
        ]
    });
    raw["operations"][0]["errors"][0]["body"] =
        serde_json::json!({"ref": "Choice", "variant": "label"});
    serde_json::from_value(raw).unwrap()
}

#[test]
fn a_narrowed_untagged_variant_only_tries_the_variant_it_names() {
    let document = narrowed_untagged_document();
    let files = emit_with_strategy(&document, EmitMode::Lossy, ErrorStrategy::Throw)
        .unwrap()
        .files;
    let auth = files
        .iter()
        .find(|file| file.path == "operations/auth.ts")
        .unwrap();
    assert!(
        auth.content.contains("ApiError<number, string, Headers>"),
        "the narrowed declaration must not offer the other variant:\n{}",
        auth.content
    );

    let harness = r#"
import { createClient } from "./client.ts";

const named = createClient({
  baseUrl: "http://example.test",
  fetch: async () => new Response(JSON.stringify("late"), { status: 400, headers: { "content-type": "application/json" } }),
});
const failure = await named.auth.login({ body: { email: "person@example.test" } }).catch((cause: any) => cause);
if (failure?._tag !== "ApiError" || failure.body !== "late") {
  throw new Error(`the narrowed variant was not decoded: ${JSON.stringify(failure)}`);
}

const other = createClient({
  baseUrl: "http://example.test",
  fetch: async () => new Response(JSON.stringify(7), { status: 400, headers: { "content-type": "application/json" } }),
});
try {
  await other.auth.login({ body: { email: "person@example.test" } });
  throw new Error("a value of another variant was accepted");
} catch (cause) {
  const text = JSON.stringify(cause);
  if (!text.includes("value does not match an enum variant")) throw new Error(text);
}
"#;
    run_bun(&files, harness);
}

#[test]
fn drops_refreshed_custom_query_credentials_across_redirects() {
    let mut document: pons_ir::Document =
        serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
    document.auth_schemes.insert(
        "adapter".into(),
        pons_ir::AuthScheme::Custom {
            description: "query adapter".into(),
            adapter: None,
            ext: Default::default(),
        },
    );
    document.operations[1].auth.alternatives = vec![vec!["adapter".into()]];
    document.operations[1].provides = Some(pons_ir::CredentialProvision {
        scheme: "adapter".into(),
        value: CredentialLocator::Header {
            name: "X-Token".into(),
        },
        expires_at: None,
        ext: Default::default(),
    });
    let files = emit_with_strategy(&document, EmitMode::Strict, ErrorStrategy::Throw)
        .unwrap()
        .files;
    let harness = r#"
import { createClient } from "./client.ts";

const requested: string[] = [];
let calls = 0;
let attachments = 0;
const client = createClient({
  baseUrl: "https://example.test",
  authAdapters: {
    adapter: () => {
      attachments += 1;
      return attachments === 1 ? { query: { adapter_token: "first" } } : {};
    },
  },
  fetch: async (url) => {
    calls += 1;
    requested.push(url.searchParams.get("adapter_token") ?? "none");
    if (calls < 3) {
      return new Response(null, { status: 302, headers: { location: `/events/hop${calls}`, "X-Token": "fresh" } });
    }
    return new Response(new ReadableStream({ start(controller) { controller.close(); } }), {
      status: 200,
      headers: { "content-type": "text/event-stream", "X-Trace-ID": "trace" },
    });
  },
});
await client.events.watch({ path: { cursor: "cursor" }, query: {}, headers: {} });
if (requested.join(",") !== "first,none,none") throw new Error(`stale credential was resurrected: ${requested.join(",")}`);
"#;
    run_bun(&files, harness);
}

#[test]
fn drops_direction_invisible_fields_from_preserved_objects() {
    let mut raw: serde_json::Value =
        serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
    raw["types"]["Envelope"] = serde_json::json!({
        "kind": "struct",
        "name": ["envelope"],
        "direction": "both",
        "openness": "preserve_opaque",
        "fields": [
            { "wire": "note", "ident": ["note"], "ty": { "kind": "string" }, "presence": "required" },
            { "wire": "secret", "ident": ["secret"], "ty": { "kind": "string" }, "presence": "optional", "access": "write_only" },
            {
                "wire": "trace",
                "ident": ["trace"],
                "ty": { "kind": "opaque", "codec": "trace.codec", "wire": "json" },
                "presence": "optional",
                "access": "read_only"
            }
        ]
    });
    raw["operations"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::json!({
            "id": [["session"], ["echo"]],
            "method": "POST",
            "path": [{"lit": "session"}, {"lit": "echo"}],
            "body": {"ty": {"ref": "Envelope"}, "content": "json", "required": true},
            "responses": [{"status": 200, "content": "json", "body": {"ref": "Envelope"}}],
            "errors": [],
            "auth": {"alternatives": [], "optional": false},
            "semantics": {"safe": false, "idempotent": false}
        }));
    let document: pons_ir::Document = serde_json::from_value(raw).unwrap();
    let files = emit_with_strategy(&document, EmitMode::Strict, ErrorStrategy::Throw)
        .unwrap()
        .files;
    let harness = r#"
import { createClient } from "./client.ts";

const codecCalls: string[] = [];
let sent = "";
const client = createClient({
  baseUrl: "https://example.test",
  codecs: {
    "trace.codec": {
      encode(value: unknown, wire: string) {
        codecCalls.push(`encode:${String(value)}`);
        return `wire:${String(value)}`;
      },
      decode(value: unknown, wire: string) {
        codecCalls.push(`decode:${String(value)}`);
        return `plain:${String(value)}`;
      },
    },
  },
  fetch: async (_url, init) => {
    sent = String(init?.body);
    return new Response(JSON.stringify({ note: "hello", secret: "server-secret", trace: "wire:server-trace", extra: "kept" }), {
      status: 200,
      headers: { "content-type": "application/json" },
    });
  },
});
const value = await client.session.echo({
  body: { note: "hello", secret: "client-secret", extra: "carried" },
});
if (codecCalls.join(",") !== "decode:wire:server-trace") {
  throw new Error(`direction-invisible field was encoded: ${codecCalls.join(",")}`);
}
if (sent.includes("trace")) {
  throw new Error(`read_only field reached the wire: ${sent}`);
}
if (!sent.includes("secret")) throw new Error(`write_only field was dropped from the request: ${sent}`);
if (!sent.includes("carried")) throw new Error(`unknown key was dropped: ${sent}`);
if (Object.hasOwn(value, "secret")) throw new Error(`write_only field surfaced: ${JSON.stringify(value)}`);
if (value.trace !== "plain:wire:server-trace") throw new Error(`read_only field was not decoded: ${JSON.stringify(value)}`);
if (value.extra !== "kept") throw new Error(`unknown key was not preserved: ${JSON.stringify(value)}`);
"#;
    run_bun(&files, harness);
}

#[test]
fn a_read_only_request_field_is_refused_where_it_would_be_dropped() {
    let files = emit_with_strategy(
        &read_only_request_document(),
        EmitMode::Strict,
        ErrorStrategy::Throw,
    )
    .unwrap()
    .files;
    let harness = r#"
import { createClient } from "./client.ts";

let fetched = 0;
let sent = "";
const client = createClient({
  baseUrl: "https://example.test",
  fetch: async (_url, init) => {
    fetched += 1;
    sent = String(init?.body);
    return new Response(JSON.stringify({ id: "1" }), { status: 200, headers: { "content-type": "application/json" } });
  },
});
const refuses = async (body: any, expected: string) => {
  try {
    await client.auth.login({ body });
    throw new Error(`the client accepted ${JSON.stringify(body)}`);
  } catch (cause) {
    if (!JSON.stringify(cause).includes(expected)) throw new Error(`${JSON.stringify(body)} failed with ${JSON.stringify(cause)}`);
  }
};

await refuses({ email: "person@example.test", id: "7" }, "field id is read_only");
await refuses({ email: "person@example.test", id: "7", extra: "carried" }, "field id is read_only");
if (fetched !== 0) throw new Error(`a refused body reached fetch: ${sent}`);
if (sent !== "") throw new Error(`a refused body was sent: ${sent}`);

const value = await client.auth.login({ body: { email: "person@example.test", extra: "carried" } });
if (fetched !== 1) throw new Error("the legal body never reached fetch");
if (!sent.includes("carried")) throw new Error(`an open key was dropped: ${sent}`);
if (sent.includes("id")) throw new Error(`a dropped field was invented: ${sent}`);
if (value.id !== "1") throw new Error(`the response was not decoded: ${JSON.stringify(value)}`);
"#;
    run_bun(&files, harness);
}

fn read_only_request_document() -> pons_ir::Document {
    let mut raw: serde_json::Value =
        serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
    raw["types"]["LoginRequest"] = serde_json::json!({
        "kind": "struct",
        "name": ["login", "request"],
        "direction": "both",
        "openness": "preserve_opaque",
        "fields": [
            {"wire": "email", "ident": ["email"], "ty": {"kind": "string"}, "presence": "required"},
            {"wire": "id", "ident": ["id"], "ty": {"kind": "string"}, "presence": "optional", "access": "read_only"}
        ]
    });
    raw["operations"][0]["provides"] = serde_json::Value::Null;
    serde_json::from_value(raw).unwrap()
}

#[test]
fn an_untyped_openness_value_checks_the_extras_it_preserves() {
    let mut raw: serde_json::Value =
        serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
    raw["types"]["Metadata"] = serde_json::json!({
        "kind": "struct",
        "name": ["metadata"],
        "direction": "both",
        "openness": {"preserve_typed": {"value": {"kind": "string"}}},
        "fields": []
    });
    raw["operations"][0]["body"] = serde_json::json!({
        "ty": {"ref": "Metadata"},
        "content": "json",
        "required": true
    });
    raw["operations"][0]["responses"][0]["body"] = serde_json::json!({"ref": "Metadata"});
    raw["operations"][0]["provides"] = serde_json::Value::Null;
    let document: pons_ir::Document = serde_json::from_value(raw).unwrap();
    let openapi: serde_json::Value =
        serde_json::from_str(&pons_emit_openapi::emit(&document).unwrap()).unwrap();
    assert_eq!(
        openapi["components"]["schemas"]["Metadata"]["additionalProperties"],
        serde_json::json!({"type": "string"})
    );
    let files = emit_with_strategy(&document, EmitMode::Strict, ErrorStrategy::Throw)
        .unwrap()
        .files;
    let harness = r#"
import { createClient } from "./client.ts";

let reply: any = { extra: "kept" };
const client = createClient({
  baseUrl: "https://example.test",
  fetch: async () => new Response(JSON.stringify(reply), { status: 200, headers: { "content-type": "application/json" } }),
});
const login = async (body: any) => {
  try { return { value: await client.auth.login({ body }) }; }
  catch (cause) { return { failure: cause }; }
};
const refuses = (result: any, where: string) => {
  if (!result.failure) throw new Error(`${where} was accepted: ${JSON.stringify(result)}`);
  if (!String(result.failure.message).includes("expected string")) {
    throw new Error(`${where} failed with ${JSON.stringify(result.failure)}`);
  }
};

refuses(await login({ id: "1", extra: 2 }), "an untyped extra value");
reply = { extra: 2 };
refuses(await login({ id: "1", extra: "carried" }), "an untyped response value");
reply = { extra: "kept" };
const sent = await login({ id: "1", extra: "carried" });
if (sent.failure) throw new Error(`a typed extra was rejected: ${JSON.stringify(sent.failure)}`);
if (sent.value.extra !== "kept") throw new Error(`the extra was lost: ${JSON.stringify(sent.value)}`);
"#;
    run_bun(&files, harness);
}

#[test]
fn counts_unicode_code_points_in_string_length_bounds() {
    let mut raw: serde_json::Value =
        serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
    raw["types"]["LoginRequest"]["fields"][0]["ty"] = serde_json::json!({
        "kind": "string",
        "min_length": 3,
        "max_length": 4
    });
    raw["types"]["User"]["fields"][0]["ty"] = serde_json::json!({
        "kind": "string",
        "min_length": 3,
        "max_length": 4
    });
    raw["operations"][0]["provides"] = serde_json::Value::Null;
    let document: pons_ir::Document = serde_json::from_value(raw).unwrap();
    let files = emit_with_strategy(&document, EmitMode::Strict, ErrorStrategy::Throw)
        .unwrap()
        .files;
    let harness = r#"
import { createClient } from "./client.ts";

let sent = "";
let calls = 0;
const client = createClient({
  baseUrl: "https://example.test",
  fetch: async (_url, init) => {
    calls += 1;
    sent = String(init?.body);
    return new Response(JSON.stringify({ id: "\u{1F600}\u{1F600}\u{1F600}" }), {
      status: 200,
      headers: { "content-type": "application/json" },
    });
  },
});
const value = await client.auth.login({ body: { email: "\u{1F600}\u{1F600}\u{1F600}" } });
if (value.id !== "\u{1F600}\u{1F600}\u{1F600}") throw new Error(`response was not decoded: ${JSON.stringify(value)}`);
if (!sent.includes("\u{1F600}")) throw new Error(`request was not encoded: ${sent}`);

let shortRequest = 0;
const tooShort = createClient({
  baseUrl: "https://example.test",
  fetch: async () => {
    shortRequest += 1;
    return new Response("{}", { status: 200, headers: { "content-type": "application/json" } });
  },
});
try {
  await tooShort.auth.login({ body: { email: "\u{1F600}\u{1F600}" } });
  throw new Error("a two code point request was accepted");
} catch (cause) {
  if (cause?._tag !== "RequestError") throw new Error(JSON.stringify(cause));
}
if (shortRequest !== 0) throw new Error("a short request reached the wire");

let longRequest = 0;
const tooLong = createClient({
  baseUrl: "https://example.test",
  fetch: async () => {
    longRequest += 1;
    return new Response("{}", { status: 200, headers: { "content-type": "application/json" } });
  },
});
try {
  await tooLong.auth.login({ body: { email: "\u{1F600}\u{1F600}\u{1F600}\u{1F600}\u{1F600}" } });
  throw new Error("a five code point request was accepted");
} catch (cause) {
  if (cause?._tag !== "RequestError") throw new Error(JSON.stringify(cause));
}
if (longRequest !== 0) throw new Error("a long request reached the wire");

const tooShortResponse = createClient({
  baseUrl: "https://example.test",
  fetch: async () => new Response(JSON.stringify({ id: "\u{1F600}\u{1F600}" }), {
    status: 200,
    headers: { "content-type": "application/json" },
  }),
});
try {
  await tooShortResponse.auth.login({ body: { email: "\u{1F600}\u{1F600}\u{1F600}" } });
  throw new Error("a two code point response was accepted");
} catch (cause) {
  if (cause?._tag !== "DecodeError") throw new Error(JSON.stringify(cause));
}

const tooLongResponse = createClient({
  baseUrl: "https://example.test",
  fetch: async () => new Response(JSON.stringify({ id: "\u{1F600}\u{1F600}\u{1F600}\u{1F600}\u{1F600}" }), {
    status: 200,
    headers: { "content-type": "application/json" },
  }),
});
try {
  await tooLongResponse.auth.login({ body: { email: "\u{1F600}\u{1F600}\u{1F600}" } });
  throw new Error("a five code point response was accepted");
} catch (cause) {
  if (cause?._tag !== "DecodeError") throw new Error(JSON.stringify(cause));
}
if (calls !== 1) throw new Error(`unexpected calls ${calls}`);
"#;
    run_bun(&files, harness);
}

#[test]
fn follows_cross_origin_redirects_that_carry_no_credentials() {
    let mut document: pons_ir::Document =
        serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
    document.operations[1].auth.alternatives = Vec::new();
    document.operations[1].headers = vec![pons_ir::HeaderParameter {
        wire: "X-Accept".into(),
        ident: vec!["accept".into()],
        ty: TypeRef::Inline(InlineType::String {
            format: None,
            min_length: None,
            max_length: None,
            pattern: None,
        }),
        presence: Presence::Required,
        ext: BTreeMap::new(),
    }];
    let files = emit_with_strategy(&document, EmitMode::Strict, ErrorStrategy::Throw)
        .unwrap()
        .files;
    let harness = r#"
import { createClient } from "./client.ts";

const requested: string[] = [];
let calls = 0;
const client = createClient({
  baseUrl: "https://a.test",
  fetch: async (url, init) => {
    calls += 1;
    requested.push(`${url.origin}${url.pathname} ${new Headers(init?.headers).get("x-accept")}`);
    if (calls === 1) return new Response(null, { status: 302, headers: { location: "https://b.test/events/redirected" } });
    return new Response(new ReadableStream({ start(controller) { controller.close(); } }), {
      status: 200,
      headers: { "content-type": "text/event-stream", "X-Trace-ID": "trace" },
    });
  },
});
await client.events.watch({ path: { cursor: "cursor" }, query: {}, headers: { "X-Accept": "application/json" } });
if (calls !== 2) throw new Error(`cross-origin redirect was refused: ${JSON.stringify(requested)}`);
if (!requested[1].startsWith("https://b.test/events/redirected application/json")) {
  throw new Error(`wrong redirect target: ${requested[1]}`);
}
"#;
    run_bun(&files, harness);
}

#[test]
fn refuses_cross_origin_redirects_without_capturing_their_credentials() {
    let document: pons_ir::Document =
        serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
    let files = emit_with_strategy(&document, EmitMode::Strict, ErrorStrategy::Throw)
        .unwrap()
        .files;
    let harness = r#"
import { createClient } from "./client.ts";

let stored = 0;
let calls = 0;
const client = createClient({
  baseUrl: "https://a.test",
  auth: {
    session: {
      get: async () => undefined,
      set: async () => {
        stored += 1;
      },
      clear: async () => {},
    },
  },
  fetch: async () => {
    calls += 1;
    if (calls === 1) {
      return new Response(JSON.stringify({ token: "captured", id: "1" }), {
        status: 307,
        headers: { "content-type": "application/json", location: "https://b.test/auth/login" },
      });
    }
    return new Response(JSON.stringify({ id: "1" }), {
      status: 200,
      headers: { "content-type": "application/json" },
    });
  },
});
let failure: any;
try {
  await client.auth.login({ body: { email: "person@example.test" } });
} catch (cause) {
  failure = cause;
}
if (failure?._tag !== "RedirectError") throw new Error(`unexpected failure ${JSON.stringify(failure)}`);
if (calls !== 1) throw new Error("a refused redirect was followed");
if (stored !== 0) throw new Error(`a refused redirect stored ${stored} credentials`);
"#;
    run_bun(&files, harness);
}

#[test]
fn does_not_capture_a_body_credential_from_an_allowed_cross_origin_redirect() {
    let document: pons_ir::Document =
        serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
    let files = emit_with_strategy(&document, EmitMode::Strict, ErrorStrategy::Throw)
        .unwrap()
        .files;
    let harness = r#"
import { createClient } from "./client.ts";

const stored: string[] = [];
let calls = 0;
const client = createClient({
  baseUrl: "https://a.test",
  auth: {
    session: {
      get: async () => undefined,
      set: async (value) => { stored.push(String(value)); },
      clear: async () => {},
    },
  },
  fetch: async (_url, init) => {
    calls += 1;
    if (calls === 1) {
      return new Response(JSON.stringify({ token: "cross-origin", id: "1" }), {
        status: 303,
        headers: { "content-type": "application/json", location: "https://b.test/auth/login" },
      });
    }
    if (new Headers(init?.headers).has("authorization")) throw new Error("the redirect was authenticated");
    return new Response(JSON.stringify({ id: "1" }), {
      status: 200,
      headers: { "content-type": "application/json" },
    });
  },
});
const user = await client.auth.login({ body: { email: "person@example.test" } });
if (calls !== 2) throw new Error(`the allowed redirect was not followed: ${calls}`);
if (stored.length !== 0) throw new Error(`a cross-origin redirect stored ${JSON.stringify(stored)}`);
if (user.id !== "1") throw new Error(`unexpected login result ${JSON.stringify(user)}`);
"#;
    run_bun(&files, harness);
}

#[test]
fn does_not_capture_a_set_cookie_credential_from_an_allowed_cross_origin_redirect() {
    let mut document: pons_ir::Document =
        serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
    document.operations[0].provides = Some(pons_ir::CredentialProvision {
        scheme: "session".into(),
        value: CredentialLocator::SetCookie { name: "sid".into() },
        expires_at: None,
        ext: BTreeMap::new(),
    });
    let files = emit_with_strategy(&document, EmitMode::Strict, ErrorStrategy::Throw)
        .unwrap()
        .files;
    let harness = r#"
import { createClient } from "./client.ts";

const stored: string[] = [];
let calls = 0;
const client = createClient({
  baseUrl: "https://a.test",
  auth: {
    session: {
      get: async () => undefined,
      set: async (value) => { stored.push(String(value)); },
      clear: async () => {},
    },
  },
  fetch: async () => {
    calls += 1;
    if (calls === 1) {
      return new Response(null, {
        status: 303,
        headers: { "set-cookie": "sid=from-cross-origin; Path=/", location: "https://b.test/auth/login" },
      });
    }
    return new Response(JSON.stringify({ id: "1" }), {
      status: 200,
      headers: { "content-type": "application/json" },
    });
  },
});
await client.auth.login({ body: { email: "person@example.test" } });
if (calls !== 2) throw new Error(`the allowed redirect was not followed: ${calls}`);
if (stored.length !== 0) throw new Error(`a cross-origin redirect stored ${JSON.stringify(stored)}`);
"#;
    run_bun(&files, harness);
}

#[test]
fn reports_a_body_credential_locator_on_a_stream_as_an_auth_source_failure() {
    let mut document: pons_ir::Document =
        serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
    document.operations[1].auth.alternatives = Vec::new();
    document.operations[1].provides = Some(pons_ir::CredentialProvision {
        scheme: "session".into(),
        value: CredentialLocator::Body {
            pointer: "/token".into(),
        },
        expires_at: None,
        ext: BTreeMap::new(),
    });
    let files = emit_with_strategy(&document, EmitMode::Strict, ErrorStrategy::Throw)
        .unwrap()
        .files;
    let harness = r#"
import { createClient } from "./client.ts";

const client = createClient({
  baseUrl: "https://example.test",
  fetch: async () => new Response(new ReadableStream({ start(controller) { controller.close(); } }), {
    status: 200,
    headers: { "content-type": "text/event-stream", "X-Trace-ID": "trace" },
  }),
});
let failure: any;
try {
  await client.events.watch({ path: { cursor: "cursor" }, query: {}, headers: {} });
} catch (cause) {
  failure = cause;
}
if (failure?._tag !== "AuthSourceError") throw new Error(`unexpected failure ${JSON.stringify(failure)}`);
if (failure.scheme !== "session") throw new Error(`unexpected scheme ${JSON.stringify(failure)}`);
if (String(failure.cause?.message).includes("stream") === false) {
  throw new Error(`unhelpful cause ${JSON.stringify(failure.cause)}`);
}
"#;
    run_bun(&files, harness);
}

#[test]
fn rejects_request_headers_the_runtime_overwrites() {
    let document: pons_ir::Document =
        serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
    let mut header = document.operations[1].headers[0].clone();
    header.presence = Presence::Required;
    assert!(
        emit_with_strategy(&document, EmitMode::Strict, ErrorStrategy::Throw).is_ok(),
        "a request header the runtime keeps must still emit"
    );
    for wire in ["Content-Type", "authorization"] {
        let mut named = header.clone();
        named.wire = wire.into();
        named.ident = vec![wire.to_ascii_lowercase()];
        let mut refused = document.clone();
        if wire == "authorization" {
            refused.operations[0].headers = Vec::new();
            refused.operations[1].headers = vec![named];
        } else {
            refused.operations[0].headers = vec![named];
        }
        let rejection = emit_with_strategy(&refused, EmitMode::Strict, ErrorStrategy::Throw)
            .expect_err("a request header the runtime overwrites must be rejected");
        assert!(
            rejection.0.contains(wire)
                && rejection
                    .0
                    .contains("is owned by the request body or the security model"),
            "{rejection}"
        );
    }
}

#[test]
fn rejects_request_headers_a_header_credential_overwrites() {
    let mut document: pons_ir::Document =
        serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
    let mut header = document.operations[1].headers[0].clone();
    header.wire = "X-Tenant".into();
    header.ident = vec!["tenant".into()];
    document.operations[1].headers = vec![header];
    for mode in [EmitMode::Strict, EmitMode::Lossy] {
        let rejection = emit_with_strategy(&document, mode, ErrorStrategy::Throw)
            .expect_err("a request header the credential overwrites must be rejected");
        assert!(
            rejection.0.contains("X-Tenant")
                && rejection.0.contains("operations[1].headers")
                && rejection.0.contains("tenant")
                && rejection.0.contains("overwrites"),
            "{}",
            rejection
        );
    }

    let mut cookie_document: pons_ir::Document =
        serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
    let mut cookie_name = cookie_document.operations[1].headers[0].clone();
    cookie_name.wire = "X-Cookie-Key".into();
    cookie_name.ident = vec!["cookie".into(), "key".into()];
    cookie_document.auth_schemes.insert(
        "cookiekey".into(),
        pons_ir::AuthScheme::ApiKey {
            location: ApiKeyLocation::Cookie,
            name: "X-Cookie-Key".into(),
        },
    );
    cookie_document.operations[1].auth.alternatives = vec![vec!["cookiekey".into()]];
    cookie_document.operations[1].headers = vec![cookie_name];
    assert!(
        emit_with_strategy(&cookie_document, EmitMode::Strict, ErrorStrategy::Throw).is_ok(),
        "a cookie credential merges into the cookie header instead of discarding the declared one"
    );
}

#[test]
fn a_catch_all_success_status_still_follows_redirects() {
    let base: pons_ir::Document =
        serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
    let harness = r#"
import { createClient } from "./client.ts";

let hops = 0;
const client = createClient({
  baseUrl: "https://example.test",
  fetch: async (_url, init) => {
    if (init?.redirect !== "manual") throw new Error("redirects are not handled by the client");
    hops += 1;
    if (hops === 1) {
      return new Response(null, { status: 302, headers: { location: "https://example.test/v2/auth/login" } });
    }
    return new Response(JSON.stringify({ id: "1" }), {
      status: 503,
      headers: { "content-type": "application/json" },
    });
  },
});
const value = await client.auth.login({ body: { email: "person@example.test" } });
if (hops !== 2) throw new Error(`the redirect was not followed: ${hops} hops`);
if (value.status !== 503) throw new Error(`the catch-all did not match: ${JSON.stringify(value)}`);
if (value.value.id !== "1") throw new Error(`the body was not decoded: ${JSON.stringify(value)}`);
"#;
    for status in [
        pons_ir::ResponseStatus::Unknown,
        pons_ir::ResponseStatus::Default,
    ] {
        let mut document = base.clone();
        document.operations[0].responses[0].status = status;
        let files = emit_with_strategy(&document, EmitMode::Strict, ErrorStrategy::Throw)
            .unwrap()
            .files;
        run_bun(&files, harness);
    }
}

#[test]
fn a_catch_all_success_status_matches_any_status_a_fixed_one_does_not() {
    let base: pons_ir::Document =
        serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
    let harness = r#"
import { createClient } from "./client.ts";

const unavailable = createClient({
  baseUrl: "https://example.test",
  fetch: async () => new Response(JSON.stringify({ id: "1" }), {
    status: 503,
    headers: { "content-type": "application/json" },
  }),
});
const value = await unavailable.auth.login({ body: { email: "person@example.test" } });
if (value.status !== 503) throw new Error(`the runtime status was not preserved: ${JSON.stringify(value)}`);
if (value.value.id !== "1") throw new Error(`the body was not decoded: ${JSON.stringify(value)}`);
"#;
    for status in [
        pons_ir::ResponseStatus::Unknown,
        pons_ir::ResponseStatus::Default,
    ] {
        let mut document = base.clone();
        document.operations[0].responses[0].status = status;
        let files = emit_with_strategy(&document, EmitMode::Strict, ErrorStrategy::Throw)
            .unwrap()
            .files;
        run_bun(&files, harness);
    }

    let files = emit_with_strategy(&base, EmitMode::Strict, ErrorStrategy::Throw)
        .unwrap()
        .files;
    let harness = r#"
import { createClient } from "./client.ts";

const created = createClient({
  baseUrl: "https://example.test",
  fetch: async () => new Response(JSON.stringify({ id: "1" }), {
    status: 201,
    headers: { "content-type": "application/json" },
  }),
});
try {
  await created.auth.login({ body: { email: "person@example.test" } });
  throw new Error("an undeclared 201 was accepted");
} catch (cause) {
  if (cause?._tag !== "UnexpectedStatusError") throw new Error(JSON.stringify(cause));
  if (cause.status !== 201) {
    throw new Error(`unhelpful failure ${JSON.stringify(cause)}`);
  }
}
"#;
    run_bun(&files, harness);
}

fn catch_all_success_document() -> pons_ir::Document {
    let mut document: pons_ir::Document =
        serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
    document.operations[0].responses[0].status = pons_ir::ResponseStatus::Unknown;
    document
}

#[test]
fn a_declared_error_status_wins_over_a_catch_all_success() {
    let files = emit_with_strategy(
        &catch_all_success_document(),
        EmitMode::Strict,
        ErrorStrategy::Throw,
    )
    .unwrap()
    .files;
    let harness = r#"
import { createClient } from "./client.ts";

const missing = createClient({
  baseUrl: "https://example.test",
  fetch: async () => new Response(JSON.stringify({ id: "1", code: "invalid" }), {
    status: 404,
    headers: { "content-type": "application/json" },
  }),
});
const failure: any = await missing.auth.login({ body: { email: "person@example.test" } })
  .then(() => undefined, (cause) => cause);
if (failure?._tag !== "ApiError") throw new Error(`a declared 4xx produced a success: ${JSON.stringify(failure)}`);
if (failure.status !== 404) throw new Error(`the error status was lost: ${JSON.stringify(failure)}`);
if (failure.body?.code !== "invalid") throw new Error(`the typed error body was lost: ${JSON.stringify(failure)}`);
"#;
    run_bun(&files, harness);
}

#[test]
fn a_catch_all_success_still_matches_a_status_no_error_declares() {
    let files = emit_with_strategy(
        &catch_all_success_document(),
        EmitMode::Strict,
        ErrorStrategy::Throw,
    )
    .unwrap()
    .files;
    let harness = r#"
import { createClient } from "./client.ts";

const unavailable = createClient({
  baseUrl: "https://example.test",
  fetch: async () => new Response(JSON.stringify({ id: "1" }), {
    status: 503,
    headers: { "content-type": "application/json" },
  }),
});
const value = await unavailable.auth.login({ body: { email: "person@example.test" } });
if (value.status !== 503) throw new Error(`the catch-all did not match: ${JSON.stringify(value)}`);
if (value.value.id !== "1") throw new Error(`the body was not decoded: ${JSON.stringify(value)}`);
"#;
    run_bun(&files, harness);
}

#[test]
fn a_default_error_takes_every_status_no_success_declares() {
    let mut document: pons_ir::Document =
        serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
    document.operations[0].responses.clear();
    document.operations[0].errors[0].status = pons_ir::ErrorStatus::Default;
    let files = emit_with_strategy(&document, EmitMode::Strict, ErrorStrategy::Throw)
        .unwrap()
        .files;
    let harness = r#"
import { createClient } from "./client.ts";

const answering = (status: number) => createClient({
  baseUrl: "https://example.test",
  fetch: async () => new Response(JSON.stringify({ code: "invalid" }), {
    status,
    headers: { "content-type": "application/json" },
  }),
});
for (const status of [200, 201, 503]) {
  const failure: any = await answering(status).auth.login({ body: { email: "person@example.test" } })
    .then(() => undefined, (cause) => cause);
  if (failure?._tag !== "ApiError") throw new Error(`${status} was not the default error: ${JSON.stringify(failure)}`);
  if (failure.status !== status) throw new Error(`the error status was lost: ${JSON.stringify(failure)}`);
  if (failure.body?.code !== "invalid") throw new Error(`the typed error body was lost: ${JSON.stringify(failure)}`);
}
"#;
    run_bun(&files, harness);
}

fn string_wire_integer_document() -> pons_ir::Document {
    let mut raw: serde_json::Value =
        serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
    raw["types"]["LoginRequest"]["fields"] = serde_json::json!([
        {"wire": "email", "ident": ["email"], "ty": {"kind": "string"}, "presence": "required"},
        {"wire": "count", "ident": ["count"], "ty": {"kind": "int", "bits": 32, "signed": false, "wire": "string"}, "presence": "required"},
        {"wire": "offset", "ident": ["offset"], "ty": {"kind": "int", "bits": 32, "signed": true, "wire": "string"}, "presence": "required"}
    ]);
    raw["types"]["User"]["fields"] = serde_json::json!([
        {"wire": "id", "ident": ["id"], "ty": {"kind": "int", "bits": 64, "signed": false, "wire": "string"}, "presence": "required"},
        {"wire": "offset", "ident": ["offset"], "ty": {"kind": "int", "bits": 32, "signed": true, "wire": "string"}, "presence": "required"}
    ]);
    raw["operations"][0]["provides"] = serde_json::Value::Null;
    serde_json::from_value(raw).unwrap()
}

#[test]
fn string_wire_integers_reject_every_form_the_published_pattern_rejects() {
    let files = emit_with_strategy(
        &string_wire_integer_document(),
        EmitMode::Strict,
        ErrorStrategy::Throw,
    )
    .unwrap()
    .files;
    let harness = r#"
import { createClient } from "./client.ts";

let reply: any = { id: "0", offset: "-0" };
let sent = "";
const client = createClient({
  baseUrl: "https://example.test",
  fetch: async (_url, init) => {
    sent = String(init?.body);
    return new Response(JSON.stringify(reply), { status: 200, headers: { "content-type": "application/json" } });
  },
});
const login = async (body: any) => {
  let failure: any;
  let value: any;
  try { value = await client.auth.login({ body }); } catch (cause) { failure = cause; }
  return { failure, value };
};
const rejects = (result: any, where: string, tag: string) => {
  if (result.failure?._tag !== tag) throw new Error(`${where} was accepted: ${JSON.stringify(result)}`);
  if (!String(result.failure.message).includes("expected integer string")) {
    throw new Error(`${where} failed with ${JSON.stringify(result.failure)}`);
  }
};

for (const count of ["007", "00", "-0", "+0", " 7", "7 "]) {
  rejects(await login({ email: "person@example.test", count, offset: "0" }), `request count ${count}`, "RequestError");
}
for (const offset of ["007", "00", "-00"]) {
  rejects(await login({ email: "person@example.test", count: "7", offset }), `request offset ${offset}`, "RequestError");
}

const sent_ok = await login({ email: "person@example.test", count: "70", offset: "-0" });
if (sent_ok.failure) throw new Error(`a canonical request was rejected: ${JSON.stringify(sent_ok.failure)}`);
if (!sent.includes(`"count":"70"`) || !sent.includes(`"offset":"-0"`)) {
  throw new Error(`the request lost its integers: ${sent}`);
}
if (sent_ok.value.id !== "0") throw new Error(`the response was not returned: ${JSON.stringify(sent_ok.value)}`);

for (const id of ["007", "00", "-0"]) {
  reply = { id, offset: "0" };
  rejects(await login({ email: "person@example.test", count: "7", offset: "0" }), `response id ${id}`, "DecodeError");
}
for (const offset of ["007", "00", "-00"]) {
  reply = { id: "7", offset };
  rejects(await login({ email: "person@example.test", count: "7", offset: "0" }), `response offset ${offset}`, "DecodeError");
}
reply = { id: "0", offset: "-0" };
const received = await login({ email: "person@example.test", count: "7", offset: "0" });
if (received.failure) throw new Error(`a canonical response was rejected: ${JSON.stringify(received.failure)}`);
if (received.value.id !== "0" || received.value.offset !== "-0") {
  throw new Error(`the canonical response was lost: ${JSON.stringify(received.value)}`);
}
"#;
    run_bun(&files, harness);
}

fn base64_document() -> pons_ir::Document {
    let mut raw: serde_json::Value =
        serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
    raw["types"]["LoginRequest"]["fields"] = serde_json::json!([
        {"wire": "email", "ident": ["email"], "ty": {"kind": "string"}, "presence": "required"},
        {"wire": "blob", "ident": ["blob"], "ty": {"kind": "bytes", "encoding": "base64"}, "presence": "required"},
        {"wire": "url", "ident": ["url"], "ty": {"kind": "bytes", "encoding": "base64url"}, "presence": "required"}
    ]);
    raw["types"]["User"]["fields"] = serde_json::json!([
        {"wire": "id", "ident": ["id"], "ty": {"kind": "int", "bits": 64, "signed": false, "wire": "string"}, "presence": "required"},
        {"wire": "blob", "ident": ["blob"], "ty": {"kind": "bytes", "encoding": "base64"}, "presence": "optional"},
        {"wire": "url", "ident": ["url"], "ty": {"kind": "bytes", "encoding": "base64url"}, "presence": "optional"}
    ]);
    raw["operations"][0]["provides"] = serde_json::Value::Null;
    serde_json::from_value(raw).unwrap()
}

#[test]
fn base64_padding_follows_one_rule_per_encoding() {
    let files = emit_with_strategy(&base64_document(), EmitMode::Strict, ErrorStrategy::Throw)
        .unwrap()
        .files;
    let harness = r#"
import { createClient } from "./client.ts";

let reply: any = { id: "0" };
let sent = "";
const client = createClient({
  baseUrl: "https://example.test",
  fetch: async (_url, init) => {
    sent = String(init?.body);
    return new Response(JSON.stringify(reply), { status: 200, headers: { "content-type": "application/json" } });
  },
});
const login = async (body: any) => {
  let failure: any;
  let value: any;
  try { value = await client.auth.login({ body }); } catch (cause) { failure = cause; }
  return { failure, value };
};
const rejects = (result: any, where: string, message: string) => {
  if (result.failure?._tag !== "DecodeError") throw new Error(`${where} was accepted: ${JSON.stringify(result)}`);
  if (!String(result.failure.message).includes(message)) {
    throw new Error(`${where} failed with ${JSON.stringify(result.failure)}`);
  }
};

const byte = new Uint8Array([65]);
const sent_ok = await login({ email: "person@example.test", blob: byte, url: byte });
if (sent_ok.failure) throw new Error(`a byte request was rejected: ${JSON.stringify(sent_ok.failure)}`);
if (!sent.includes(`"blob":"QQ=="`) || !sent.includes(`"url":"QQ"`)) {
  throw new Error(`the request did not use one rule per encoding: ${sent}`);
}

reply = { id: "0", blob: "QQ==" };
if ((await login({ email: "person@example.test", blob: byte, url: byte })).failure) {
  throw new Error("padded base64 was rejected");
}
reply = { id: "0", url: "QQ" };
if ((await login({ email: "person@example.test", blob: byte, url: byte })).failure) {
  throw new Error("unpadded base64url was rejected");
}
reply = { id: "0", url: "QQ==" };
if ((await login({ email: "person@example.test", blob: byte, url: byte })).failure) {
  throw new Error("padded base64url was rejected");
}
const body = { email: "person@example.test", blob: byte, url: byte };
reply = { id: "0", blob: "QQ" };
rejects(await login(body), "unpadded base64", "invalid base64 bytes");
reply = { id: "0", url: "Q" };
rejects(await login(body), "a short base64url", "invalid base64url bytes");
reply = { id: "0", url: "QQ=" };
rejects(await login(body), "a broken base64url", "invalid base64url bytes");
reply = { id: "0", blob: "QUJD", url: "QUJD" };
const decoded = await login(body);
if (decoded.failure) throw new Error(`a legal reply was rejected: ${JSON.stringify(decoded.failure)}`);
if (decoded.value.blob.length !== 3 || decoded.value.url.length !== 3) {
  throw new Error(`the reply was not decoded: ${JSON.stringify(decoded.value)}`);
}
"#;
    run_bun(&files, harness);
}

#[test]
fn a_text_body_rejects_the_unknown_properties_a_json_body_rejects() {
    let mut raw: serde_json::Value =
        serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
    raw["types"]["LoginRequest"]["direction"] = serde_json::json!("both");
    raw["types"]["LoginRequest"]["openness"] = serde_json::json!("reject");
    raw["operations"][0]["body"]["content"] = serde_json::json!("text");
    raw["operations"][0]["responses"][0]["body"] = serde_json::json!({"ref": "LoginRequest"});
    raw["operations"][0]["provides"] = serde_json::Value::Null;
    let document: pons_ir::Document = serde_json::from_value(raw).unwrap();
    let files = emit_with_strategy(&document, EmitMode::Strict, ErrorStrategy::Throw)
        .unwrap()
        .files;
    let harness = r#"
import { createClient } from "./client.ts";

let reply: any = { email: "person@example.test" };
const client = createClient({
  baseUrl: "https://example.test",
  fetch: async () => new Response(JSON.stringify(reply), { status: 200, headers: { "content-type": "application/json" } }),
});
const login = async (body: any) => {
  let failure: any;
  let value: any;
  try { value = await client.auth.login({ body }); } catch (cause) { failure = cause; }
  return { failure, value };
};
const rejects = (result: any, where: string, tag: string) => {
  if (result.failure?._tag !== tag) throw new Error(`${where} was accepted: ${JSON.stringify(result)}`);
  if (!String(result.failure.message).includes("unknown property extra")) {
    throw new Error(`${where} failed with ${JSON.stringify(result.failure)}`);
  }
};

rejects(await login({ email: "person@example.test", extra: 1 }), "an unknown text body property", "RequestError");
reply = { email: "person@example.test", extra: 1 };
rejects(await login({ email: "person@example.test" }), "an unknown response property", "DecodeError");
reply = { email: "person@example.test" };
const sent = await login({ email: "person@example.test" });
if (sent.failure) throw new Error(`a closed body was rejected: ${JSON.stringify(sent.failure)}`);
if (sent.value.email !== "person@example.test") throw new Error("the body was not returned");
"#;
    run_bun(&files, harness);
}

fn keyed_openness_document() -> pons_ir::Document {
    let mut raw: serde_json::Value =
        serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
    raw["types"]["Metadata"] = serde_json::json!({
        "kind": "struct",
        "name": ["metadata"],
        "direction": "both",
        "openness": {"preserve_typed": {"value": {
            "kind": "map",
            "key": {"kind": "string", "pattern": "^[a-z][a-z0-9]*$"},
            "value": {"kind": "int", "bits": 16, "signed": false, "wire": "number"}
        }}},
        "fields": []
    });
    raw["operations"][0]["body"] = serde_json::json!({
        "ty": {"ref": "Metadata"},
        "content": "json",
        "required": true
    });
    raw["operations"][0]["responses"][0]["body"] = serde_json::json!({"ref": "Metadata"});
    raw["operations"][0]["provides"] = serde_json::Value::Null;
    serde_json::from_value(raw).unwrap()
}

#[test]
fn a_typed_openness_checks_the_keys_it_preserves() {
    let files = emit_with_strategy(
        &keyed_openness_document(),
        EmitMode::Strict,
        ErrorStrategy::Throw,
    )
    .unwrap()
    .files;
    let harness = r#"
import { createClient } from "./client.ts";

let reply: any = { good: 1 };
const client = createClient({
  baseUrl: "https://example.test",
  fetch: async () => new Response(JSON.stringify(reply), { status: 200, headers: { "content-type": "application/json" } }),
});
const login = async (body: any) => {
  let failure: any;
  let value: any;
  try { value = await client.auth.login({ body }); } catch (cause) { failure = cause; }
  return { failure, value };
};
const rejects = (result: any, where: string, expected: string, tag: string) => {
  if (result.failure?._tag !== tag) throw new Error(`${where} was accepted: ${JSON.stringify(result)}`);
  if (!String(result.failure.message).includes(expected)) {
    throw new Error(`${where} failed with ${JSON.stringify(result.failure)}`);
  }
};

for (const [body, where, expected] of [
  [{ Bad: 1 }, "an unknown request key", "string does not match the contract"],
  [{ good: "x" }, "an unknown request value", "expected an exact integer"],
] as Array<[any, string, string]>) {
  const result = await login(body);
  rejects(result, where, expected, "RequestError");
}
for (const [body, where, expected] of [
  [{ Bad: 1 }, "an unknown response key", "string does not match the contract"],
  [{ good: "x" }, "an unknown response value", "expected an exact integer"],
] as Array<[any, string, string]>) {
  reply = body;
  rejects(await login({ good: 1 }), where, expected, "DecodeError");
}
reply = { good: 1, other: 2 };
const sent = await login({ good: 1, other: 2 });
if (sent.failure) throw new Error(`the map keys of the contract were rejected: ${JSON.stringify(sent.failure)}`);
if (sent.value.good !== 1 || sent.value.other !== 2) throw new Error(`the extras were lost: ${JSON.stringify(sent.value)}`);
"#;
    run_bun(&files, harness);
}

#[test]
fn an_uncompilable_pattern_is_refused_before_any_client_exists() {
    let fixture: pons_ir::Document =
        serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();

    for (type_id, field_index, pattern) in [("LoginRequest", 0, "[a-"), ("User", 1, "(?<unclosed")]
    {
        let mut document = fixture.clone();
        let TypeKind::Struct { fields, .. } =
            &mut document.types.get_mut(type_id).unwrap().definition
        else {
            unreachable!()
        };
        fields[field_index].ty = TypeRef::Inline(InlineType::String {
            format: None,
            min_length: None,
            max_length: None,
            pattern: Some(pattern.into()),
        });
        let refusal = emit_with_strategy(&document, EmitMode::Strict, ErrorStrategy::Throw)
            .expect_err("a pattern that cannot compile must stop the emit");
        assert!(
            refusal.0.contains("is not a usable regular expression"),
            "{}",
            refusal.0
        );
        assert!(refusal.0.contains(pattern), "{}", refusal.0);
    }
}

#[test]
fn rejects_a_success_header_the_contract_refuses() {
    let mut raw: serde_json::Value =
        serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
    raw["operations"][1]["responses"][0]["headers"][0]["ty"] =
        serde_json::json!({"kind": "string", "format": "uuid"});
    let document: pons_ir::Document = serde_json::from_value(raw).unwrap();
    let files = validated_files(&document);
    let harness = r#"
import { createClient } from "./client.ts";

const client = createClient({
  baseUrl: "https://example.test",
  auth: { tenant: "tenant" },
  fetch: async () => new Response(new ReadableStream({ start(controller) { controller.close(); } }), {
    status: 200,
    headers: { "content-type": "text/event-stream", "X-Trace-ID": "not-a-uuid" },
  }),
});
let failure: any;
try {
  await client.events.watch({ path: { cursor: "cursor" }, query: {}, headers: {} });
} catch (cause) {
  failure = cause;
}
if (failure?._tag !== "DecodeError") throw new Error(`unexpected failure ${JSON.stringify(failure)}`);
if (!String(failure.message).startsWith("the response does not match the contract:")) {
  throw new Error(`the contract prefix is missing: ${String(failure.message)}`);
}
"#;
    run_bun(&files, harness);
}

#[test]
fn rejects_an_error_header_the_contract_refuses() {
    let mut raw: serde_json::Value =
        serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
    raw["operations"][0]["errors"][0]["headers"] = serde_json::json!([{
        "wire": "X-Reason",
        "ty": {"kind": "string", "format": "uuid"},
        "presence": "required"
    }]);
    let document: pons_ir::Document = serde_json::from_value(raw).unwrap();
    let files = validated_files(&document);
    let harness = r#"
import { createClient } from "./client.ts";

const client = createClient({
  baseUrl: "https://example.test",
  fetch: async () => new Response(JSON.stringify({ code: "invalid" }), {
    status: 400,
    headers: { "content-type": "application/json", "X-Reason": "not-a-uuid" },
  }),
});
let failure: any;
try {
  await client.auth.login({ body: { email: "person@example.test" } });
} catch (cause) {
  failure = cause;
}
if (failure?._tag !== "DecodeError") throw new Error(`unexpected failure ${JSON.stringify(failure)}`);
if (!String(failure.message).startsWith("the response does not match the contract:")) {
  throw new Error(`the contract prefix is missing: ${String(failure.message)}`);
}
"#;
    run_bun(&files, harness);
}

#[test]
fn rejects_an_sse_event_the_contract_refuses() {
    let mut raw: serde_json::Value =
        serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
    raw["operations"][1]["stream"]["events"][0]["data"] =
        serde_json::json!({"kind": "string", "format": "uuid"});
    let document: pons_ir::Document = serde_json::from_value(raw).unwrap();
    let files = validated_files(&document);
    let harness = r#"
import { createClient } from "./client.ts";

const client = createClient({
  baseUrl: "https://example.test",
  auth: { tenant: "tenant" },
  fetch: async () => new Response(
    new ReadableStream({
      start(controller) {
        controller.enqueue(new TextEncoder().encode('event: user\ndata: "not-a-uuid"\n\n'));
        controller.close();
      },
    }),
    { status: 200, headers: { "content-type": "text/event-stream", "X-Trace-ID": "trace" } },
  ),
});
const stream = await client.events.watch({ path: { cursor: "cursor" }, query: {}, headers: {} });
let failure: any;
for await (const item of stream.value) {
  failure = item;
  break;
}
if (!failure?.isErr()) throw new Error("the invalid event was admitted");
if (failure.error._tag !== "StreamDecodeError") throw new Error(`unexpected failure ${JSON.stringify(failure.error)}`);
if (failure.error.event !== "user") throw new Error(`the wrong event was named: ${failure.error.event}`);
"#;
    run_bun(&files, harness);
}

#[test]
fn rejects_an_ndjson_item_the_contract_refuses() {
    let mut raw: serde_json::Value =
        serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
    raw["operations"][1]["stream"] = serde_json::json!({
        "kind": "ndjson",
        "item": {"kind": "string", "format": "uuid"}
    });
    let document: pons_ir::Document = serde_json::from_value(raw).unwrap();
    let files = validated_files(&document);
    let harness = r#"
import { createClient } from "./client.ts";

const client = createClient({
  baseUrl: "https://example.test",
  auth: { tenant: "tenant" },
  fetch: async () => new Response('"not-a-uuid"\n', {
    status: 200,
    headers: { "content-type": "application/x-ndjson", "X-Trace-ID": "trace" },
  }),
});
const stream = await client.events.watch({ path: { cursor: "cursor" }, query: {}, headers: {} });
let failure: any;
for await (const item of stream.value) {
  failure = item;
  break;
}
if (!failure?.isErr()) throw new Error("the invalid item was admitted");
if (failure.error._tag !== "StreamDecodeError") throw new Error(`unexpected failure ${JSON.stringify(failure.error)}`);
"#;
    run_bun(&files, harness);
}
