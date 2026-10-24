use pons_ir::Document;
use serde_json::{Value, json};
use std::collections::BTreeSet;

fn document(variants: Value) -> Document {
    let document: Document = serde_json::from_value(json!({
        "ir_version": "0.2",
        "info": {"title": "Example", "version": "1"},
        "types": {
            "Choice": {
                "kind": "enum", "name": ["choice"], "direction": "both",
                "repr": {"kind": "untagged"},
                "variants": variants
            }
        },
        "operations": [{
            "id": [["post"], ["send"]],
            "method": "POST",
            "path": [{"lit": "send"}],
            "body": {"ty": {"ref": "Choice"}, "content": "json", "required": true},
            "responses": [{"status": 200, "content": "json", "body": {"kind": "string"}}],
            "auth": {"alternatives": [], "optional": false},
            "semantics": {"safe": false, "idempotent": false}
        }]
    }))
    .unwrap();
    document.validate().unwrap();
    document
}

fn competing() -> Value {
    json!([
        {"wire": "text", "ident": ["text"],
         "payload": {"kind": "newtype", "ty": {"kind": "string"}}},
        {"wire": "count", "ident": ["count"],
         "payload": {"kind": "newtype",
                     "ty": {"kind": "int", "bits": 8, "signed": false, "wire": "number"}}}
    ])
}

#[test]
fn both_targets_refuse_and_admit_the_same_untagged_subjects() {
    let document = document(competing());
    let lossy = ["operations.post.send", "types.Choice"]
        .into_iter()
        .map(str::to_owned)
        .collect::<BTreeSet<_>>();

    let openapi_strict =
        pons_emit_openapi::emit_with_mode(&document, pons_emit_openapi::EmitMode::Strict)
            .expect_err("the OpenAPI emitter must refuse an ambiguous untagged enum");
    let typescript_strict = pons_emit_ts::emit_with_mode(&document, pons_emit_ts::EmitMode::Strict)
        .expect_err("the TypeScript emitter must refuse an ambiguous untagged enum");
    for (target, subjects, refusal) in [
        ("OpenAPI", &lossy, openapi_strict.to_string()),
        ("the TypeScript", &lossy, typescript_strict.to_string()),
    ] {
        assert_eq!(
            refusal,
            format!(
                "{target} emitter reported lossy coverage in strict mode: {}",
                subjects.iter().cloned().collect::<Vec<_>>().join(", ")
            )
        );
    }

    let openapi = pons_emit_openapi::emit_with_mode(&document, pons_emit_openapi::EmitMode::Lossy)
        .expect("the OpenAPI emitter must admit it in lossy mode");
    let typescript = pons_emit_ts::emit_with_mode(&document, pons_emit_ts::EmitMode::Lossy)
        .expect("the TypeScript emitter must admit it in lossy mode");
    assert_eq!(openapi.coverage.lossy, lossy);
    assert_eq!(typescript.coverage.lossy, lossy);
    assert!(openapi.coverage.exact.is_empty());
    assert!(typescript.coverage.exact.is_empty());

    let schema: Value = serde_json::from_str(&openapi.content).unwrap();
    let any_of = schema["components"]["schemas"]["Choice"]["anyOf"]
        .as_array()
        .unwrap();
    assert_eq!(any_of.len(), 2);
    assert!(
        schema["components"]["schemas"]["Choice"]["oneOf"].is_null(),
        "oneOf would claim a value belongs to exactly one variant, which is what an untagged enum cannot say"
    );
    let types = typescript
        .files
        .iter()
        .find(|file| file.path == "types.ts")
        .unwrap();
    assert!(
        types
            .content
            .contains("export type Choice = number | string"),
        "{}",
        types.content
    );
}

#[test]
fn both_targets_keep_one_winner_untagged_at_exact() {
    for variants in [
        json!([
            {"wire": "nothing", "ident": ["nothing"], "payload": {"kind": "unit"}},
            {"wire": "text", "ident": ["text"],
             "payload": {"kind": "newtype", "ty": {"kind": "string"}}}
        ]),
        json!([
            {"wire": "nothing", "ident": ["nothing"], "payload": {"kind": "unit"}},
            {"wire": "empty", "ident": ["empty"], "payload": {"kind": "unit"}}
        ]),
    ] {
        let document = document(variants);
        let openapi =
            pons_emit_openapi::emit_with_mode(&document, pons_emit_openapi::EmitMode::Strict)
                .expect("OpenAPI must stay strict");
        let typescript = pons_emit_ts::emit_with_mode(&document, pons_emit_ts::EmitMode::Strict)
            .expect("TypeScript must stay strict");
        assert_eq!(openapi.coverage.exact, typescript.coverage.exact);
        assert!(openapi.coverage.lossy.is_empty());
        assert!(typescript.coverage.lossy.is_empty());
    }
}
