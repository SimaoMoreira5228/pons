use std::any::type_name;
use std::collections::BTreeMap;

use pons_core::{Schema, TypeCx};
use pons_ir::{
    Access, Direction, Document, EnumRepr, Field, FieldDefault, IR_VERSION, Info, InlineType,
    IntegerWire, ObjectOpenness, OpaqueWire, Presence, TypeKind, TypeRef, VariantPayload,
};
use pons_schemars::Schemars;
use schemars::generate::SchemaGenerator;
use serde::{Deserialize, Serialize};

#[derive(Debug, schemars::JsonSchema, Serialize, Deserialize)]
#[allow(dead_code)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Address {
    street: String,
    zip: String,
}

#[derive(Debug, schemars::JsonSchema, Serialize, Deserialize)]
#[allow(dead_code)]
#[serde(rename_all = "snake_case")]
enum Tier {
    Free,
    Pro,
}

#[derive(Debug, schemars::JsonSchema, Serialize, Deserialize)]
#[allow(dead_code)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Profile {
    user_id: u64,
    #[serde(rename = "e-mail")]
    email: String,
    #[serde(default)]
    nickname: Option<String>,
    #[serde(default)]
    active: bool,
    address: Address,
    tier: Tier,
    #[serde(skip_serializing)]
    secret: String,
    #[serde(skip_deserializing)]
    created_at: String,
}

#[derive(Debug, schemars::JsonSchema, Serialize, Deserialize)]
#[allow(dead_code)]
struct Collections {
    labels: BTreeMap<String, u32>,
    pair: (String, u16),
    explicit: i16,
}

#[derive(Debug, schemars::JsonSchema, Serialize, Deserialize)]
#[allow(dead_code)]
struct BareInteger {
    #[schemars(schema_with = "bare_integer")]
    value: i64,
}

fn bare_integer(_: &mut SchemaGenerator) -> schemars::Schema {
    schemars::json_schema!({"type": "integer"})
}

#[derive(Debug, schemars::JsonSchema, Serialize, Deserialize)]
#[allow(dead_code)]
#[schemars(extend("allOf" = [{"type": "object"}, {"type": "object"}]))]
struct Intersection {
    value: String,
}

#[derive(Debug, schemars::JsonSchema, Serialize, Deserialize)]
#[allow(dead_code)]
enum Choice {
    First,
    Second { value: u8 },
}

fn document(
    types: BTreeMap<String, pons_ir::TypeDef>,
    diagnostics: Vec<pons_ir::Diagnostic>,
) -> Document {
    Document {
        ir_version: IR_VERSION.to_owned(),
        info: Info {
            title: "schemars bridge".into(),
            version: "1".into(),
            description: None,
            servers: Vec::new(),
            ext: BTreeMap::new(),
        },
        types,
        auth_schemes: BTreeMap::new(),
        common_errors: Vec::new(),
        operations: Vec::new(),
        ext: BTreeMap::new(),
        diagnostics,
    }
}

fn struct_definition<'cx>(
    cx: &'cx TypeCx,
    reference: &TypeRef,
) -> (&'cx [Field], &'cx ObjectOpenness) {
    let TypeRef::Named(named) = reference else {
        panic!()
    };
    let TypeKind::Struct {
        fields, openness, ..
    } = &cx.definitions()[&named.id].definition
    else {
        panic!()
    };
    (fields, openness)
}

fn field<'a>(fields: &'a [Field], wire: &str) -> &'a Field {
    fields
        .iter()
        .find(|field| field.wire == wire)
        .unwrap_or_else(|| panic!("no field named {wire}"))
}

fn definition_diagnostics<'cx>(cx: &'cx TypeCx, reference: &TypeRef) -> &'cx [pons_ir::Diagnostic] {
    let TypeRef::Named(named) = reference else {
        panic!()
    };
    &cx.definitions()[&named.id].diagnostics
}

