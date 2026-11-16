use pons_ir::{Direction, Field, InlineType, ObjectOpenness, Presence, TypeKind, TypeRef};

use super::{definition, field, has_code, lower_root};

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
