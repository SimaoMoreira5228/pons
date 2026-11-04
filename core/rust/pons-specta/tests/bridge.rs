use std::any::type_name;
use std::collections::BTreeMap;

use pons_core::{Schema, TypeCx};
use pons_ir::{
    Direction, Document, EnumRepr, Field, IR_VERSION, Info, InlineType, IntegerWire,
    ObjectOpenness, OpaqueWire, Presence, TypeKind, TypeRef, VariantPayload,
};
use pons_specta::Specta;
use serde::{Deserialize, Serialize};

#[derive(Debug, specta::Type, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[allow(dead_code)]
struct Address {
    street: String,
    zip: String,
}

#[derive(Debug, specta::Type, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[allow(dead_code)]
struct Account {
    user_id: i64,
    #[serde(rename = "e-mail")]
    email: String,
    #[specta(optional)]
    note: String,
    nickname: Option<String>,
    address: Address,
}

#[derive(Debug, specta::Type, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[allow(dead_code)]
enum Tier {
    Free,
    Pro,
}

#[derive(Debug, specta::Type, Serialize, Deserialize)]
#[serde(tag = "kind", content = "data", rename_all = "snake_case")]
#[allow(dead_code)]
enum Event {
    Created { id: u64, label: String },
    Deleted(String),
    Ping,
    Pair(u32, String),
}

#[derive(Debug, specta::Type, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
#[allow(dead_code)]
enum State {
    Ready { code: u32 },
    Idle,
}

#[derive(Debug, specta::Type, Serialize, Deserialize)]
#[allow(dead_code)]
struct Directory {
    entries: BTreeMap<String, u64>,
}

#[derive(Debug, specta::Type, Serialize, Deserialize)]
#[allow(dead_code)]
struct Node {
    value: u32,
    children: Vec<Node>,
}

struct Intersected;

impl specta::Type for Intersected {
    fn definition(_: &mut specta::Types) -> specta::datatype::DataType {
        specta::datatype::DataType::Intersection(Vec::new())
    }
}

