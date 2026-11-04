mod support;

use pons_emit_ts::{EmitMode, ErrorStrategy, emit_with_strategy};
use support::runtime::run_bun;

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
