use pons_core::TypeCx;
use pons_ir::{
    Direction, EnumRepr, Field, InlineType, IntegerWire, ObjectOpenness, Presence, TypeDef,
    TypeKind, TypeRef,
};
use pons_json_schema::{RootType, lower};

fn definition<'cx>(cx: &'cx TypeCx, reference: &TypeRef) -> &'cx TypeDef {
    let TypeRef::Named(named) = reference else {
        panic!()
    };
    &cx.definitions()[&named.id]
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
