use pons_ir::{Direction, EnumRepr, Presence, TypeKind, VariantPayload};

use super::{definition, has_code, lower_root, opaque_codec};

#[test]
fn a_discriminator_with_a_mapping_uses_mapping_keys_as_wire_names() {
    let schema = serde_json::json!({
        "oneOf": [
            { "$ref": "#/$defs/Card" },
            { "$ref": "#/$defs/Cash" }
        ],
        "discriminator": {
            "propertyName": "kind",
            "mapping": {
                "card": "#/$defs/Card",
                "cash-payment": "#/$defs/Cash"
            }
        },
        "$defs": {
            "Card": {
                "type": "object",
                "properties": { "number": { "type": "string" } },
                "required": ["number"]
            },
            "Cash": {
                "type": "object",
                "properties": { "amount": { "type": "integer" } },
                "required": ["amount"]
            }
        }
    });
    let (cx, reference) = lower_root(schema, "acme::Payment", "Payment", Direction::Both);

    let TypeKind::Enum { repr, variants } = &definition(&cx, &reference).definition else {
        panic!()
    };
    assert_eq!(repr, &EnumRepr::Internal { tag: "kind".into() });
    assert_eq!(
        variants
            .iter()
            .map(|variant| variant.wire.as_str())
            .collect::<Vec<_>>(),
        ["card", "cash-payment"]
    );
    let cash = variants
        .iter()
        .find(|variant| variant.wire == "cash-payment")
        .unwrap();
    assert_eq!(cash.ident, ["cash", "payment"]);
    let VariantPayload::Struct { fields } = &cash.payload else {
        panic!()
    };
    assert_eq!(fields.len(), 1);
    assert_eq!(fields[0].wire, "amount");
    assert_eq!(fields[0].presence, Presence::Required);
}

#[test]
fn a_discriminator_without_a_mapping_uses_branch_ref_names_as_wire_names() {
    let schema = serde_json::json!({
        "anyOf": [
            { "$ref": "#/$defs/Card" },
            { "$ref": "#/$defs/Cash" }
        ],
        "discriminator": { "propertyName": "kind" },
        "$defs": {
            "Card": {
                "type": "object",
                "properties": { "number": { "type": "string" } }
            },
            "Cash": {
                "type": "object",
                "properties": { "amount": { "type": "integer" } }
            }
        }
    });
    let (cx, reference) = lower_root(schema, "acme::Payment", "Payment", Direction::Both);

    let TypeKind::Enum { repr, variants } = &definition(&cx, &reference).definition else {
        panic!()
    };
    assert_eq!(repr, &EnumRepr::Internal { tag: "kind".into() });
    assert_eq!(
        variants
            .iter()
            .map(|variant| variant.wire.as_str())
            .collect::<Vec<_>>(),
        ["Card", "Cash"]
    );
    assert_eq!(variants[0].ident, ["card"]);
}

#[test]
fn a_discriminator_without_a_mapping_accepts_titled_inline_branches() {
    let schema = serde_json::json!({
        "oneOf": [
            {
                "title": "card",
                "type": "object",
                "properties": { "number": { "type": "string" } },
                "required": ["number"]
            },
            {
                "title": "cash",
                "type": "object",
                "properties": { "amount": { "type": "integer" } }
            }
        ],
        "discriminator": { "propertyName": "kind" }
    });
    let (cx, reference) = lower_root(schema, "acme::Payment", "Payment", Direction::Both);

    let TypeKind::Enum { variants, .. } = &definition(&cx, &reference).definition else {
        panic!()
    };
    assert_eq!(
        variants
            .iter()
            .map(|variant| variant.wire.as_str())
            .collect::<Vec<_>>(),
        ["card", "cash"]
    );
    let VariantPayload::Struct { fields } = &variants[0].payload else {
        panic!()
    };
    assert_eq!(fields[0].wire, "number");
}

#[test]
fn a_discriminator_removes_the_tag_from_each_payload() {
    let schema = serde_json::json!({
        "oneOf": [
            { "$ref": "#/$defs/Card" },
            { "$ref": "#/$defs/Cash" }
        ],
        "discriminator": { "propertyName": "kind" },
        "$defs": {
            "Card": {
                "type": "object",
                "properties": {
                    "kind": { "type": "string" },
                    "number": { "type": "string" }
                },
                "required": ["kind", "number"]
            },
            "Cash": {
                "type": "object",
                "properties": {
                    "kind": { "type": "string" },
                    "amount": { "type": "integer" }
                },
                "required": ["kind", "amount"]
            }
        }
    });
    let (cx, reference) = lower_root(schema, "acme::Payment", "Payment", Direction::Both);

    let TypeKind::Enum { variants, .. } = &definition(&cx, &reference).definition else {
        panic!()
    };
    for variant in variants {
        let VariantPayload::Struct { fields } = &variant.payload else {
            panic!()
        };
        assert!(!fields.iter().any(|field| field.wire == "kind"));
    }
    assert!(!has_code(&cx, &reference, "PONS-W202"));
}

#[test]
fn a_discriminator_tag_with_another_required_type_becomes_opaque() {
    let schema = serde_json::json!({
        "oneOf": [{ "$ref": "#/$defs/Card" }],
        "discriminator": { "propertyName": "kind" },
        "$defs": {
            "Card": {
                "type": "object",
                "properties": {
                    "kind": { "type": "integer" },
                    "number": { "type": "string" }
                },
                "required": ["kind", "number"]
            }
        }
    });
    let (cx, reference) = lower_root(schema, "acme::Payment", "Payment", Direction::Both);

    assert_eq!(opaque_codec(&cx, &reference), "json-schema:discriminator");
    assert!(has_code(&cx, &reference, "PONS-W202"));
}

#[test]
fn a_discriminator_branch_that_is_not_an_object_becomes_opaque() {
    let schema = serde_json::json!({
        "oneOf": [{ "$ref": "#/$defs/Label" }],
        "discriminator": {
            "propertyName": "kind",
            "mapping": { "label": "#/$defs/Label" }
        },
        "$defs": {
            "Label": { "type": "string" }
        }
    });
    let (cx, reference) = lower_root(schema, "acme::Payment", "Payment", Direction::Both);

    assert_eq!(opaque_codec(&cx, &reference), "json-schema:discriminator");
    assert!(has_code(&cx, &reference, "PONS-W202"));
}
