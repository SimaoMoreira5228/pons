mod support;

use pons_emit_ts::{EmitMode, ErrorStrategy, emit_with_strategy};
use pons_ir::{CredentialLocator, InlineType, OpaqueWire, TypeKind, TypeRef};
use support::runtime::{
    flattened_named_opaque_document, flattened_opaque_document,
    flattened_with_sibling_opaque_document, nested_flattened_opaque_document, opaque_document,
    run_bun,
};

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
        serde_json::from_str(include_str!("../../../spec/fixtures/login.json")).unwrap();
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
        serde_json::from_str(include_str!("../../../spec/fixtures/login.json")).unwrap();
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
        serde_json::from_str(include_str!("../../../spec/fixtures/login.json")).unwrap();
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
        serde_json::from_str(include_str!("../../../spec/fixtures/login.json")).unwrap();
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
        serde_json::from_str(include_str!("../../../spec/fixtures/login.json")).unwrap();
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
