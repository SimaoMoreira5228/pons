mod support;

use pons_emit_ts::{EmitMode, ErrorStrategy, emit_with_strategy};
use pons_ir::Document;

use support::type_check::{files, fixture, operations_of, type_checks, types_of};

#[test]
fn an_enum_struct_payload_is_open_like_the_runtime_treats_it() {
    let mut raw = fixture();
    raw["types"]["LoginError"]["variants"][1]["payload"]["fields"] = serde_json::json!([
        {"wire": "retry_after", "ident": ["retry", "after"], "ty": {"kind": "int", "bits": 32, "signed": false, "wire": "number"}, "presence": "required"},
        {"wire": "note", "ident": ["note"], "ty": {"kind": "string"}, "presence": "optional"}
    ]);
    let document: Document = serde_json::from_value(raw).unwrap();
    let files = files(&document);
    let types = types_of(&files);
    assert!(
        types.contains("\"note\"?: string; \"retry_after\": number; [key: string]: unknown }"),
        "{types}"
    );
    type_checks(
        &files,
        r#"
import type { LoginError } from "./types.js";

const locked: LoginError = { code: "locked", retry_after: 1, undeclared: "preserved" };
// @ts-expect-error a declared required payload field is still required
const missing: LoginError = { code: "locked", undeclared: "preserved" };
console.log(locked, missing);
"#,
    );
}

#[test]
fn a_narrowed_untagged_variant_carries_only_its_own_payload() {
    let mut raw = fixture();
    raw["types"]["Choice"] = serde_json::json!({
        "kind": "enum", "name": ["choice"], "direction": "output", "repr": {"kind": "untagged"},
        "variants": [
            {"wire": "label", "ident": ["label"], "payload": {"kind": "newtype", "ty": {"kind": "string"}}},
            {"wire": "count", "ident": ["count"], "payload": {"kind": "newtype", "ty": {"kind": "int", "bits": 16, "signed": false, "wire": "number"}}}
        ]
    });
    raw["operations"][0]["errors"][0]["body"] =
        serde_json::json!({"ref": "Choice", "variant": "label"});
    let document: Document = serde_json::from_value(raw).unwrap();
    let files = emit_with_strategy(&document, EmitMode::Lossy, ErrorStrategy::Throw)
        .expect("the TypeScript emitter must accept the document")
        .files;
    assert!(
        operations_of(&files, "auth").contains("ApiError<number, string, Headers>"),
        "{}",
        operations_of(&files, "auth")
    );
    type_checks(
        &files,
        r#"
import { createClient, type AuthLoginError } from "./client.js";

const client = createClient({ baseUrl: "https://api.example.test" });
const failure: unknown = await client
  .auth.login({ body: { email: "person@example.test" } })
  .catch((cause: unknown) => cause);
if (typeof failure === "object" && failure !== null && "error" in failure) {
  const error = (failure as { error: AuthLoginError }).error;
  if (error._tag === "ApiError") {
    const label: string = error.body;
    // @ts-expect-error the runtime only ever tries the variant the reference names
    const other: number = error.body;
    console.log(label, other);
  }
}
"#,
    );
}
