use std::{collections::BTreeMap, fs, process::Command};

use pons_emit_ts::{EmitMode, ErrorStrategy, GeneratedFile, emit_with_strategy};
use pons_ir::{
    ByteEncoding, Direction, EnumRepr, InlineType, IntegerWire, NamedRef, PathSegment, Presence,
    ResponseHeader, TypeKind, TypeRef, Variant, VariantPayload,
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

fn run_bun(files: &[GeneratedFile], harness: &str) {
    let Ok(version) = Command::new("bun").arg("--version").output() else {
        return;
    };
    assert!(version.status.success());
    let directory = tempfile::tempdir().unwrap();
    for file in files {
        fs::write(directory.path().join(&file.path), &file.content).unwrap();
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
    assert!(client.content.contains("function validateWire"));

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
    assert!(error.0.contains("fractional integer constraint"));
    assert!(error.0.contains("types.User.fields[0].ty.multiple_of"));
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
    let client = files.iter().find(|file| file.path == "client.ts").unwrap();
    assert!(client.content.contains("function validateInteger"));

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
fn generated_client_preflights_request_inputs() {
    let optional = request_input_document(false);
    let optional_files = emit_with_strategy(&optional, EmitMode::Strict, ErrorStrategy::Throw)
        .unwrap()
        .files;
    let optional_client = optional_files
        .iter()
        .find(|file| file.path == "client.ts")
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
fn generated_client_transforms_untagged_byte_payloads() {
    let document = untagged_bytes_document();
    let files = emit_with_strategy(&document, EmitMode::Strict, ErrorStrategy::Throw)
        .unwrap()
        .files;
    let client = files.iter().find(|file| file.path == "client.ts").unwrap();
    assert!(client.content.contains("selectUntaggedVariant"));

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
