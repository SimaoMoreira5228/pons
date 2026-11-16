use pons_core::TypeCx;
use pons_ir::{
    Direction, EnumRepr, InlineType, IntegerWire, ObjectOpenness, Presence, TypeKind, TypeRef,
};
use pons_json_schema::{RootType, lower};

use super::{definition, field};

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
