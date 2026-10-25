mod support;

use pons_emit_ts::{EmitMode, ErrorStrategy, emit_with_strategy};
use pons_ir::{ByteEncoding, ContentKind, InlineType, TypeKind, TypeRef};
use support::runtime::{run_bun, untagged_bytes_document, wide_number_document};

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
fn validates_text_response_constraints() {
    let mut raw: serde_json::Value =
        serde_json::from_str(include_str!("../../../spec/fixtures/login.json")).unwrap();
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
        serde_json::from_str(include_str!("../../../spec/fixtures/login.json")).unwrap();
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
fn byte_response_media_matching_remains_strict() {
    let mut document: pons_ir::Document =
        serde_json::from_str(include_str!("../../../spec/fixtures/login.json")).unwrap();
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
        serde_json::from_str(include_str!("../../../spec/fixtures/login.json")).unwrap();
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
        serde_json::from_str(include_str!("../../../spec/fixtures/login.json")).unwrap();
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
