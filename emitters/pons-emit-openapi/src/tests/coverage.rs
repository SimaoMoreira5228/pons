use std::collections::BTreeMap;

use pons_ir::{Coverage, InlineType, ObjectOpenness, OpaqueWire, TypeKind, TypeRef};
use serde_json::{Value, json};

use super::fixture;

#[test]
fn emits_opaque_metadata_without_inventing_a_schema() {
    let mut document = fixture();
    let user = document.types.get_mut("User").unwrap();
    let TypeKind::Struct { fields, .. } = &mut user.definition else {
        unreachable!()
    };
    fields[0].ty = TypeRef::Inline(InlineType::Opaque {
        codec: "example.codec".into(),
        wire: OpaqueWire::Json,
        ext: Default::default(),
    });
    let emitted = crate::emit_with_mode(&document, crate::EmitMode::Strict).unwrap();
    assert!(emitted.coverage.opaque.contains("types.User"));
    assert!(emitted.coverage.opaque.contains("operations.auth.login"));
    let output: Value = serde_json::from_str(&emitted.content).unwrap();
    let field = &output["components"]["schemas"]["User"]["properties"]["id"];
    assert_eq!(field["x-pons-opaque"]["codec"], "example.codec");
    assert_eq!(field["x-pons-opaque"]["wire"], "json");
    assert!(field.get("type").is_none());
}

#[test]
fn lossy_mode_warns_for_non_exact_coverage() {
    let mut raw: Value = serde_json::to_value(fixture()).unwrap();
    raw["types"]["LoginError"]["variants"][1]["payload"]["fields"][0]["ty"]["wire"] =
        json!("number");
    raw["types"]["LoginError"]["variants"][1]["payload"]["fields"][0]["ty"]["bits"] = json!(64);
    let document: pons_ir::Document = serde_json::from_value(raw).unwrap();

    let strict = crate::emit_with_mode(&document, crate::EmitMode::Strict).unwrap();
    assert!(strict.warnings.is_empty());
    assert!(strict.coverage.exact.contains("types.LoginError"));

    let lossy = crate::emit_with_mode(&document, crate::EmitMode::Lossy).unwrap();
    assert!(lossy.coverage.lossy.contains("types.LoginError"));
    assert!(
        lossy
            .warnings
            .iter()
            .any(|warning| warning.contains("lossy coverage")
                && warning.contains("types.LoginError"))
    );
}

#[test]
fn opaque_feature_marks_preserve_lossy_classification() {
    let mut raw: Value = serde_json::to_value(fixture()).unwrap();
    raw["types"]["User"]["fields"][0]["ty"]["wire"] = json!("number");
    raw["types"]["User"]["fields"][0]["ty"]["bits"] = json!(64);
    let document: pons_ir::Document = serde_json::from_value(raw).unwrap();

    let emitted = crate::emit_with_mode(&document, crate::EmitMode::Lossy).unwrap();
    for subject in [
        "types.User",
        "operations.auth.login",
        "operations.events.watch",
    ] {
        assert!(emitted.coverage.lossy.contains(subject), "{subject}");
        assert!(!emitted.coverage.opaque.contains(subject), "{subject}");
    }
    assert!(emitted.warnings.iter().any(|warning| {
        warning.contains("lossy coverage")
            && warning.contains("types.User")
            && warning.contains("operations.auth.login")
            && warning.contains("operations.events.watch")
    }));
}

#[test]
fn lossy_mode_warns_for_opaque_coverage_while_strict_allows_it() {
    let mut document = fixture();
    let user = document.types.get_mut("User").unwrap();
    let TypeKind::Struct { fields, .. } = &mut user.definition else {
        unreachable!()
    };
    fields[0].ty = TypeRef::Inline(InlineType::Opaque {
        codec: "example.codec".into(),
        wire: OpaqueWire::Json,
        ext: Default::default(),
    });

    let strict = crate::emit_with_mode(&document, crate::EmitMode::Strict).unwrap();
    assert!(strict.warnings.is_empty());
    assert!(strict.coverage.opaque.contains("types.User"));

    let lossy = crate::emit_with_mode(&document, crate::EmitMode::Lossy).unwrap();
    assert!(
        lossy
            .warnings
            .iter()
            .any(|warning| warning.contains("opaque coverage") && warning.contains("types.User"))
    );
}

