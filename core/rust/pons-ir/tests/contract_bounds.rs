use pons_ir::{Diagnostic, DiagnosticSeverity, Document, InlineType, TypeKind, TypeRef};
use std::collections::BTreeMap;

#[test]
fn field_error_diagnostics_block_validation() {
    let mut document: Document =
        serde_json::from_str(include_str!("../../../../spec/fixtures/login.json")).unwrap();
    let user = document.types.get_mut("User").unwrap();
    let TypeKind::Struct { fields, .. } = &mut user.definition else {
        unreachable!()
    };
    fields[0].diagnostics.push(Diagnostic {
        severity: DiagnosticSeverity::Error,
        code: "FIELD_ERROR".into(),
        message: "field is blocked".into(),
        origin: None,
        fidelity: None,
        capability: None,
        ext: BTreeMap::new(),
    });

    assert!(
        document
            .validate()
            .unwrap_err()
            .iter()
            .any(|error| error.to_string().contains("FIELD_ERROR"))
    );
}

#[test]
fn variant_error_diagnostics_block_validation() {
    let mut document: Document =
        serde_json::from_str(include_str!("../../../../spec/fixtures/login.json")).unwrap();
    let definition = document.types.get_mut("LoginError").unwrap();
    let TypeKind::Enum { variants, .. } = &mut definition.definition else {
        unreachable!()
    };
    variants[0].diagnostics.push(Diagnostic {
        severity: DiagnosticSeverity::Error,
        code: "VARIANT_ERROR".into(),
        message: "variant is blocked".into(),
        origin: None,
        fidelity: None,
        capability: None,
        ext: BTreeMap::new(),
    });

    let errors = document
        .validate()
        .unwrap_err()
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    assert!(
        errors.iter().any(|error| error.contains("VARIANT_ERROR")),
        "{errors:?}"
    );
    assert!(
        errors
            .iter()
            .any(|error| error.contains("type LoginError.variants.invalid diagnostics"))
    );

    let TypeKind::Enum { variants, .. } =
        &mut document.types.get_mut("LoginError").unwrap().definition
    else {
        unreachable!()
    };
    variants[0].diagnostics[0].severity = DiagnosticSeverity::Warning;
    document.validate().unwrap();
}

#[test]
fn validation_checks_64_bit_integer_bounds_exactly() {
    let base: Document =
        serde_json::from_str(include_str!("../../../../spec/fixtures/login.json")).unwrap();
    let mut document = base.clone();
    let definition = document.types.get_mut("User").unwrap();
    let TypeKind::Struct { fields, .. } = &mut definition.definition else {
        panic!()
    };
    let TypeRef::Inline(InlineType::Int {
        minimum, signed, ..
    }) = &mut fields[0].ty
    else {
        panic!()
    };
    *signed = true;
    *minimum = Some(serde_json::json!(9_223_372_036_854_775_808_u64));
    assert!(document.validate().unwrap_err().iter().any(|error| {
        error
            .to_string()
            .contains("minimum is above its native range")
    }));

    let mut document = base.clone();
    let definition = document.types.get_mut("User").unwrap();
    let TypeKind::Struct { fields, .. } = &mut definition.definition else {
        panic!()
    };
    let TypeRef::Inline(InlineType::Int {
        minimum,
        maximum,
        signed,
        ..
    }) = &mut fields[0].ty
    else {
        panic!()
    };
    *signed = true;
    *minimum = Some(serde_json::json!(9_223_372_036_854_775_808_u64));
    *maximum = Some(serde_json::json!(i64::MAX));
    assert!(
        document
            .validate()
            .unwrap_err()
            .iter()
            .any(|error| error.to_string().contains("minimum exceeds maximum"))
    );

    let mut document = base.clone();
    let definition = document.types.get_mut("User").unwrap();
    let TypeKind::Struct { fields, .. } = &mut definition.definition else {
        panic!()
    };
    let TypeRef::Inline(InlineType::Int {
        minimum, maximum, ..
    }) = &mut fields[0].ty
    else {
        panic!()
    };
    *minimum = Some(serde_json::json!(9_007_199_254_740_993_u64));
    *maximum = Some(serde_json::json!(9_007_199_254_740_992.0_f64));
    assert!(
        document
            .validate()
            .unwrap_err()
            .iter()
            .any(|error| error.to_string().contains("minimum exceeds maximum"))
    );

    let mut document = base;
    let definition = document.types.get_mut("User").unwrap();
    let TypeKind::Struct { fields, .. } = &mut definition.definition else {
        panic!()
    };
    let TypeRef::Inline(InlineType::Int { maximum, .. }) = &mut fields[0].ty else {
        panic!()
    };
    *maximum = Some(serde_json::json!(u64::MAX));
    assert!(document.validate().is_ok());
}

