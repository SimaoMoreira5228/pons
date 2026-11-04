mod support;

use pons_emit_ts::{ErrorStrategy, Newtypes, TsOptions};
use pons_ir::Document;
use serde_json::{Value, json};
use support::options::{emitted, file_of, fixture, run_bun};

fn tagged_document() -> Document {
    let mut raw = fixture();
    raw["types"]["Tag"] = json!({
        "kind": "newtype", "name": ["tag"], "direction": "both", "inner": {"kind": "string"}
    });
    raw["types"]["Session"] = json!({
        "kind": "struct", "name": ["session"], "direction": "both", "openness": "reject",
        "fields": [
            {"wire": "tag", "ident": ["tag"], "ty": {"ref": "Tag"}, "presence": "required"},
            {"wire": "alias", "ident": ["alias"], "ty": {"ref": "Tag"}, "presence": "required"}
        ]
    });
    raw["types"]["LoginRequest"] = json!({
        "kind": "struct", "name": ["login", "request"], "direction": "input", "openness": "reject",
        "fields": [
            {"wire": "tag", "ident": ["tag"], "ty": {"ref": "Tag"}, "presence": "required"}
        ]
    });
    raw["operations"][0]["responses"][0]["body"] = json!({"ref": "Session"});
    raw["operations"][0]["provides"] = Value::Null;
    let document: Document = serde_json::from_value(raw).unwrap();
    document.validate().unwrap();
    document
}

#[test]
fn a_newtype_is_an_alias_today_and_nominal_under_the_branded_option() {
    let document = tagged_document();
    let alias = file_of(&document, TsOptions::default(), "types.ts");
    assert!(
        alias.contains("export type Tag = string;") && !alias.contains("brandTag"),
        "{alias}"
    );

    let branded = file_of(
        &document,
        TsOptions {
            newtypes: Newtypes::Branded,
            ..TsOptions::default()
        },
        "types.ts",
    );
    assert!(
        branded.contains("declare const brandTag: unique symbol;\nexport type Tag = string & { readonly [brandTag]: never };"),
        "{branded}"
    );
    assert!(
        branded.contains("\"alias\": Tag; \"tag\": Tag"),
        "a field keeps the newtype it declares:\n{branded}"
    );
    assert!(
        branded.contains("\"tag\": Tag };"),
        "a request argument takes the branded type too:\n{branded}"
    );
    assert!(
        branded.contains("export const typeDefinitions"),
        "the wire metadata the runtime reads is the wire, so the brand does not reach it:\n{branded}"
    );

    let files = emitted(
        &document,
        TsOptions {
            newtypes: Newtypes::Branded,
            errors: ErrorStrategy::Throw,
            ..TsOptions::default()
        },
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
    return new Response('{"tag":"from-the-wire","alias":"second"}', {
      status: 200,
      headers: { "content-type": "application/json" },
    });
  },
});
const session = await client.auth.login({ body: { tag: "from-the-caller" } });
if (sent !== '{"tag":"from-the-caller"}') throw new Error(`the wire carries ${sent}`);
if (typeof session.tag !== "string" || session.tag !== "from-the-wire") {
  throw new Error(`a brand changed the value: ${String(session.tag)}`);
}
if (session.tag === session.alias) throw new Error("the response was not the declared one");
"#,
    );
}