#[test]
fn strict_mode_never_claims_more_capability_than_canonical_coverage() {
    let mut rejecting = fixture();
    for id in ["User", "LoginRequest"] {
        let TypeKind::Struct { openness, .. } =
            &mut rejecting.types.get_mut(id).unwrap().definition
        else {
            unreachable!()
        };
        *openness = ObjectOpenness::Reject;
    }
    let cases: [(pons_ir::Document, &[&str], &[&str]); 2] = [
        (
            fixture(),
            &[
                "operations.auth.login",
                "operations.events.watch",
                "types.LoginRequest",
                "types.User",
            ],
            &["types.EventQuery", "types.LoginError"],
        ),
        (
            rejecting,
            &["operations.auth.login", "operations.events.watch"],
            &[
                "types.EventQuery",
                "types.LoginError",
                "types.LoginRequest",
                "types.User",
            ],
        ),
    ];

    for (document, opaque, exact) in cases {
        let canonical = pons_ir::canonical_coverage(&document, false).unwrap();
        let emitted = crate::emit_with_mode(&document, crate::EmitMode::Strict).unwrap();
        let reported: BTreeMap<String, pons_ir::CoverageLevel> =
            emitted.coverage.levels().collect();

        assert_eq!(
            reported.keys().collect::<Vec<_>>(),
            canonical.keys().collect::<Vec<_>>()
        );
        for (subject, level) in &canonical {
            assert!(
                reported[subject] >= *level,
                "the emitter reports {subject} as {} where canonical coverage is {}",
                reported[subject].as_str(),
                level.as_str()
            );
        }
        assert_eq!(emitted.warnings, Vec::<String>::new());
        assert_eq!(
            emitted.coverage.opaque,
            opaque.iter().map(|s| (*s).to_owned()).collect()
        );
        assert_eq!(
            emitted.coverage.exact,
            exact.iter().map(|s| (*s).to_owned()).collect()
        );
    }
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
    assert_eq!(
        canonical["operations.post.send"],
        pons_ir::CoverageLevel::Lossy
    );

    let refused = crate::emit_with_mode(&document, crate::EmitMode::Strict).unwrap_err();
    assert_eq!(
        refused.to_string(),
        "OpenAPI emitter reported lossy coverage in strict mode: \
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
    assert_eq!(emitted.warnings.len(), 1);
    assert_eq!(
        emitted.warnings[0],
        "OpenAPI emits lossy coverage at operations.post.send, types.Choice"
    );
    let output: Value = serde_json::from_str(&emitted.content).unwrap();
    assert!(output["components"]["schemas"]["Choice"]["anyOf"].is_array());
    assert!(output["components"]["schemas"]["Choice"]["oneOf"].is_null());
}

fn untagged_document() -> pons_ir::Document {
    let document: pons_ir::Document = serde_json::from_value(json!({
        "ir_version": "0.2",
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

#[test]
fn strict_mode_rejects_narrowed_coverage_but_carries_opaque_coverage() {
    for (coverage, expected) in [
        (
            Coverage {
                lossy: ["types.User".into()].into_iter().collect(),
                ..Coverage::default()
            },
            "lossy coverage",
        ),
        (
            Coverage {
                unsupported: ["types.User".into()].into_iter().collect(),
                ..Coverage::default()
            },
            "unsupported coverage",
        ),
    ] {
        let error = crate::coverage_warnings(crate::EmitMode::Strict, &coverage)
            .unwrap_err()
            .0;
        assert!(error.contains(expected), "{error}");
        assert!(error.contains("types.User"), "{error}");
        assert!(error.contains("in strict mode"), "{error}");
        assert!(
            !crate::coverage_warnings(crate::EmitMode::Lossy, &coverage)
                .unwrap()
                .is_empty()
        );
    }

    let carried = Coverage {
        opaque: ["types.User".into()].into_iter().collect(),
        ..Coverage::default()
    };
    assert!(crate::coverage_warnings(crate::EmitMode::Strict, &carried).is_ok());
}

#[test]
fn strict_mode_names_every_violating_coverage_class() {
    let coverage = Coverage {
        lossy: ["types.User".into()].into_iter().collect(),
        unsupported: ["operations.auth.login".into()].into_iter().collect(),
        ..Coverage::default()
    };
    let error = crate::coverage_warnings(crate::EmitMode::Strict, &coverage)
        .unwrap_err()
        .0;
    assert_eq!(
        error,
        "OpenAPI emitter reported unsupported coverage in strict mode: operations.auth.login; \
         lossy coverage in strict mode: types.User",
        "every violating class is named, worst class first"
    );
    assert!(crate::coverage_warnings(crate::EmitMode::Lossy, &coverage).is_ok());
}

#[test]
fn marks_extension_only_operations_as_opaque() {
    let document = fixture();
    let emitted = crate::emit_with_mode(&document, crate::EmitMode::Strict).unwrap();
    assert!(emitted.coverage.opaque.contains("operations.auth.login"));
    assert!(emitted.coverage.opaque.contains("operations.events.watch"));
    assert!(emitted.coverage.opaque.contains("types.User"));
    assert!(emitted.coverage.opaque.contains("types.LoginRequest"));
    assert!(!emitted.coverage.exact.contains("operations.auth.login"));
    assert!(!emitted.coverage.exact.contains("operations.events.watch"));
    assert!(!emitted.coverage.lossy.contains("operations.auth.login"));
    assert!(!emitted.coverage.lossy.contains("operations.events.watch"));
}

#[test]
fn marks_preservation_through_type_references() {
    let mut raw = serde_json::to_value(fixture()).unwrap();
    raw["types"]["Wrapper"] = json!({
        "kind": "struct",
        "name": ["wrapper"],
        "direction": "both",
        "openness": "reject",
        "fields": [{
            "wire": "user",
            "ident": ["user"],
            "ty": { "ref": "User" },
            "presence": "required"
        }]
    });
    raw["operations"][1]["stream"] = Value::Null;
    raw["operations"][1]["responses"] = json!([{
        "status": 200,
        "content": "json",
        "body": { "ref": "Wrapper" }
    }]);
    let document: pons_ir::Document = serde_json::from_value(raw).unwrap();
    let emitted = crate::emit_with_mode(&document, crate::EmitMode::Strict).unwrap();
    assert!(emitted.coverage.opaque.contains("types.Wrapper"));
    assert!(emitted.coverage.opaque.contains("operations.events.watch"));
}
