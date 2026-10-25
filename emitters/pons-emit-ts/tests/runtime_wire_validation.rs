mod support;

use std::collections::BTreeMap;

use pons_emit_ts::{EmitMode, ErrorStrategy, emit_with_strategy};
use pons_ir::{Direction, InlineType, IntegerWire, Presence, ResponseHeader, TypeKind, TypeRef};
use support::runtime::{constrained_integer, run_bun, validated_files};

#[test]
fn generated_client_rejects_missing_fields_and_unknown_enum_variants() {
    let document: pons_ir::Document =
        serde_json::from_str(include_str!("../../../spec/fixtures/login.json")).unwrap();
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
        serde_json::from_str(include_str!("../../../spec/fixtures/login.json")).unwrap();
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
        serde_json::from_str(include_str!("../../../spec/fixtures/login.json")).unwrap();
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
        serde_json::from_str(include_str!("../../../spec/fixtures/login.json")).unwrap();
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
fn refuses_enum_variants_with_unrepresentable_flatten() {
    let mut raw: serde_json::Value =
        serde_json::from_str(include_str!("../../../spec/fixtures/login.json")).unwrap();
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
fn generated_client_validates_required_fields_by_direction_and_access() {
    let mut raw: serde_json::Value =
        serde_json::from_str(include_str!("../../../spec/fixtures/login.json")).unwrap();
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
        serde_json::from_str(include_str!("../../../spec/fixtures/login.json")).unwrap();
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
        serde_json::from_str(include_str!("../../../spec/fixtures/login.json")).unwrap();
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
fn drops_direction_invisible_fields_from_preserved_objects() {
    let mut raw: serde_json::Value =
        serde_json::from_str(include_str!("../../../spec/fixtures/login.json")).unwrap();
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
        serde_json::from_str(include_str!("../../../spec/fixtures/login.json")).unwrap();
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
        serde_json::from_str(include_str!("../../../spec/fixtures/login.json")).unwrap();
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
fn a_text_body_rejects_the_unknown_properties_a_json_body_rejects() {
    let mut raw: serde_json::Value =
        serde_json::from_str(include_str!("../../../spec/fixtures/login.json")).unwrap();
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
        serde_json::from_str(include_str!("../../../spec/fixtures/login.json")).unwrap();
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
