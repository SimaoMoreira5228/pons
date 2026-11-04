mod support;

use pons_ir::Document;
use serde_json::Value;

use support::type_check::{
    client_of, files, files_with, fixture, operations_of, type_checks, types_of,
};

fn documented() -> Value {
    let mut raw = fixture();
    raw["types"]["User"]["doc"] = serde_json::json!({
        "summary": "A person",
        "description": "Ends the comment early */ and then\ncarries on"
    });
    raw["types"]["User"]["fields"][1]["doc"] =
        serde_json::json!({"summary": "How they would like to be called."});
    raw["types"]["LoginRequest"]["doc"] =
        serde_json::json!({"summary": "Credentials", "deprecated": true});
    raw["types"]["LoginError"]["variants"][0]["doc"] = serde_json::json!({
        "summary": "The credentials were refused.",
        "deprecated": true
    });
    raw["types"]["LoginError"]["variants"][1]["payload"]["fields"][0]["doc"] =
        serde_json::json!({"summary": "How long to wait."});
    raw["operations"][0]["doc"] = serde_json::json!({
        "summary": "Exchange a password for a session.",
        "description": "The session token arrives in the body."
    });
    raw["operations"][0]["errors"][0]["doc"] =
        serde_json::json!({"summary": "The credentials did not match."});
    raw["common_errors"] = serde_json::json!([{
        "status": "5xx",
        "name": ["server"],
        "content": "text",
        "body": {"kind": "string"},
        "doc": {"summary": "The server failed."}
    }]);
    raw
}

#[test]
fn a_user_type_never_shadows_the_syntax_the_emitter_prints() {
    let mut raw = fixture();
    raw["types"]["Array"] = serde_json::json!({
        "kind": "struct", "name": ["array"], "direction": "both", "openness": "reject",
        "fields": [{"wire": "a", "ident": ["a"], "ty": {"kind": "string"}, "presence": "required"}]
    });
    raw["types"]["Record"] = serde_json::json!({
        "kind": "struct", "name": ["record"], "direction": "both", "openness": "reject",
        "fields": [{"wire": "b", "ident": ["b"], "ty": {"kind": "string"}, "presence": "required"}]
    });
    raw["types"]["Extract"] = serde_json::json!({
        "kind": "struct", "name": ["extract"], "direction": "both", "openness": "reject",
        "fields": [{"wire": "c", "ident": ["c"], "ty": {"kind": "string"}, "presence": "required"}]
    });
    raw["types"]["Uint8Array"] = serde_json::json!({
        "kind": "struct", "name": ["uint8", "array"], "direction": "both", "openness": "reject",
        "fields": [{"wire": "d", "ident": ["d"], "ty": {"kind": "string"}, "presence": "required"}]
    });
    raw["types"]["Holder"] = serde_json::json!({
        "kind": "struct", "name": ["holder"], "direction": "output", "openness": "reject",
        "fields": [
            {"wire": "list", "ident": ["list"], "ty": {"kind": "array", "item": {"kind": "string"}}, "presence": "required"},
            {"wire": "dict", "ident": ["dict"], "ty": {"kind": "map", "key": {"kind": "string"}, "value": {"kind": "int", "bits": 16, "signed": false, "wire": "number"}}, "presence": "required"},
            {"wire": "blob", "ident": ["blob"], "ty": {"kind": "bytes", "encoding": "base64"}, "presence": "required"}
        ]
    });
    raw["operations"][0]["responses"][0]["body"] = serde_json::json!({"ref": "Holder"});
    raw["operations"][0]["provides"] = serde_json::Value::Null;
    let document: Document = serde_json::from_value(raw).unwrap();
    let files = files(&document);
    let types = types_of(&files);
    for reserved in ["Array", "Record", "Extract", "Uint8Array"] {
        assert!(
            !types.contains(&format!("export type {reserved} =")),
            "{reserved} kept the name the emitter prints it beside:\n{types}"
        );
    }
    for suffixed in ["Array2", "Record2", "Extract2", "Uint8Array2"] {
        assert!(
            types.contains(&format!("export type {suffixed} =")),
            "{suffixed} is missing:\n{types}"
        );
    }

    assert!(types.contains("\"blob\": Uint8Array;"), "{types}");
    type_checks(
        &files,
        r#"
import { createClient } from "./client.js";

const client = createClient({ baseUrl: "https://api.example.test" });
const value = await client.auth.login({ body: { email: "person@example.test" } });
const bytes: Uint8Array = value.blob;
const list: string[] = value.list;
const dict: Record<string, number> = value.dict;
// @ts-expect-error the bytes field is not the user's Uint8Array struct
const wrong: { d: string } = value.blob;
console.log(bytes.length, list, dict, wrong);
"#,
    );
}