#[test]
fn validation_checks_integer_bounds_in_both_directions() {
    let base: Document =
        serde_json::from_str(include_str!("../../../../spec/fixtures/login.json")).unwrap();
    let set_bounds = |document: &mut Document,
                      bits: u16,
                      signed: bool,
                      minimum: Option<serde_json::Value>,
                      maximum: Option<serde_json::Value>,
                      multiple_of: Option<serde_json::Value>| {
        let definition = document.types.get_mut("User").unwrap();
        let TypeKind::Struct { fields, .. } = &mut definition.definition else {
            panic!()
        };
        let TypeRef::Inline(InlineType::Int {
            bits: field_bits,
            signed: field_signed,
            minimum: field_minimum,
            maximum: field_maximum,
            multiple_of: field_multiple_of,
            ..
        }) = &mut fields[0].ty
        else {
            panic!()
        };
        *field_bits = bits;
        *field_signed = signed;
        *field_minimum = minimum;
        *field_maximum = maximum;
        *field_multiple_of = multiple_of;
    };
    for (bits, signed, minimum, maximum, multiple_of, message) in [
        (
            32,
            true,
            Some(serde_json::json!(2_147_483_648_i64)),
            None,
            None,
            "minimum is above its native range",
        ),
        (
            32,
            true,
            None,
            Some(serde_json::json!(-2_147_483_649_i64)),
            None,
            "maximum is below its native range",
        ),
        (
            32,
            false,
            Some(serde_json::json!(4_294_967_296_u64)),
            None,
            None,
            "minimum is above its native range",
        ),
        (
            32,
            false,
            None,
            Some(serde_json::json!(-1)),
            None,
            "maximum is below its native range",
        ),
        (
            128,
            true,
            None,
            Some(serde_json::json!(
                170_141_183_460_469_231_731_687_303_715_884_105_728.0_f64
            )),
            None,
            "maximum is above its native range",
        ),
        (
            128,
            false,
            None,
            Some(serde_json::json!(
                340_282_366_920_938_463_463_374_607_431_768_211_456.0_f64
            )),
            None,
            "maximum is above its native range",
        ),
        (
            8,
            false,
            Some(serde_json::json!(1.5)),
            None,
            None,
            "minimum must be an integral value",
        ),
        (
            8,
            false,
            None,
            Some(serde_json::json!(255.5)),
            None,
            "maximum must be an integral value",
        ),
        (
            8,
            false,
            Some(serde_json::json!(0)),
            Some(serde_json::json!(255)),
            Some(serde_json::json!(0.5)),
            "multiple_of must be an integral value",
        ),
    ] {
        let mut document = base.clone();
        set_bounds(&mut document, bits, signed, minimum, maximum, multiple_of);
        let errors = document
            .validate()
            .unwrap_err()
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>();
        assert!(
            errors.iter().any(|error| error.contains(message)),
            "{message:?} missing from {errors:?}"
        );
    }

    for (minimum, maximum, multiple_of) in [
        (
            serde_json::json!(1.0),
            serde_json::json!(10.0),
            serde_json::json!(2.0),
        ),
        (
            serde_json::json!(0),
            serde_json::json!(255),
            serde_json::json!(1),
        ),
    ] {
        let mut document = base.clone();
        set_bounds(
            &mut document,
            8,
            false,
            Some(minimum),
            Some(maximum),
            Some(multiple_of),
        );
        assert!(document.validate().is_ok());
    }
}

#[test]
fn error_diagnostics_block_document_validation() {
    let mut document: Document =
        serde_json::from_str(include_str!("../../../../spec/fixtures/login.json")).unwrap();
    document.operations[0].diagnostics.push(Diagnostic {
        severity: DiagnosticSeverity::Error,
        code: "PONS-E999".into(),
        message: "unsupported boundary".into(),
        origin: None,
        fidelity: None,
        capability: None,
        ext: BTreeMap::new(),
    });
    assert!(
        document
            .validate()
            .unwrap_err()
            .iter()
            .any(|error| error.to_string().contains("PONS-E999"))
    );
}

