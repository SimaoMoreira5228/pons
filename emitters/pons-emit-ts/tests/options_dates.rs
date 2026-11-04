mod support;

use pons_emit_ts::{DateMode, ErrorStrategy, FieldCase, TsOptions};
use pons_ir::Document;
use serde_json::{Value, json};
use support::options::{emitted, file, fixture, run_bun};

fn date_document() -> Document {
    let fields = json!([
        {"wire": "day", "ident": ["day"], "ty": {"kind": "string", "format": "date"}, "presence": "required"},
        {"wire": "at", "ident": ["at"], "ty": {"kind": "string", "format": "date-time"}, "presence": "required"}
    ]);
    let mut raw = fixture();
    raw["types"]["User"] = json!({
        "kind": "struct", "name": ["user"], "direction": "output", "openness": "reject", "fields": fields
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
fn date_mode_date_reads_a_day_and_an_instant_differently_and_writes_both_back() {
    let document = date_document();
    let files = emitted(
        &document,
        TsOptions {
            date_mode: DateMode::Date,
            ..TsOptions::default()
        },
    );
    let types = file(&files, "types.ts");
    assert!(
        types.contains("export type DateOnly = string & { readonly [brandDateOnly]: never };"),
        "a calendar day is a branded string, because no Date can hold a day:\n{types}"
    );
    assert!(
        types.contains("export type User = { \"at\": Date; \"day\": DateOnly };"),
        "{types}"
    );
    assert!(
        types.contains("\\\"carrier\\\":\\\"date\\\""),
        "the runtime is told the date-time is an instant:\n{types}"
    );
    run_bun(
        &files,
        r#"
import { createClient } from "./client.ts";
import type { DateOnly } from "./types.ts";

let sent = "";
let answer = '{"day":"2024-01-31","at":"2024-01-31T12:00:00.000Z"}';
const client = createClient({
  baseUrl: "http://example.test",
  fetch: async (_url, init) => {
    sent = String(init?.body);
    return new Response(answer, { status: 200, headers: { "content-type": "application/json" } });
  },
});
const first = await client.auth.login({
  body: { day: "2024-01-31" as DateOnly, at: new Date("2024-01-31T12:00:00Z") },
});
if (!first.isOk()) throw new Error(`the call failed: ${JSON.stringify(first.error)}`);
if (first.value.day !== "2024-01-31") throw new Error(`the day is ${String(first.value.day)}`);
if (!(first.value.at instanceof Date)) throw new Error("the instant is not a Date");
if (first.value.at.toISOString() !== "2024-01-31T12:00:00.000Z") {
  throw new Error(`the instant is ${first.value.at.toISOString()}`);
}
if (sent !== '{"at":"2024-01-31T12:00:00.000Z","day":"2024-01-31"}') {
  throw new Error(`the wire carries ${sent}`);
}

answer = '{"day":"2024-01-31","at":"2024-01-31T12:00:00.123456Z"}';
const finer = await client.auth.login({ body: { day: "2024-01-31" as DateOnly, at: new Date() } });
if (finer.isOk()) throw new Error("a nanosecond date-time was read as a Date");
if ((finer as any).error._tag !== "DecodeError" || !(finer as any).error.issues.some((issue: string) => issue.includes("not an instant a Date can hold"))) {
  throw new Error(JSON.stringify(finer));
}
"#,
    );
}

#[test]
fn date_mode_date_refuses_a_day_the_option_does_not_carry_as_a_date() {
    let document = date_document();
    let files = emitted(
        &document,
        TsOptions {
            date_mode: DateMode::Date,
            errors: ErrorStrategy::Throw,
            ..TsOptions::default()
        },
    );
    run_bun(
        &files,
        r#"
import { createClient } from "./client.ts";

const client = createClient({
  baseUrl: "http://example.test",
  fetch: async () => new Response("{}", { status: 200, headers: { "content-type": "application/json" } }),
});
let asText: any;
try {
  await client.auth.login({ body: { day: "2024-01-31" as never, at: "2024-01-31T12:00:00Z" as never } });
} catch (cause) {
  asText = cause;
}
if (asText?.message !== "expected a Date") throw new Error(`a date-time went out as text: ${JSON.stringify(asText)}`);

let asDate: any;
try {
  await client.auth.login({ body: { day: new Date() as never, at: new Date() } });
} catch (cause) {
  asDate = cause;
}
if (asDate?.message !== "expected string") throw new Error(`a day went out as a Date: ${JSON.stringify(asDate)}`);
"#,
    );
}

#[test]
fn a_declared_member_is_carried_once_when_its_key_changes_under_a_typed_openness() {
    let mut raw = fixture();
    for (id, name, direction) in [
        ("User", "user", "output"),
        ("LoginRequest", "login request", "input"),
    ] {
        let name = name
            .split(' ')
            .map(|word| word.to_string())
            .collect::<Vec<_>>();
        raw["types"][id] = json!({
            "kind": "struct", "name": name, "direction": direction,
            "openness": {"preserve_typed": {"value": {"kind": "string", "format": "date-time"}}},
            "fields": [
                {"wire": "seen_at", "ident": ["seen", "at"], "ty": {"kind": "string", "format": "date-time"}, "presence": "required"}
            ]
        });
    }
    raw["operations"][0]["provides"] = Value::Null;
    let document: Document = serde_json::from_value(raw).unwrap();
    document.validate().unwrap();
    let files = emitted(
        &document,
        TsOptions {
            field_case: FieldCase::Camel,
            date_mode: DateMode::Date,
            errors: ErrorStrategy::Throw,
            ..TsOptions::default()
        },
    );
    assert!(
        file(&files, "types.ts")
            .contains("export type User = { \"seenAt\": Date; [key: string]: Date };"),
        "{}",
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
    return new Response('{"seen_at":"2024-01-31T12:00:00Z","also_at":"2024-01-31T12:00:00Z"}', {
      status: 200,
      headers: { "content-type": "application/json" },
    });
  },
});
const value = await client.auth.login({ body: { seenAt: new Date("2024-01-31T12:00:00Z") } });
if (sent !== '{"seen_at":"2024-01-31T12:00:00.000Z"}') throw new Error(`the wire carries ${sent}`);
if (!(value.seenAt instanceof Date)) throw new Error("the declared member is not a Date");
if (!(value.also_at instanceof Date) || value.also_at.toISOString() !== "2024-01-31T12:00:00.000Z") {
  throw new Error(`an undeclared member is a ${typeof value.also_at}`);
}
"#,
    );
}
