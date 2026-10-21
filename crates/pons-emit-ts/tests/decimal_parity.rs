use std::collections::BTreeSet;

use pons_emit_ts::{EmitMode, EmitOutput, emit_with_mode};
use pons_ir::Document;
use serde_json::{Value, json};

fn priced_document(wire: &str) -> Document {
    let mut raw: Value =
        serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
    raw["types"]["LoginRequest"]["openness"] = json!("reject");
    raw["types"]["Price"] = json!({
        "kind": "struct",
        "name": ["price"],
        "direction": "both",
        "openness": "reject",
        "fields": [{
            "wire": "amount",
            "ident": ["amount"],
            "ty": {"kind": "decimal", "wire": wire},
            "presence": "required"
        }]
    });
    raw["types"]["LoginRequest"]["fields"]
        .as_array_mut()
        .unwrap()
        .push(json!({
            "wire": "price",
            "ident": ["price"],
            "ty": {"ref": "Price"},
            "presence": "required"
        }));
    serde_json::from_value(raw).unwrap()
}

fn types_of(output: &EmitOutput) -> &str {
    &output
        .files
        .iter()
        .find(|file| file.path == "types.ts")
        .unwrap()
        .content
}

#[test]
fn a_decimal_that_crosses_the_wire_as_a_string_is_exact() {
    let output = emit_with_mode(&priced_document("string"), EmitMode::Strict).unwrap();
    assert!(types_of(&output).contains("\"amount\": string"));
    assert!(output.coverage.lossy.is_empty());
    assert!(output.warnings.is_empty());
}

#[test]
fn a_decimal_on_the_number_wire_is_a_number_to_both_targets() {
    let document = priced_document("number");
    let error = emit_with_mode(&document, EmitMode::Strict).unwrap_err();
    assert!(error.0.contains("types.Price.fields[0].ty"), "{}", error.0);

    let output = emit_with_mode(&document, EmitMode::Lossy).unwrap();
    assert!(types_of(&output).contains("\"amount\": number"));
    assert_eq!(
        output.coverage.lossy,
        BTreeSet::from([
            "operations.auth.login".to_owned(),
            "types.LoginRequest".to_owned(),
            "types.Price".to_owned(),
        ])
    );
    assert_eq!(
        output.warnings,
        vec![
            "TypeScript emits decimals that cross the wire as numbers at types.Price.fields[0].ty"
                .to_owned()
        ]
    );

    let openapi: Value = serde_json::from_str(
        &pons_emit_openapi::emit(&document).expect("this emitter describes the wire as it is"),
    )
    .unwrap();
    let amount = &openapi["components"]["schemas"]["Price"]["properties"]["amount"];
    assert_eq!(
        amount["type"],
        json!("number"),
        "both targets claim one wire"
    );
    assert_eq!(amount["x-pons-decimal-wire"], json!("number"));
}
