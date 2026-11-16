use pons_core::TypeCx;
use pons_ir::{Direction, InlineType, ObjectOpenness, TypeKind, TypeRef};
use pons_json_schema::{RootType, lower};

use super::{definition, has_code, lower_root, opaque_codec};

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
