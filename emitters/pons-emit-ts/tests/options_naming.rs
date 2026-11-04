mod support;

use pons_emit_ts::{ErrorStrategy, FieldCase, TsOptions, emit_with_options};
use pons_ir::Document;
use serde_json::{Value, json};
use support::options::{emitted, file, fixture, run_bun};

fn camel_document() -> Document {
    let fields = json!([
        {"wire": "user_id", "ident": ["user", "id"], "ty": {"kind": "string"}, "presence": "required"},
        {"wire": "retry_after", "ident": ["retry", "after"], "ty": {"kind": "int", "bits": 32, "signed": false, "wire": "number"}, "presence": "required"}
    ]);
    let mut raw = fixture();
    raw["types"]["User"] = json!({
        "kind": "struct", "name": ["user"], "direction": "output", "openness": "preserve_opaque", "fields": fields
    });
    raw["types"]["LoginRequest"] = json!({
        "kind": "struct", "name": ["login", "request"], "direction": "input", "openness": "reject", "fields": fields
    });
    raw["operations"][0]["provides"] = Value::Null;
    let document: Document = serde_json::from_value(raw).unwrap();
    document.validate().unwrap();
    document
}

#[test]
fn field_case_camel_names_a_member_camel_and_keeps_the_wire_name() {
    let document = camel_document();
    let files = emitted(
        &document,
        TsOptions {
            field_case: FieldCase::Camel,
            errors: ErrorStrategy::Throw,
            ..TsOptions::default()
        },
    );
    assert!(
        file(&files, "types.ts")
            .contains("export type User = { \"retryAfter\": number; \"userId\": string; [key: string]: unknown };"),
        "{}",
        file(&files, "types.ts")
    );
    assert!(
        file(&files, "types.ts").contains("\\\"key\\\":\\\"userId\\\""),
        "the runtime reads the member it is given:\n{}",
        file(&files, "types.ts")
    );
    assert!(
        file(&files, "types.ts").contains("export type EventQuery = { \"limit\"?: number };"),
        "a member that is already camel does not change:\n{}",
        file(&files, "types.ts")
    );
    run_bun(
        &files,
        r#"
import { createClient } from "./client.ts";

let sent = "";
const client = createClient({
  baseUrl: "http://example.test",
  fetch: async (_url, init) => {
    sent = String(init?.body);
    return new Response('{"user_id":"u-1","retry_after":30,"trace_id":"t-1"}', {
      status: 200,
      headers: { "content-type": "application/json" },
    });
  },
});
const value = await client.auth.login({ body: { userId: "u-1", retryAfter: 30 } });
if (sent !== '{"retry_after":30,"user_id":"u-1"}') throw new Error(`the wire carries ${sent}`);
if (value.userId !== "u-1" || value.retryAfter !== 30) {
  throw new Error(`the answer is ${JSON.stringify(value)}`);
}
if (value.trace_id !== "t-1") throw new Error("an undeclared member did not survive");
if ("user_id" in value) throw new Error("the answer kept the wire spelling");
"#,
    );
    run_bun(
        &files,
        r#"
import { createClient } from "./client.ts";

const client = createClient({
  baseUrl: "http://example.test",
  fetch: async () => new Response('{"user_id":"u-1","retry_after":30}', {
    status: 200,
    headers: { "content-type": "application/json" },
  }),
});
let failure: any;
try {
  // @ts-ignore the member the caller can name is userId
  await client.auth.login({ body: { user_id: "u-1", retry_after: 30 } });
} catch (cause) {
  failure = cause;
}
if (failure === undefined) throw new Error("a wire name in the body was accepted");
if (failure._tag !== "RequestError" || !failure.message.includes("unknown property user_id")) {
  throw new Error(JSON.stringify(failure));
}
"#,
    );
}

#[test]
fn field_case_camel_refuses_two_members_that_spell_the_same_way() {
    let mut raw = fixture();
    raw["types"]["User"] = json!({
        "kind": "struct", "name": ["user"], "direction": "output", "openness": "reject",
        "fields": [
            {"wire": "retry_after", "ident": ["retry", "after"], "ty": {"kind": "string"}, "presence": "required"},
            {"wire": "retryAfter", "ident": ["retry", "after"], "ty": {"kind": "string"}, "presence": "required"}
        ]
    });
    let document: Document = serde_json::from_value(raw).unwrap();
    document.validate().unwrap();
    let error = emit_with_options(
        &document,
        TsOptions {
            field_case: FieldCase::Camel,
            ..TsOptions::default()
        },
    )
    .unwrap_err();
    assert_eq!(
        error.0,
        "the TypeScript emitter cannot spell the fields of the type User in camelCase, because \
         the wire names retryAfter and retry_after both spell retryAfter in the emitted client"
    );
    assert!(
        emit_with_options(&document, TsOptions::default()).is_ok(),
        "the two members are only one member under the option that keeps the wire name"
    );
}
