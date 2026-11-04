mod support;

use pons_emit_ts::{EmitMode, ErrorStrategy, Int64, TsOptions, emit_with_options};
use pons_ir::Document;
use serde_json::{Value, json};
use support::options::{emitted, file, fixture, run_bun};

fn wide_document() -> Document {
    let mut raw = fixture();
    raw["types"]["User"] = json!({
        "kind": "struct", "name": ["user"], "direction": "output", "openness": "reject",
        "fields": [
            {"wire": "count", "ident": ["count"], "ty": {"kind": "int", "bits": 64, "signed": false, "wire": "number"}, "presence": "required"}
        ]
    });
    raw["types"]["LoginRequest"] = json!({
        "kind": "struct", "name": ["login", "request"], "direction": "input", "openness": "reject",
        "fields": [
            {"wire": "count", "ident": ["count"], "ty": {"kind": "int", "bits": 64, "signed": false, "wire": "number"}, "presence": "required"}
        ]
    });
    raw["operations"][0]["provides"] = Value::Null;
    let document: Document = serde_json::from_value(raw).unwrap();
    document.validate().unwrap();
    document
}

#[test]
fn int64_bigint_holds_a_wire_number_a_javascript_number_cannot() {
    let document = wide_document();
    let files = emitted(
        &document,
        TsOptions {
            errors: ErrorStrategy::Throw,
            ..TsOptions::default()
        },
    );
    let types = file(&files, "types.ts");
    assert!(types.contains("\"count\": bigint"), "{types}");
    assert!(
        file(&files, "operations/auth.ts").contains("preserveWideIntegers: true"),
        "the lossless reader is the only way a bigint holds every digit:\n{}",
        file(&files, "operations/auth.ts")
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
    return new Response('{"count":9007199254740993}', {
      status: 200,
      headers: { "content-type": "application/json" },
    });
  },
});
const value = await client.auth.login({ body: { count: 9007199254740993n } });
if (sent !== '{"count":9007199254740993}') throw new Error(`the wire carries ${sent}`);
if (typeof value.count !== "bigint") throw new Error(`the answer is a ${typeof value.count}`);
if (value.count !== 9007199254740993n) throw new Error(`the answer is ${value.count}`);
"#,
    );
}

#[test]
fn int64_number_refuses_a_wide_integer_in_strict_mode_and_loses_it_in_lossy() {
    let document = wide_document();
    let number = TsOptions {
        int64: Int64::Number,
        ..TsOptions::default()
    };
    let refused = emit_with_options(&document, number.clone()).unwrap_err();
    assert_eq!(
        refused.0,
        "the TypeScript emitter reported lossy coverage in strict mode: \
         operations.auth.login, operations.events.watch, types.LoginRequest, types.User",
        "a number cannot hold every digit, so strict mode says so instead of emitting one:\n{}",
        refused.0
    );

    let lossy = emit_with_options(
        &document,
        TsOptions {
            mode: EmitMode::Lossy,
            errors: ErrorStrategy::Throw,
            ..number
        },
    )
    .unwrap();
    assert!(
        lossy
            .warnings
            .iter()
            .any(|warning| warning.contains("wide numeric integers as number")),
        "{:?}",
        lossy.warnings
    );
    let files = lossy.files;
    assert!(file(&files, "types.ts").contains("\"count\": number"));
    assert!(
        !file(&files, "operations/auth.ts").contains("preserveWideIntegers"),
        "the lossless reader is what a number cannot use:\n{}",
        file(&files, "operations/auth.ts")
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
    return new Response('{"count":9007199254740993}', {
      status: 200,
      headers: { "content-type": "application/json" },
    });
  },
});
const value = await client.auth.login({ body: { count: 9007199254740993 } });
if (typeof value.count !== "number") throw new Error(`the answer is a ${typeof value.count}`);
if (value.count !== 9007199254740992) throw new Error(`the answer is ${value.count}`);
if (sent !== '{"count":9007199254740992}') {
  throw new Error(`the literal 9007199254740993 is already 9007199254740992, so the request rounds too: ${sent}`);
}
"#,
    );
}

#[test]
fn int64_string_carries_a_wide_integer_as_a_canonical_decimal_string() {
    let document = wide_document();
    let files = emitted(
        &document,
        TsOptions {
            int64: Int64::String,
            errors: ErrorStrategy::Throw,
            ..TsOptions::default()
        },
    );
    let types = file(&files, "types.ts");
    assert!(types.contains("\"count\": string"), "{types}");
    assert!(
        types.contains("\\\"carrier\\\":\\\"string\\\""),
        "the runtime is told which representation the caller holds:\n{types}"
    );
    assert!(
        file(&files, "operations/auth.ts").contains("preserveWideIntegers: true"),
        "the lossless reader is the only way a decimal string holds every digit:\n{}",
        file(&files, "operations/auth.ts")
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
    return new Response('{"count":9007199254740993}', {
      status: 200,
      headers: { "content-type": "application/json" },
    });
  },
});
const value = await client.auth.login({ body: { count: "9007199254740993" } });
if (sent !== '{"count":9007199254740993}') throw new Error(`the wire carries ${sent}, a JSON string`);
if (typeof value.count !== "string" || value.count !== "9007199254740993") {
  throw new Error(`the answer is ${String(value.count)}`);
}
if (BigInt(value.count) !== 9007199254740993n) throw new Error("the answer lost a digit");
"#,
    );
    run_bun(
        &files,
        r#"
import { createClient } from "./client.ts";

const client = createClient({
  baseUrl: "http://example.test",
  fetch: async () => new Response("{}", { status: 200, headers: { "content-type": "application/json" } }),
});
for (const count of ["9007199254740993.0", " 12", "12 ", "+1", "", "0x10"]) {
  let failure: any;
  try {
    await client.auth.login({ body: { count } });
  } catch (cause) {
    failure = cause;
  }
  if (failure?.message !== "expected integer string") {
    throw new Error(`${JSON.stringify(count)} was accepted: ${JSON.stringify(failure)}`);
  }
}
let number: any;
try {
  // @ts-ignore the option says the member is a string
  await client.auth.login({ body: { count: 9007199254740993 } });
} catch (cause) {
  number = cause;
}
if (number?.message !== "expected integer string") throw new Error(JSON.stringify(number));
"#,
    );
}
