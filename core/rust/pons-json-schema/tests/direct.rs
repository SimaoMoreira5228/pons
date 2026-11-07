use pons_core::TypeCx;
use pons_ir::{
    Direction, EnumRepr, Field, InlineType, IntegerWire, ObjectOpenness, Presence, TypeDef,
    TypeKind, TypeRef, VariantPayload,
};
use pons_json_schema::{RootType, lower};

fn definition<'cx>(cx: &'cx TypeCx, reference: &TypeRef) -> &'cx TypeDef {
    let TypeRef::Named(named) = reference else {
        panic!()
    };
    &cx.definitions()[&named.id]
}

fn lower_root(
    schema: serde_json::Value,
    id: &str,
    name: &str,
    direction: Direction,
) -> (TypeCx, TypeRef) {
    let mut cx = TypeCx::new();
    let reference = lower(
        &mut cx,
        schema,
        RootType {
            id,
            name,
            origin: Some(id),
            direction,
        },
    );
    (cx, reference)
}

fn opaque_codec<'cx>(cx: &'cx TypeCx, reference: &TypeRef) -> &'cx str {
    let TypeKind::Newtype { inner } = &definition(cx, reference).definition else {
        panic!()
    };
    let TypeRef::Inline(InlineType::Opaque { codec, .. }) = inner else {
        panic!()
    };
    codec
}

fn has_code(cx: &TypeCx, reference: &TypeRef, code: &str) -> bool {
    definition(cx, reference)
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == code)
}

fn field<'a>(fields: &'a [Field], wire: &str) -> &'a Field {
    fields
        .iter()
        .find(|field| field.wire == wire)
        .unwrap_or_else(|| panic!("no field named {wire}"))
}

#[test]
fn a_definition_ref_and_a_bare_integer_keep_their_ids_and_diagnostic() {
    let mut cx = TypeCx::new();
    let schema = serde_json::json!({
        "type": "object",
        "properties": {
            "address": { "$ref": "#/$defs/Address" },
            "count": { "type": "integer" }
        },
        "required": ["address", "count"],
        "additionalProperties": false,
        "$defs": {
            "Address": {
                "type": "object",
                "properties": { "street": { "type": "string" } },
                "required": ["street"],
                "additionalProperties": false
            }
        }
    });
    let reference = lower(
        &mut cx,
        schema,
        RootType {
            id: "acme::Order",
            name: "Order",
            origin: Some("acme::Order"),
            direction: Direction::Input,
        },
    );
    let TypeRef::Named(named) = &reference else {
        panic!()
    };
    assert_eq!(named.id, "acme::Order#input");

    let root = definition(&cx, &reference);
    assert_eq!(root.origin.as_deref(), Some("acme::Order"));
    let TypeKind::Struct {
        fields, openness, ..
    } = &root.definition
    else {
        panic!()
    };
    assert_eq!(openness, &ObjectOpenness::Reject);

    let address = field(fields, "address");
    let TypeRef::Named(address_ref) = &address.ty else {
        panic!()
    };
    assert_eq!(address_ref.id, "acme::Order::Address#input");
    assert_eq!(
        definition(&cx, &address.ty).origin.as_deref(),
        Some("acme::Order::Address")
    );

    let TypeRef::Inline(InlineType::Int {
        bits, signed, wire, ..
    }) = &field(fields, "count").ty
    else {
        panic!()
    };
    assert_eq!((*bits, *signed, *wire), (64, false, IntegerWire::Number));

    assert!(
        root.diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "PONS-W201")
    );
}

#[test]
fn a_one_of_with_several_branches_becomes_opaque() {
    let mut cx = TypeCx::new();
    let schema = serde_json::json!({
        "oneOf": [{ "type": "string" }, { "type": "integer" }]
    });
    let reference = lower(
        &mut cx,
        schema,
        RootType {
            id: "acme::Choice",
            name: "Choice",
            origin: None,
            direction: Direction::Both,
        },
    );

    let root = definition(&cx, &reference);
    assert_eq!(root.origin, None);
    let TypeKind::Newtype { inner } = &root.definition else {
        panic!()
    };
    let TypeRef::Inline(InlineType::Opaque { codec, .. }) = inner else {
        panic!()
    };
    assert_eq!(codec, "json-schema:oneOf");
    assert!(
        root.diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "PONS-W202")
    );
}

#[test]
fn legacy_definitions_are_named_with_the_root_prefix_and_direction() {
    let mut cx = TypeCx::new();
    let schema = serde_json::json!({
        "type": "object",
        "properties": { "label": { "$ref": "#/definitions/Label" } },
        "required": ["label"],
        "definitions": {
            "Label": { "type": "string", "enum": ["alpha", "beta"] }
        }
    });
    let reference = lower(
        &mut cx,
        schema,
        RootType {
            id: "acme::Tag",
            name: "Tag",
            origin: Some("acme::Tag"),
            direction: Direction::Output,
        },
    );

    let TypeRef::Named(named) = &reference else {
        panic!()
    };
    assert_eq!(named.id, "acme::Tag#output");

    let TypeKind::Struct { fields, .. } = &definition(&cx, &reference).definition else {
        panic!()
    };
    let label = field(fields, "label");
    assert_eq!(label.presence, Presence::Required);
    let TypeRef::Named(label_ref) = &label.ty else {
        panic!()
    };
    assert_eq!(label_ref.id, "acme::Tag::Label#output");

    let TypeKind::Enum { repr, variants } = &definition(&cx, &label.ty).definition else {
        panic!()
    };
    assert_eq!(repr, &EnumRepr::String);
    assert_eq!(variants.len(), 2);
}