#[test]
fn a_documented_contract_compiles_under_the_strictest_flags() {
    let document: Document = serde_json::from_value(documented()).unwrap();
    let files = files(&document);
    for source in [
        types_of(&files),
        client_of(&files),
        operations_of(&files, "auth"),
        operations_of(&files, "events"),
    ] {
        assert!(
            source.contains("/**"),
            "the document is documented:\n{source}"
        );
        assert_eq!(
            source.matches("/**").count(),
            source.matches("*/").count(),
            "a summary that ends a comment early leaves every comment closed:\n{source}"
        );
        assert!(
            !source.contains("\n */"),
            "a comment ends on the line it starts:\n{source}"
        );
    }
    assert!(
        types_of(&files)
            .contains("/** A person Ends the comment early *\\/ and then carries on */"),
        "{}",
        types_of(&files)
    );
    type_checks(
        &files,
        r#"
import { createClient, type AuthLoginError } from "./client.js";

const client = createClient({ baseUrl: "https://api.example.test" });
const loggedIn = await client.auth.login({ body: { email: "person@example.test" } });
const failure: AuthLoginError | undefined = undefined;
console.log(loggedIn, failure);
"#,
    );
}

fn tagged() -> Document {
    let mut raw = fixture();
    raw["types"]["Tag"] = serde_json::json!({
        "kind": "newtype", "name": ["tag"], "direction": "both", "inner": {"kind": "string"}
    });
    raw["types"]["Session"] = serde_json::json!({
        "kind": "struct", "name": ["session"], "direction": "both", "openness": "reject",
        "fields": [
            {"wire": "tag", "ident": ["tag"], "ty": {"ref": "Tag"}, "presence": "required"}
        ]
    });
    raw["types"]["LoginRequest"] = serde_json::json!({
        "kind": "struct", "name": ["login", "request"], "direction": "input", "openness": "reject",
        "fields": [
            {"wire": "tag", "ident": ["tag"], "ty": {"ref": "Tag"}, "presence": "required"}
        ]
    });
    raw["operations"][0]["responses"][0]["body"] = serde_json::json!({"ref": "Session"});
    raw["operations"][0]["provides"] = serde_json::Value::Null;
    serde_json::from_value(raw).unwrap()
}

#[test]
fn a_branded_newtype_is_nominal_where_an_alias_is_not() {
    let document = tagged();
    let branded_files = files_with(
        &document,
        pons_emit_ts::TsOptions {
            newtypes: pons_emit_ts::Newtypes::Branded,
            ..pons_emit_ts::TsOptions::default()
        },
    );
    assert!(
        types_of(&branded_files)
            .contains("export type Tag = string & { readonly [brandTag]: never };"),
        "{}",
        types_of(&branded_files)
    );
    type_checks(
        &branded_files,
        r#"
import { createClient } from "./client.js";
import type { Tag } from "./types.js";

const client = createClient({ baseUrl: "https://api.example.test" });
const session = await client.auth.login({ body: { tag: "abc" as Tag } });
export const branded: Tag = session.tag;
const plain: string = session.tag;
// @ts-expect-error a plain string is not a branded newtype
export const forged: Tag = "abc";
// @ts-expect-error the brand does not leak onto the alias either
export const swapped: string & { readonly [brandTag]: never } = branded;
console.log(branded, plain, forged, swapped);
"#,
    );
    let aliased = files(&document);
    type_checks(
        &aliased,
        r#"
import { createClient } from "./client.js";
import type { Tag } from "./types.js";

const client = createClient({ baseUrl: "https://api.example.test" });
const session = await client.auth.login({ body: { tag: "abc" as Tag } });
export const plain: string = session.tag;
export const alias: Tag = session.tag;
console.log(plain, alias);
"#,
    );
}

fn camel_document() -> Document {
    let fields = serde_json::json!([
        {"wire": "user_id", "ident": ["user", "id"], "ty": {"kind": "string"}, "presence": "required"},
        {"wire": "retry_after", "ident": ["retry", "after"], "ty": {"kind": "int", "bits": 32, "signed": false, "wire": "number"}, "presence": "required"}
    ]);
    let mut raw = fixture();
    raw["types"]["User"] = serde_json::json!({
        "kind": "struct", "name": ["user"], "direction": "output", "openness": "reject", "fields": fields
    });
    raw["types"]["LoginRequest"] = serde_json::json!({
        "kind": "struct", "name": ["login", "request"], "direction": "input", "openness": "reject", "fields": fields
    });
    raw["operations"][0]["provides"] = serde_json::Value::Null;
    serde_json::from_value(raw).unwrap()
}

