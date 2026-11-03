use std::any::type_name;
use std::collections::BTreeMap;

use pons_core::{Schema, TypeCx};
use pons_ir::{
    Direction, Document, EnumRepr, Field, IR_VERSION, Info, InlineType, ObjectOpenness, OpaqueWire,
    Presence, TypeKind, TypeRef, VariantPayload,
};
use pons_utoipa::Utoipa;
use serde::{Deserialize, Serialize};

#[derive(Debug, utoipa::ToSchema, Serialize, Deserialize)]
#[allow(dead_code)]
struct Address {
    street: String,
    zip: String,
}

#[derive(Debug, utoipa::ToSchema, Serialize, Deserialize)]
#[schema(title = "Tier")]
#[serde(rename_all = "snake_case")]
#[allow(dead_code)]
enum Tier {
    Free,
    Pro,
}

#[derive(Debug, utoipa::ToSchema, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[allow(dead_code)]
struct Account {
    user_id: i64,
    #[serde(rename = "e-mail")]
    email: String,
    nickname: Option<String>,
    labels: Vec<String>,
    #[schema(inline)]
    tier: Tier,
    address: Address,
}

fn document(
    types: BTreeMap<String, pons_ir::TypeDef>,
    diagnostics: Vec<pons_ir::Diagnostic>,
) -> Document {
    Document {
        ir_version: IR_VERSION.to_owned(),
        info: Info {
            title: "utoipa bridge".into(),
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
fn objects_keep_renames_optional_fields_arrays_and_nested_enums() {
    let mut cx = TypeCx::new();
    let input = Utoipa::<Account>::schema(&mut cx, Direction::Input);
    let TypeRef::Named(input_root) = &input else {
        panic!()
    };
    assert_eq!(input_root.id, format!("{}#input", type_name::<Account>()));
    assert_eq!(cx.definitions()[&input_root.id].name, vec!["account"]);

    let (fields, openness) = struct_definition(&cx, &input);
    assert_eq!(openness, &ObjectOpenness::Reject);
    assert_eq!(
        fields
            .iter()
            .map(|field| field.wire.as_str())
            .collect::<Vec<_>>(),
        ["address", "e-mail", "labels", "nickname", "tier", "userId"]
    );

    let TypeRef::Inline(InlineType::Int { bits, signed, .. }) = &field(fields, "userId").ty else {
        panic!()
    };
    assert_eq!((*bits, *signed), (64, true));

    let email = field(fields, "e-mail");
    assert_eq!(email.ident, vec!["e", "mail"]);
    assert!(matches!(
        email.ty,
        TypeRef::Inline(InlineType::String { format: None, .. })
    ));

    let nickname = field(fields, "nickname");
    assert_eq!(nickname.presence, Presence::Optional);
    assert!(matches!(
        nickname.ty,
        TypeRef::Inline(InlineType::Nullable { .. })
    ));

    let TypeRef::Inline(InlineType::Array { item, .. }) = &field(fields, "labels").ty else {
        panic!()
    };
    assert!(matches!(
        item.as_ref(),
        TypeRef::Inline(InlineType::String { format: None, .. })
    ));

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
    assert!(
        variants
            .iter()
            .all(|variant| matches!(variant.payload, VariantPayload::Unit))
    );
}

#[test]
fn a_string_enum_becomes_a_string_repr_enum() {
    let mut cx = TypeCx::new();
    let reference = Utoipa::<Tier>::schema(&mut cx, Direction::Both);
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
            .map(|variant| variant.ident.clone())
            .collect::<Vec<_>>(),
        [vec!["free"], vec!["pro"]]
    );
    assert!(cx.diagnostics().is_empty());
}

#[test]
fn a_referenced_type_stays_opaque_with_a_diagnostic() {
    let mut cx = TypeCx::new();
    let reference = Utoipa::<Account>::schema(&mut cx, Direction::Input);
    let (fields, _) = struct_definition(&cx, &reference);

    let TypeRef::Inline(InlineType::Opaque { codec, wire, .. }) = &field(fields, "address").ty
    else {
        panic!()
    };
    assert_eq!(codec, "json-schema:ref");
    assert_eq!(*wire, OpaqueWire::Json);

    let diagnostics = definition_diagnostics(&cx, &reference);
    let diagnostic = diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == "PONS-W202")
        .expect("the unresolved reference is reported");
    assert!(diagnostic.message.contains("#/components/schemas/Address"));
}

#[test]
fn the_wrapper_keeps_the_wire() {
    let wrapped = Utoipa::new(Account {
        user_id: 7,
        email: "main@example.com".into(),
        nickname: None,
        labels: vec!["a".into()],
        tier: Tier::Pro,
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
            "nickname": null,
            "labels": ["a"],
            "tier": "pro",
            "address": {"street": "main", "zip": "123"}
        })
    );
    let back: Utoipa<Account> = serde_json::from_value(json).unwrap();
    assert_eq!(back.into_inner().user_id, 7);
}

#[test]
fn a_document_built_from_the_wrapper_validates() {
    let mut cx = TypeCx::new();
    let account = Utoipa::<Account>::schema(&mut cx, Direction::Input);
    let tier = Utoipa::<Tier>::schema(&mut cx, Direction::Both);
    assert!(matches!(account, TypeRef::Named(_)));
    assert!(matches!(tier, TypeRef::Named(_)));

    let (types, diagnostics) = cx.into_parts();
    let document = document(types, diagnostics);
    document.validate().unwrap();
    assert!(document.has_opaque());
}