#[test]
fn all_of_merges_properties_in_branch_order_and_unions_required() {
    let schema = serde_json::json!({
        "allOf": [
            {
                "type": "object",
                "properties": { "id": { "type": "string" } },
                "required": ["id"],
                "additionalProperties": false
            },
            {
                "type": "object",
                "properties": { "label": { "type": "string" } },
                "required": ["label"]
            }
        ]
    });
    let (cx, reference) = lower_root(schema, "acme::Node", "Node", Direction::Both);

    let TypeKind::Struct {
        fields, openness, ..
    } = &definition(&cx, &reference).definition
    else {
        panic!()
    };
    assert_eq!(openness, &ObjectOpenness::Reject);
    assert_eq!(
        fields
            .iter()
            .map(|field| field.wire.as_str())
            .collect::<Vec<_>>(),
        ["id", "label"]
    );
    assert_eq!(field(fields, "id").presence, Presence::Required);
    assert_eq!(field(fields, "label").presence, Presence::Required);
    assert!(!has_code(&cx, &reference, "PONS-W202"));
}

#[test]
fn all_of_with_a_conflicting_property_becomes_opaque() {
    let schema = serde_json::json!({
        "allOf": [
            {
                "type": "object",
                "properties": { "id": { "type": "string" } }
            },
            {
                "type": "object",
                "properties": { "id": { "type": "integer" } }
            }
        ]
    });
    let (cx, reference) = lower_root(schema, "acme::Conflict", "Conflict", Direction::Both);

    assert_eq!(opaque_codec(&cx, &reference), "json-schema:allOf");
    assert!(has_code(&cx, &reference, "PONS-W202"));
}

#[test]
fn all_of_intersects_additional_properties() {
    let schema = serde_json::json!({
        "allOf": [
            {
                "type": "object",
                "properties": { "id": { "type": "string" } },
                "additionalProperties": { "type": "string" }
            },
            {
                "type": "object",
                "properties": { "label": { "type": "string" } },
                "additionalProperties": { "type": "string" }
            }
        ]
    });
    let (cx, reference) = lower_root(schema, "acme::Node", "Node", Direction::Both);

    let TypeKind::Struct { openness, .. } = &definition(&cx, &reference).definition else {
        panic!()
    };
    let ObjectOpenness::PreserveTyped { value } = openness else {
        panic!()
    };
    assert!(matches!(value, TypeRef::Inline(InlineType::String { .. })));
}

#[test]
fn all_of_with_disagreeing_additional_properties_reports_a_diagnostic() {
    let schema = serde_json::json!({
        "allOf": [
            {
                "type": "object",
                "properties": { "id": { "type": "string" } },
                "additionalProperties": { "type": "string" }
            },
            {
                "type": "object",
                "properties": { "label": { "type": "string" } },
                "additionalProperties": { "type": "integer" }
            }
        ]
    });
    let (cx, reference) = lower_root(schema, "acme::Node", "Node", Direction::Both);

    let TypeKind::Struct {
        fields, openness, ..
    } = &definition(&cx, &reference).definition
    else {
        panic!()
    };
    assert_eq!(openness, &ObjectOpenness::PreserveOpaque);
    assert_eq!(fields.len(), 2);
    assert!(has_code(&cx, &reference, "PONS-W202"));
}

fn inline_struct(reference: &TypeRef) -> (&[Field], &ObjectOpenness) {
    let TypeRef::Inline(InlineType::Struct {
        fields, openness, ..
    }) = reference
    else {
        panic!()
    };
    (fields, openness)
}

fn inline_opaque_codec(reference: &TypeRef) -> &str {
    let TypeRef::Inline(InlineType::Opaque { codec, .. }) = reference else {
        panic!()
    };
    codec
}

#[test]
fn an_inline_all_of_merges_a_ref_and_an_inline_branch() {
    let schema = serde_json::json!({
        "type": "object",
        "properties": {
            "node": {
                "allOf": [
                    { "$ref": "#/$defs/Base" },
                    {
                        "type": "object",
                        "properties": { "label": { "type": "string" } },
                        "required": ["label"],
                        "additionalProperties": false
                    }
                ]
            }
        },
        "required": ["node"],
        "$defs": {
            "Base": {
                "type": "object",
                "properties": { "id": { "type": "string" } },
                "required": ["id"],
                "additionalProperties": false
            }
        }
    });
    let (cx, reference) = lower_root(schema, "acme::Order", "Order", Direction::Both);

    let TypeKind::Struct { fields, .. } = &definition(&cx, &reference).definition else {
        panic!()
    };
    let (merged, openness) = inline_struct(&field(fields, "node").ty);
    assert_eq!(
        merged
            .iter()
            .map(|field| field.wire.as_str())
            .collect::<Vec<_>>(),
        ["id", "label"]
    );
    assert_eq!(field(merged, "id").presence, Presence::Required);
    assert_eq!(field(merged, "label").presence, Presence::Required);
    assert_eq!(openness, &ObjectOpenness::Reject);
    assert!(!has_code(&cx, &reference, "PONS-W202"));
}