#[test]
fn objects_map_rename_optional_nullable_nested_refs_and_access() {
    let mut cx = TypeCx::new();
    let input = Schemars::<Profile>::schema(&mut cx, Direction::Input);
    let TypeRef::Named(input_root) = &input else {
        panic!()
    };
    assert_eq!(input_root.id, format!("{}#input", type_name::<Profile>()));

    let (fields, openness) = struct_definition(&cx, &input);
    assert_eq!(openness, &ObjectOpenness::Reject);
    assert_int(&field(fields, "userId").ty, 64, false);
    assert_eq!(field(fields, "e-mail").ident, vec!["e", "mail"]);
    assert!(matches!(
        field(fields, "nickname").ty,
        TypeRef::Inline(InlineType::Nullable { .. })
    ));
    assert_eq!(field(fields, "nickname").presence, Presence::Optional);
    assert_eq!(field(fields, "active").presence, Presence::Optional);
    assert_eq!(
        field(fields, "active").default,
        FieldDefault::Value(serde_json::json!(false))
    );
    assert_eq!(field(fields, "secret").access, Some(Access::WriteOnly));
    assert!(fields.iter().all(|field| field.wire != "createdAt"));

    let TypeRef::Named(address) = &field(fields, "address").ty else {
        panic!()
    };
    assert!(address.id.ends_with("::Address#input"));
    let TypeKind::Struct {
        fields: address_fields,
        ..
    } = &cx.definitions()[&address.id].definition
    else {
        panic!()
    };
    assert_eq!(
        address_fields
            .iter()
            .map(|field| field.wire.as_str())
            .collect::<Vec<_>>(),
        ["street", "zip"]
    );

    let TypeRef::Named(tier) = &field(fields, "tier").ty else {
        panic!()
    };
    let TypeKind::Enum { repr, variants } = &cx.definitions()[&tier.id].definition else {
        panic!()
    };
    assert_eq!(repr, &EnumRepr::String);
    assert_eq!(
        variants
            .iter()
            .map(|variant| variant.wire.as_str())
            .collect::<Vec<_>>(),
        ["free", "pro"]
    );

    let output = Schemars::<Profile>::schema(&mut cx, Direction::Output);
    let (fields, _) = struct_definition(&cx, &output);
    assert_eq!(field(fields, "nickname").presence, Presence::Required);
    assert_eq!(field(fields, "createdAt").access, Some(Access::ReadOnly));
    assert_eq!(field(fields, "active").presence, Presence::Required);
    assert_eq!(field(fields, "active").default, FieldDefault::Unspecified);
    let diagnostics = definition_diagnostics(&cx, &output);
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "PONS-W203"
                && diagnostic.message.contains("always present"))
    );
}

#[test]
fn a_string_enum_becomes_a_string_repr_enum() {
    let mut cx = TypeCx::new();
    let reference = Schemars::<Tier>::schema(&mut cx, Direction::Both);
    let TypeKind::Enum { repr, variants } = &cx.definitions()[match &reference {
        TypeRef::Named(named) => &named.id,
        TypeRef::Inline(_) => panic!(),
    }]
    .definition
    else {
        panic!()
    };
    assert_eq!(repr, &EnumRepr::String);
    assert_eq!(variants.len(), 2);
    assert!(matches!(&variants[0].payload, VariantPayload::Unit));
    assert_eq!(variants[0].ident, vec!["free"]);
    assert!(cx.diagnostics().is_empty());
}