fn document(
    types: BTreeMap<String, pons_ir::TypeDef>,
    diagnostics: Vec<pons_ir::Diagnostic>,
) -> Document {
    Document {
        ir_version: IR_VERSION.to_owned(),
        info: Info {
            title: "specta bridge".into(),
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

#[test]
fn objects_keep_renames_optionality_nullability_and_nested_references() {
    let mut cx = TypeCx::new();
    let reference = Specta::<Account>::schema(&mut cx, Direction::Input);
    let TypeRef::Named(named) = &reference else {
        panic!()
    };
    assert_eq!(named.id, format!("{}#input", type_name::<Account>()));
    assert_eq!(cx.definitions()[&named.id].name, vec!["account"]);

    let (fields, openness) = struct_definition(&cx, &reference);
    assert_eq!(openness, &ObjectOpenness::Reject);

    let user_id = field(fields, "userId");
    let TypeRef::Inline(InlineType::Int {
        bits, signed, wire, ..
    }) = &user_id.ty
    else {
        panic!()
    };
    assert_eq!((*bits, *signed), (64, true));
    assert_eq!(*wire, IntegerWire::Number);

    let email = field(fields, "e-mail");
    assert_eq!(email.ident, vec!["e", "mail"]);
    assert!(matches!(
        email.ty,
        TypeRef::Inline(InlineType::String { format: None, .. })
    ));

    let note = field(fields, "note");
    assert_eq!(note.presence, Presence::Optional);

    let nickname = field(fields, "nickname");
    assert_eq!(nickname.presence, Presence::Required);
    let TypeRef::Inline(InlineType::Nullable { inner }) = &nickname.ty else {
        panic!()
    };
    assert!(matches!(
        inner.as_ref(),
        TypeRef::Inline(InlineType::String { format: None, .. })
    ));

    let TypeRef::Named(address) = &field(fields, "address").ty else {
        panic!()
    };
    assert_eq!(address.id, format!("{}#input", type_name::<Address>()));
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
}

#[test]
fn a_string_enum_becomes_a_string_repr_enum() {
    let mut cx = TypeCx::new();
    let reference = Specta::<Tier>::schema(&mut cx, Direction::Both);
    let TypeRef::Named(named) = &reference else {
        panic!()
    };
    let TypeKind::Enum { repr, variants } = &cx.definitions()[&named.id].definition else {
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
    assert!(
        variants
            .iter()
            .all(|variant| matches!(variant.payload, VariantPayload::Unit))
    );
}

#[test]
fn tagged_enums_keep_their_representation_and_payloads() {
    let mut cx = TypeCx::new();
    let reference = Specta::<Event>::schema(&mut cx, Direction::Both);
    let TypeRef::Named(named) = &reference else {
        panic!()
    };
    let TypeKind::Enum { repr, variants } = &cx.definitions()[&named.id].definition else {
        panic!()
    };
    assert_eq!(
        repr,
        &EnumRepr::Adjacent {
            tag: "kind".into(),
            content: "data".into()
        }
    );

    let created = variants
        .iter()
        .find(|variant| variant.wire == "created")
        .expect("created variant");
    let VariantPayload::Struct { fields } = &created.payload else {
        panic!()
    };
    assert_eq!(
        fields
            .iter()
            .map(|field| field.wire.as_str())
            .collect::<Vec<_>>(),
        ["id", "label"]
    );

    let deleted = variants
        .iter()
        .find(|variant| variant.wire == "deleted")
        .expect("deleted variant");
    assert!(matches!(deleted.payload, VariantPayload::Newtype { .. }));

    let ping = variants
        .iter()
        .find(|variant| variant.wire == "ping")
        .expect("ping variant");
    assert!(matches!(ping.payload, VariantPayload::Unit));

    let pair = variants
        .iter()
        .find(|variant| variant.wire == "pair")
        .expect("pair variant");
    let VariantPayload::Tuple { items } = &pair.payload else {
        panic!()
    };
    assert_eq!(items.len(), 2);

    let state = Specta::<State>::schema(&mut cx, Direction::Both);
    let TypeRef::Named(state) = &state else {
        panic!()
    };
    let TypeKind::Enum { repr, .. } = &cx.definitions()[&state.id].definition else {
        panic!()
    };
    assert_eq!(repr, &EnumRepr::Internal { tag: "type".into() });
}

#[test]
fn maps_become_maps() {
    let mut cx = TypeCx::new();
    let reference = Specta::<Directory>::schema(&mut cx, Direction::Both);
    let (fields, _) = struct_definition(&cx, &reference);
    let TypeRef::Inline(InlineType::Map { key, value }) = &field(fields, "entries").ty else {
        panic!()
    };
    assert!(matches!(
        key.as_ref(),
        TypeRef::Inline(InlineType::String { format: None, .. })
    ));
    let TypeRef::Inline(InlineType::Int {
        bits, signed, wire, ..
    }) = value.as_ref()
    else {
        panic!()
    };
    assert_eq!((*bits, *signed), (64, false));
    assert_eq!(*wire, IntegerWire::Number);
}

#[test]
fn a_recursive_type_becomes_a_named_reference() {
    let mut cx = TypeCx::new();
    let reference = Specta::<Node>::schema(&mut cx, Direction::Both);
    let TypeRef::Named(named) = &reference else {
        panic!()
    };
    let (fields, _) = struct_definition(&cx, &reference);
    let TypeRef::Inline(InlineType::Array { item, .. }) = &field(fields, "children").ty else {
        panic!()
    };
    let TypeRef::Named(children) = item.as_ref() else {
        panic!()
    };
    assert_eq!(children.id, named.id);
}

#[test]
fn an_unrepresentable_construct_becomes_opaque_with_a_diagnostic() {
    let mut cx = TypeCx::new();
    let reference = Specta::<Intersected>::schema(&mut cx, Direction::Both);
    let TypeRef::Inline(InlineType::Opaque { codec, wire, .. }) = &reference else {
        panic!()
    };
    assert_eq!(codec, "specta:intersection");
    assert_eq!(*wire, OpaqueWire::Unknown);
    assert!(
        cx.diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.code == "PONS-W202")
    );
}

#[test]
fn the_wrapper_keeps_the_wire() {
    let wrapped = Specta::new(Account {
        user_id: 7,
        email: "main@example.com".into(),
        note: "hello".into(),
        nickname: None,
        address: Address {
            street: "main".into(),
            zip: "123".into(),
        },
    });
    let json = serde_json::to_value(&wrapped).unwrap();
    assert_eq!(
        json,
        serde_json::json!({
            "userId": 7,
            "e-mail": "main@example.com",
            "note": "hello",
            "nickname": null,
            "address": {"street": "main", "zip": "123"}
        })
    );
    let back: Specta<Account> = serde_json::from_value(json).unwrap();
    assert_eq!(back.into_inner().user_id, 7);
}

#[test]
fn a_document_built_from_the_wrapper_validates() {
    let mut cx = TypeCx::new();
    Specta::<Account>::schema(&mut cx, Direction::Input);
    Specta::<Tier>::schema(&mut cx, Direction::Both);
    Specta::<Event>::schema(&mut cx, Direction::Both);
    Specta::<Directory>::schema(&mut cx, Direction::Both);
    Specta::<Node>::schema(&mut cx, Direction::Both);

    let (types, diagnostics) = cx.into_parts();
    let document = document(types, diagnostics);
    document.validate().unwrap();
    assert!(!document.has_opaque());
}