#[test]
fn a_titled_inline_all_of_becomes_a_named_definition() {
    let schema = serde_json::json!({
        "type": "object",
        "properties": {
            "node": {
                "title": "Node",
                "allOf": [
                    {
                        "type": "object",
                        "properties": { "id": { "type": "string" } },
                        "required": ["id"]
                    },
                    {
                        "type": "object",
                        "properties": { "label": { "type": "string" } }
                    }
                ]
            }
        },
        "$defs": {}
    });
    let (cx, reference) = lower_root(schema, "acme::Order", "Order", Direction::Both);

    let TypeKind::Struct { fields, .. } = &definition(&cx, &reference).definition else {
        panic!()
    };
    let node = field(fields, "node");
    let TypeRef::Named(named) = &node.ty else {
        panic!()
    };
    assert_eq!(named.id, "acme::Order::Node");
    let TypeKind::Struct { fields: merged, .. } = &definition(&cx, &node.ty).definition else {
        panic!()
    };
    assert_eq!(
        merged
            .iter()
            .map(|field| field.wire.as_str())
            .collect::<Vec<_>>(),
        ["id", "label"]
    );
}

#[test]
fn a_conflicting_inline_all_of_becomes_opaque() {
    let schema = serde_json::json!({
        "type": "object",
        "properties": {
            "node": {
                "allOf": [
                    {
                        "type": "object",
                        "properties": { "id": { "type": "string" } }
                    },
                    {
                        "type": "object",
                        "properties": { "id": { "type": "integer" } }
                    }
                ]
            }
        }
    });
    let (cx, reference) = lower_root(schema, "acme::Order", "Order", Direction::Both);

    let TypeKind::Struct { fields, .. } = &definition(&cx, &reference).definition else {
        panic!()
    };
    assert_eq!(
        inline_opaque_codec(&field(fields, "node").ty),
        "json-schema:allOf"
    );
    assert!(has_code(&cx, &reference, "PONS-W202"));
}

#[test]
fn a_non_object_inline_all_of_becomes_opaque() {
    let schema = serde_json::json!({
        "type": "object",
        "properties": {
            "label": {
                "allOf": [
                    { "type": "object", "properties": { "id": { "type": "string" } } },
                    { "type": "string" }
                ]
            }
        }
    });
    let (cx, reference) = lower_root(schema, "acme::Order", "Order", Direction::Both);

    let TypeKind::Struct { fields, .. } = &definition(&cx, &reference).definition else {
        panic!()
    };
    assert_eq!(
        inline_opaque_codec(&field(fields, "label").ty),
        "json-schema:allOf"
    );
    assert!(has_code(&cx, &reference, "PONS-W202"));
}

#[test]
fn an_all_of_inside_an_array_item_merges() {
    let schema = serde_json::json!({
        "type": "object",
        "properties": {
            "nodes": {
                "type": "array",
                "items": {
                    "allOf": [
                        { "$ref": "#/$defs/Base" },
                        {
                            "type": "object",
                            "properties": { "label": { "type": "string" } },
                            "required": ["label"]
                        }
                    ]
                }
            }
        },
        "$defs": {
            "Base": {
                "type": "object",
                "properties": { "id": { "type": "string" } },
                "required": ["id"]
            }
        }
    });
    let (cx, reference) = lower_root(schema, "acme::Order", "Order", Direction::Both);

    let TypeKind::Struct { fields, .. } = &definition(&cx, &reference).definition else {
        panic!()
    };
    let TypeRef::Inline(InlineType::Array { item, .. }) = &field(fields, "nodes").ty else {
        panic!()
    };
    let (merged, _) = inline_struct(item);
    assert_eq!(
        merged
            .iter()
            .map(|field| field.wire.as_str())
            .collect::<Vec<_>>(),
        ["id", "label"]
    );
    assert_eq!(field(merged, "id").presence, Presence::Required);
    assert_eq!(field(merged, "label").presence, Presence::Required);
    assert!(!has_code(&cx, &reference, "PONS-W202"));
}

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
fn all_of_with_a_non_object_branch_becomes_opaque() {
    let schema = serde_json::json!({
        "allOf": [
            {
                "type": "object",
                "properties": { "id": { "type": "string" } }
            },
            { "type": "string" }
        ]
    });
    let (cx, reference) = lower_root(schema, "acme::NotAnObject", "NotAnObject", Direction::Both);

    assert_eq!(opaque_codec(&cx, &reference), "json-schema:allOf");
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