#[test]
fn maps_tuples_and_integer_formats_keep_their_shapes() {
    let mut cx = TypeCx::new();
    let reference = Schemars::<Collections>::schema(&mut cx, Direction::Input);
    let (fields, _) = struct_definition(&cx, &reference);

    let TypeRef::Inline(InlineType::Map { key, value }) = &field(fields, "labels").ty else {
        panic!()
    };
    assert!(matches!(
        key.as_ref(),
        TypeRef::Inline(InlineType::String { .. })
    ));
    assert_int(value, 32, false);

    let TypeRef::Inline(InlineType::Tuple { items }) = &field(fields, "pair").ty else {
        panic!()
    };
    assert_eq!(items.len(), 2);
    assert!(matches!(
        items[0],
        TypeRef::Inline(InlineType::String { .. })
    ));

    let TypeRef::Inline(InlineType::Int {
        bits, signed, wire, ..
    }) = &field(fields, "explicit").ty
    else {
        panic!()
    };
    assert_eq!((*bits, *signed, *wire), (16, true, IntegerWire::Number));
    assert!(cx.diagnostics().is_empty());
}

#[test]
fn a_bare_integer_uses_64_bit_unsigned_and_reports_the_missing_width() {
    let mut cx = TypeCx::new();
    let reference = Schemars::<BareInteger>::schema(&mut cx, Direction::Input);
    let (fields, _) = struct_definition(&cx, &reference);
    assert_int(&field(fields, "value").ty, 64, false);

    let diagnostics = definition_diagnostics(&cx, &reference);
    assert_eq!(diagnostics.len(), 1);
    assert_eq!(diagnostics[0].code, "PONS-W201");
    assert!(diagnostics[0].message.contains("width"));
    assert!(diagnostics[0].message.contains("64-bit unsigned"));
}

#[test]
fn all_of_and_one_of_become_opaque_without_inventing_a_union() {
    let mut cx = TypeCx::new();
    let all_of = Schemars::<Intersection>::schema(&mut cx, Direction::Both);
    assert_opaque(&cx, &all_of, "json-schema:allOf", "PONS-W202");

    let one_of = Schemars::<Choice>::schema(&mut cx, Direction::Both);
    assert_opaque(&cx, &one_of, "json-schema:oneOf", "PONS-W202");
}

fn assert_opaque(cx: &TypeCx, reference: &TypeRef, codec: &str, code: &str) {
    let TypeRef::Named(named) = reference else {
        panic!()
    };
    let TypeKind::Newtype { inner } = &cx.definitions()[&named.id].definition else {
        panic!()
    };
    let TypeRef::Inline(InlineType::Opaque {
        codec: found, wire, ..
    }) = inner
    else {
        panic!()
    };
    assert_eq!(found, codec);
    assert_eq!(*wire, OpaqueWire::Json);
    let diagnostics = definition_diagnostics(cx, reference);
    assert!(diagnostics.iter().any(|diagnostic| diagnostic.code == code));
}

#[test]
fn the_wrapper_keeps_the_wire() {
    let wrapped = Schemars::new(Address {
        street: "main".into(),
        zip: "123".into(),
    });
    let json = serde_json::to_value(&wrapped).unwrap();
    assert_eq!(json, serde_json::json!({"street": "main", "zip": "123"}));
    let back: Schemars<Address> = serde_json::from_value(json).unwrap();
    assert_eq!(back.into_inner().street, "main");
}

#[test]
fn a_document_built_from_the_wrapper_validates() {
    let mut cx = TypeCx::new();
    let profile = Schemars::<Profile>::schema(&mut cx, Direction::Input);
    let collections = Schemars::<Collections>::schema(&mut cx, Direction::Input);
    let intersection = Schemars::<Intersection>::schema(&mut cx, Direction::Both);
    assert!(matches!(profile, TypeRef::Named(_)));
    assert!(matches!(collections, TypeRef::Named(_)));
    assert!(matches!(intersection, TypeRef::Named(_)));

    let (types, diagnostics) = cx.into_parts();
    let document = document(types, diagnostics);
    document.validate().unwrap();
    assert!(document.has_opaque());
}

fn assert_int(reference: &TypeRef, bits: u16, signed: bool) {
    let TypeRef::Inline(InlineType::Int {
        bits: found_bits,
        signed: found_signed,
        wire: IntegerWire::Number,
        ..
    }) = reference
    else {
        panic!()
    };
    assert_eq!((*found_bits, *found_signed), (bits, signed));
}