#[test]
fn a_camel_member_is_the_only_name_a_caller_may_write() {
    let document = camel_document();
    let camel = files_with(
        &document,
        pons_emit_ts::TsOptions {
            field_case: pons_emit_ts::FieldCase::Camel,
            ..pons_emit_ts::TsOptions::default()
        },
    );
    type_checks(
        &camel,
        r#"
import { createClient } from "./client.js";
import type { User } from "./types.js";

const client = createClient({ baseUrl: "https://api.example.test" });
const value = await client.auth.login({ body: { userId: "u-1", retryAfter: 30 } });
const id: string = value.userId;
const after: number = value.retryAfter;
const user: User = { userId: "u-1", retryAfter: 30 };
// @ts-expect-error the wire name is not a member of the emitted type
const wire: User = { user_id: "u-1", retry_after: 30 };
// @ts-expect-error the caller cannot read a member the wire spells
const leaked: string = value.user_id;
console.log(id, after, user, wire, leaked);
"#,
    );
    let aliased = files(&document);
    type_checks(
        &aliased,
        r#"
import { createClient } from "./client.js";

const client = createClient({ baseUrl: "https://api.example.test" });
const value = await client.auth.login({ body: { user_id: "u-1", retry_after: 30 } });
// @ts-expect-error the wire case has no camel member
const camel: string = value.userId;
console.log(value.user_id, camel);
"#,
    );
}

fn wide_document() -> Document {
    let mut raw = fixture();
    raw["types"]["User"] = serde_json::json!({
        "kind": "struct", "name": ["user"], "direction": "output", "openness": "reject",
        "fields": [
            {"wire": "count", "ident": ["count"], "ty": {"kind": "int", "bits": 64, "signed": false, "wire": "number"}, "presence": "required"}
        ]
    });
    raw["types"]["LoginRequest"] = serde_json::json!({
        "kind": "struct", "name": ["login", "request"], "direction": "input", "openness": "reject",
        "fields": [
            {"wire": "count", "ident": ["count"], "ty": {"kind": "int", "bits": 64, "signed": false, "wire": "number"}, "presence": "required"}
        ]
    });
    raw["operations"][0]["provides"] = serde_json::Value::Null;
    serde_json::from_value(raw).unwrap()
}

#[test]
fn a_wide_integer_is_a_string_where_the_option_says_so() {
    let document = wide_document();
    let files = files_with(
        &document,
        pons_emit_ts::TsOptions {
            int64: pons_emit_ts::Int64::String,
            ..pons_emit_ts::TsOptions::default()
        },
    );
    type_checks(
        &files,
        r#"
import { createClient } from "./client.js";

const client = createClient({ baseUrl: "https://api.example.test" });
const value = await client.auth.login({ body: { count: "9007199254740993" } });
const exact: string = value.count;
const digits: bigint = BigInt(value.count);
// @ts-expect-error the option says the member is the string, not a bigint
const wrong: bigint = value.count;
// @ts-expect-error a number is not the string the option declares
await client.auth.login({ body: { count: 9007199254740993n } });
console.log(exact, digits, wrong);
"#,
    );
}

fn date_document() -> Document {
    let fields = serde_json::json!([
        {"wire": "day", "ident": ["day"], "ty": {"kind": "string", "format": "date"}, "presence": "required"},
        {"wire": "at", "ident": ["at"], "ty": {"kind": "string", "format": "date-time"}, "presence": "required"}
    ]);
    let mut raw = fixture();
    raw["types"]["User"] = serde_json::json!({
        "kind": "struct", "name": ["user"], "direction": "output", "openness": "reject", "fields": fields
    });
    raw["types"]["LoginRequest"] = serde_json::json!({
        "kind": "struct", "name": ["login", "request"], "direction": "input", "openness": "reject", "fields": fields
    });
    raw["operations"][0]["provides"] = serde_json::Value::Null;
    serde_json::from_value(raw).unwrap()
}

#[test]
fn a_day_is_a_branded_string_and_an_instant_is_a_date() {
    let document = date_document();
    let carried = files_with(
        &document,
        pons_emit_ts::TsOptions {
            date_mode: pons_emit_ts::DateMode::Date,
            ..pons_emit_ts::TsOptions::default()
        },
    );
    type_checks(
        &carried,
        r#"
import { createClient } from "./client.js";
import type { DateOnly } from "./types.js";

const client = createClient({ baseUrl: "https://api.example.test" });
const value = await client.auth.login({
  body: { day: "2024-01-31" as DateOnly, at: new Date("2024-01-31T12:00:00Z") },
});
const day: DateOnly = value.day;
const text: string = value.day;
const instant: Date = value.at;
const millis: number = value.at.getTime();
// @ts-expect-error a plain string is not a calendar day
const forged: DateOnly = "2024-01-31";
// @ts-expect-error a day is not an instant
const wrongInstant: Date = value.day;
// @ts-expect-error an instant is not a calendar day
const wrongDay: DateOnly = new Date();
// @ts-expect-error a date-time member is a Date, not a string
await client.auth.login({ body: { day: "2024-01-31" as DateOnly, at: "2024-01-31T12:00:00Z" } });
console.log(day, text, instant, millis, forged, wrongInstant, wrongDay);
"#,
    );
    let wire = files(&document);
    type_checks(
        &wire,
        r#"
import { createClient } from "./client.js";

const client = createClient({ baseUrl: "https://api.example.test" });
const value = await client.auth.login({ body: { day: "2024-01-31", at: "2024-01-31T12:00:00Z" } });
// @ts-expect-error the default options carry the wire string, not a Date
const instant: Date = value.at;
console.log(value.day, instant);
"#,
    );
}
