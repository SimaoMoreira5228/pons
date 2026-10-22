use pons_emit_ts::{EmitMode, emit_with_mode};
use pons_ir::Document;
use serde_json::{Value, json};

fn fixture() -> Value {
    serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap()
}

#[test]
fn a_document_extension_the_typescript_target_cannot_read_produces_one_warning() {
    let document: Document = serde_json::from_value(fixture()).unwrap();
    let output = emit_with_mode(&document, EmitMode::Strict).unwrap();
    assert_eq!(
        output.warnings,
        vec![
            "TypeScript does not understand the extension namespace test.example.source/v1, so the \
             emitted client ignores it"
                .to_owned()
        ],
        "the namespace appears on the document, a type, and a field, and one warning names it once"
    );
}

#[test]
fn a_document_without_extensions_warns_about_none() {
    let mut raw = fixture();
    raw["ext"] = json!({});
    raw["types"]["User"]["ext"] = json!({});
    raw["types"]["User"]["fields"][0]["ext"] = json!({});
    let document: Document = serde_json::from_value(raw).unwrap();
    let output = emit_with_mode(&document, EmitMode::Strict).unwrap();
    assert!(output.warnings.is_empty(), "{:?}", output.warnings);
}
