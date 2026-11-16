use pons_ir::{Direction, InlineType, ObjectOpenness, Presence, TypeKind, TypeRef};

use super::{definition, field, has_code, lower_root};

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
