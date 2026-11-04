mod support;

use std::collections::BTreeMap;

use pons_emit_ts::{EmitMode, ErrorStrategy, emit_with_strategy};
use pons_ir::{InlineType, IntegerWire, Presence, ResponseHeader, TypeKind, TypeRef};
use support::runtime::{constrained_integer, run_bun, validated_files};

#[test]
fn rejects_fractional_integer_constraints() {
    let mut document: pons_ir::Document =
        serde_json::from_str(include_str!("../../../spec/fixtures/login.json")).unwrap();
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
fn generated_client_enforces_integer_constraints() {
    let mut document: pons_ir::Document =
        serde_json::from_str(include_str!("../../../spec/fixtures/login.json")).unwrap();
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
fn counts_unicode_code_points_in_string_length_bounds() {
    let mut raw: serde_json::Value =
        serde_json::from_str(include_str!("../../../spec/fixtures/login.json")).unwrap();
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

fn string_wire_integer_document() -> pons_ir::Document {
    let mut raw: serde_json::Value =
        serde_json::from_str(include_str!("../../../spec/fixtures/login.json")).unwrap();
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
        serde_json::from_str(include_str!("../../../spec/fixtures/login.json")).unwrap();
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
fn an_uncompilable_pattern_is_refused_before_any_client_exists() {
    let fixture: pons_ir::Document =
        serde_json::from_str(include_str!("../../../spec/fixtures/login.json")).unwrap();

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
        serde_json::from_str(include_str!("../../../spec/fixtures/login.json")).unwrap();
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
        serde_json::from_str(include_str!("../../../spec/fixtures/login.json")).unwrap();
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
