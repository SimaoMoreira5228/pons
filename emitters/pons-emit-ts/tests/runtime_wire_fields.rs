mod support;

use pons_emit_ts::{EmitMode, ErrorStrategy, emit_with_strategy};
use pons_ir::Direction;
use support::runtime::run_bun;

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
