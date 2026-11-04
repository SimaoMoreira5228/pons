mod support;

use pons_emit_ts::TsOptions;
use pons_ir::Document;
use serde_json::json;
use support::runtime::{fixture, generated_file as file, zod_files as zod};

#[test]
fn an_inline_body_gets_one_validator_the_operation_imports() {
    let mut raw = fixture();
    raw["operations"][0]["responses"][0]["body"] = json!({"kind": "string", "format": "email"});
    let document: Document = serde_json::from_value(raw).unwrap();
    let files = zod(&document, TsOptions::default());
    let validators = file(&files, "validators.ts");
    assert!(
        validators.contains(
            "export const validateInline1: Validator = mismatch(z.string().regex(EMAIL, \"expected an email\"));"
        ),
        "{validators}"
    );
    let auth = file(&files, "operations/auth.ts");
    assert!(
        auth.contains(
            "import { validateInline1, validateLoginErrorInvalid } from \"../validators.js\";"
        ),
        "{auth}"
    );
    assert!(auth.contains("validate: validateInline1"), "{auth}");
}