#[test]
fn validation_rejects_inconsistent_payload_header_and_numeric_contracts() {
    let json = r#"{
        "ir_version": "0.3",
        "info": {"title": "Example", "version": "1"},
        "types": {
            "Bad": {
                "kind": "struct",
                "openness": "reject",
                "name": ["bad"],
                "direction": "both",
                "fields": [{
                    "wire": "value",
                    "ident": ["value"],
                    "ty": {
                        "kind": "int",
                        "bits": 32,
                        "signed": true,
                        "wire": "number",
                        "minimum": "zero",
                        "maximum": null,
                        "multiple_of": 0
                    },
                    "presence": "required"
                }, {
                    "wire": "bounded",
                    "ident": ["bounded"],
                    "ty": {
                        "kind": "int",
                        "bits": 8,
                        "signed": false,
                        "wire": "number",
                        "minimum": -1,
                        "maximum": 300
                    },
                    "presence": "required"
                }]
            },
            "Empty": {
                "kind": "enum",
                "name": ["empty"],
                "direction": "both",
                "repr": {"kind": "string"},
                "variants": []
            }
        },
        "operations": [{
            "id": [["test"]],
            "method": "POST",
            "path": [],
            "headers": [
                {"wire": "X-Trace", "ident": ["trace"], "ty": {"kind": "string"}, "presence": "optional"},
                {"wire": "x-trace", "ident": ["trace"], "ty": {"kind": "string"}, "presence": "optional"},
                {"wire": "bad header", "ident": ["bad"], "ty": {"kind": "string"}, "presence": "optional"}
            ],
            "body": {"ty": {"kind": "string"}, "content": "empty", "required": true},
            "responses": [
                {"status": 200, "content": "json"},
                {"status": 201, "content": "empty", "body": {"kind": "string"}},
                {"status": 202, "content": "stream"}
            ],
            "errors": [{
                "status": 400,
                "name": ["bad"],
                "content": "empty",
                "body": {"kind": "string"}
            }],
            "auth": {"alternatives": [], "optional": false},
            "semantics": {"safe": false, "idempotent": false}
        }]
    }"#;

    let document: Document = serde_json::from_str(json).unwrap();
    let errors = document.validate().unwrap_err();
    let messages = errors
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(messages.contains("integer minimum must be a number"));
    assert!(messages.contains("integer multiple_of must be greater than zero"));
    assert!(messages.contains("integer minimum is below its native range"));
    assert!(messages.contains("integer maximum is above its native range"));
    assert!(messages.contains("enum needs at least one variant"));
    assert!(messages.contains("request body content Empty"));
    assert!(messages.contains("duplicate request header x-trace"));
    assert!(messages.contains("not a valid HTTP field name"));
    assert!(messages.contains("response 200 content Json"));
    assert!(messages.contains("response 201 content Empty"));
    assert!(messages.contains("0 stream descriptions and 1 stream responses"));
    assert!(messages.contains("error content Empty"));
}

#[test]
fn validation_rejects_float_bounds_that_cannot_apply() {
    let base: Document =
        serde_json::from_str(include_str!("../../../../spec/fixtures/login.json")).unwrap();
    let with_float = |extra: serde_json::Value| {
        let mut document = base.clone();
        let definition = document.types.get_mut("User").unwrap();
        let TypeKind::Struct { fields, .. } = &mut definition.definition else {
            panic!()
        };
        let mut ty = serde_json::json!({"kind": "float", "bits": 64});
        ty.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        fields[0].ty = serde_json::from_value(ty).unwrap();
        document
    };
    for (extra, message) in [
        (
            serde_json::json!({"minimum": 0, "exclusive_minimum": 0}),
            "float minimum and exclusive_minimum cannot both apply",
        ),
        (
            serde_json::json!({"maximum": 0, "exclusive_maximum": 0}),
            "float maximum and exclusive_maximum cannot both apply",
        ),
        (
            serde_json::json!({"multiple_of": 0}),
            "float multiple_of must be greater than zero",
        ),
        (
            serde_json::json!({"minimum": "zero"}),
            "float minimum must be a number",
        ),
    ] {
        let errors = with_float(extra)
            .validate()
            .unwrap_err()
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>();
        assert!(
            errors.iter().any(|error| error.contains(message)),
            "{message:?} missing from {errors:?}"
        );
    }
}
