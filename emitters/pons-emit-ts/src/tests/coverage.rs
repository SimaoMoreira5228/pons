use std::collections::BTreeMap;

use pons_ir::{Document, InlineType, IntegerWire, TypeKind, TypeRef};

use super::file;

#[test]
fn strict_mode_uses_bigint_and_lossy_mode_warns() {
    let mut document: Document =
        serde_json::from_str(include_str!("../../../../spec/fixtures/login.json")).unwrap();
    let definition = document.types.get_mut("User").unwrap();
    let TypeKind::Struct { fields, .. } = &mut definition.definition else {
        panic!()
    };
    let TypeRef::Inline(InlineType::Int { wire, .. }) = &mut fields[0].ty else {
        panic!()
    };
    *wire = IntegerWire::Number;

    let files = crate::emit(&document).unwrap();
    let types = files.iter().find(|file| file.path == "types.ts").unwrap();
    assert!(types.content.contains("\"id\": bigint"));
    let transport = file(&files, "runtime/transport/request.ts");
    assert!(transport.contains("parseJson(text, preserveWideIntegers)"));
    assert!(transport.contains("stringifyJson(encodedBody)"));

    let output = crate::emit_with_mode(&document, crate::EmitMode::Lossy).unwrap();
    assert!(
        output
            .warnings
            .iter()
            .any(|warning| warning.contains("wide numeric integers"))
    );
    let types = output
        .files
        .iter()
        .find(|file| file.path == "types.ts")
        .unwrap();
    assert!(types.content.contains("\"id\": number"));
}

#[test]
fn coverage_agrees_with_the_canonical_coverage_of_the_fixture() {
    let document: Document =
        serde_json::from_str(include_str!("../../../../spec/fixtures/login.json")).unwrap();
    let canonical = pons_ir::canonical_coverage(&document, false).unwrap();
    let output = crate::emit_with_mode(&document, crate::EmitMode::Strict).unwrap();

    assert_eq!(
        output.coverage.levels().collect::<BTreeMap<_, _>>(),
        canonical
    );
    assert_eq!(
        output.warnings,
        vec![
            "TypeScript does not understand the extension namespace test.example.source/v1, \
                 so the emitted client ignores it"
                .to_owned()
        ],
        "the fixture carries one extension namespace the client cannot read"
    );
}

#[test]
fn reports_wide_integer_coverage_by_mode() {
    let mut document: Document =
        serde_json::from_str(include_str!("../../../../spec/fixtures/login.json")).unwrap();
    let definition = document.types.get_mut("User").unwrap();
    let TypeKind::Struct { fields, .. } = &mut definition.definition else {
        panic!()
    };
    let TypeRef::Inline(InlineType::Int { wire, .. }) = &mut fields[0].ty else {
        panic!()
    };
    *wire = IntegerWire::Number;

    let strict = crate::emit_with_mode(&document, crate::EmitMode::Strict).unwrap();
    assert!(!strict.coverage.exact.contains("types.User"));
    assert!(strict.coverage.opaque.contains("types.User"));
    assert!(strict.coverage.lossy.is_empty());
    let lossy = crate::emit_with_mode(&document, crate::EmitMode::Lossy).unwrap();
    assert!(lossy.coverage.lossy.contains("types.User"));
    assert!(lossy.coverage.lossy.contains("operations.auth.login"));
}

#[test]
fn an_untagged_enum_is_lossy_and_strict_mode_refuses_it() {
    let document = untagged_document();
    let canonical = pons_ir::canonical_coverage(&document, false).unwrap();
    assert_eq!(
        canonical["types.Choice"],
        pons_ir::CoverageLevel::Lossy,
        "{canonical:?}"
    );

    let refused = crate::emit_with_mode(&document, crate::EmitMode::Strict).unwrap_err();
    assert_eq!(
        refused.to_string(),
        "the TypeScript emitter reported lossy coverage in strict mode: \
             operations.post.send, types.Choice"
    );
    let emitted = crate::emit_with_mode(&document, crate::EmitMode::Lossy).unwrap();
    assert_eq!(
        emitted.coverage.lossy,
        ["operations.post.send", "types.Choice"]
            .into_iter()
            .map(str::to_owned)
            .collect()
    );
    assert!(emitted.coverage.exact.is_empty());
}

fn untagged_document() -> Document {
    let document: Document = serde_json::from_value(serde_json::json!({
        "ir_version": "0.3",
        "info": {"title": "Example", "version": "1"},
        "types": {
            "Choice": {
                "kind": "enum", "name": ["choice"], "direction": "both",
                "repr": {"kind": "untagged"},
                "variants": [
                    {"wire": "text", "ident": ["text"],
                     "payload": {"kind": "newtype", "ty": {"kind": "string"}}},
                    {"wire": "count", "ident": ["count"],
                     "payload": {"kind": "newtype", "ty": {"kind": "int", "bits": 8,
                         "signed": false, "wire": "number"}}}
                ]
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
