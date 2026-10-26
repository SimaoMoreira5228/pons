mod support;

use pons_core::{Schema, TypeCx};
use pons_ir::{DiagnosticSeverity, Direction, EnumRepr, TypeKind, TypeRef, VariantPayload};
use support::{ExternalShape, LoginState, document, error_coded};

const OPEN_VARIANT_KEY: &str = "com.pons.open_variant/v1";

#[derive(pons_core::Schema)]
#[allow(dead_code)]
enum OpenVariant {
    Known,
    #[serde(other)]
    Unknown,
}

#[derive(pons_core::Schema)]
#[serde(tag = "kind", deny_unknown_fields)]
#[allow(dead_code)]
struct TaggedStruct {
    value: String,
}

#[derive(pons_core::Schema)]
#[serde(tag = "kind")]
#[allow(dead_code)]
enum MixedTaggedVariants {
    Ready,
    #[serde(untagged)]
    Loose {
        value: u32,
    },
}

#[derive(pons_core::Schema)]
#[serde(untagged)]
#[allow(dead_code)]
enum UntaggedOverlap {
    Number { amount: u32 },
    Text { body: String },
}

#[derive(pons_core::Schema)]
#[serde(untagged)]
#[allow(dead_code)]
enum UntaggedUnits {
    Nothing,
    AlsoNothing,
}

#[derive(pons_core::Schema)]
#[serde(untagged)]
#[allow(dead_code)]
enum UntaggedSingle {
    Only(u32),
}

#[derive(pons_core::Schema)]
#[serde(deny_unknown_fields)]
#[allow(dead_code)]
enum ClosedEnum {
    Closed { value: u32 },
}

#[test]
fn an_other_variant_is_carried_only_on_the_side_that_accepts_it() {
    let mut cx = TypeCx::new();
    let input_type = OpenVariant::schema(&mut cx, Direction::Input);
    let output_type = OpenVariant::schema(&mut cx, Direction::Output);
    let both_type = OpenVariant::schema(&mut cx, Direction::Both);
    let TypeRef::Named(input_type) = input_type else {
        panic!()
    };
    let TypeRef::Named(output_type) = output_type else {
        panic!()
    };
    let TypeRef::Named(both_type) = both_type else {
        panic!()
    };
    let catch_all_of = |cx: &TypeCx, id: &str| -> bool {
        let TypeKind::Enum { variants, .. } = &cx.definitions()[id].definition else {
            panic!()
        };
        assert_eq!(variants.len(), 2);
        variants[1].ext.get(OPEN_VARIANT_KEY) == Some(&serde_json::json!(true))
    };
    assert!(catch_all_of(&cx, &input_type.id));
    assert!(!catch_all_of(&cx, &output_type.id));
    assert!(catch_all_of(&cx, &both_type.id));
    assert!(cx.diagnostics().is_empty());
    let (types, diagnostics) = cx.into_parts();
    assert!(document(types, diagnostics).validate().is_ok());
}

#[test]
fn derives_tagged_enums_and_named_payloads() {
    let mut cx = TypeCx::new();
    let reference = LoginState::schema(&mut cx, Direction::Output);
    let TypeRef::Named(named) = reference else {
        panic!();
    };
    let TypeKind::Enum { repr, variants } = &cx.definitions()[&named.id].definition else {
        panic!();
    };

    assert_eq!(repr, &EnumRepr::Internal { tag: "kind".into() });
    assert_eq!(variants[0].wire, "ready");
    assert_eq!(variants[1].wire, "locked");
    let VariantPayload::Struct { fields } = &variants[1].payload else {
        panic!();
    };
    assert_eq!(fields[0].wire, "retry_after");
}

#[test]
fn enum_representation_tracks_directional_variant_skips() {
    let mut cx = TypeCx::new();
    let input = ExternalShape::schema(&mut cx, Direction::Input);
    let output = ExternalShape::schema(&mut cx, Direction::Output);
    let TypeRef::Named(input) = input else {
        panic!()
    };
    let TypeRef::Named(output) = output else {
        panic!()
    };
    let TypeKind::Enum {
        repr: input_repr, ..
    } = &cx.definitions()[&input.id].definition
    else {
        panic!()
    };
    let TypeKind::Enum {
        repr: output_repr, ..
    } = &cx.definitions()[&output.id].definition
    else {
        panic!()
    };
    assert_eq!(input_repr, &EnumRepr::External);
    assert_eq!(output_repr, &EnumRepr::String);
}

#[test]
fn serde_tag_on_a_struct_is_refused_instead_of_dropped() {
    let mut cx = TypeCx::new();
    let reference = TaggedStruct::schema(&mut cx, Direction::Output);
    let TypeRef::Named(named) = reference else {
        panic!()
    };
    let TypeKind::Struct { openness, .. } = &cx.definitions()[&named.id].definition else {
        panic!()
    };
    assert_eq!(openness, &pons_ir::ObjectOpenness::Reject);

    let diagnostic = error_coded(&cx, "PONS-E101", "kind");
    assert!(diagnostic.message.contains("no literal type"));
    let (types, diagnostics) = cx.into_parts();
    assert!(document(types, diagnostics).validate().is_err());
}

#[test]
fn an_untagged_variant_under_a_tagged_container_is_refused() {
    let mut cx = TypeCx::new();
    let _ = MixedTaggedVariants::schema(&mut cx, Direction::Output);
    let diagnostic = error_coded(&cx, "PONS-E101", "Loose");
    assert!(diagnostic.message.contains("untagged"));
    assert!(diagnostic.message.contains("EnumRepr"));
    let (types, diagnostics) = cx.into_parts();
    assert!(document(types, diagnostics).validate().is_err());
}

#[test]
fn an_untagged_container_with_several_payload_variants_warns_on_its_type() {
    let mut cx = TypeCx::new();
    let reference = UntaggedOverlap::schema(&mut cx, Direction::Output);
    let TypeRef::Named(named) = reference else {
        panic!()
    };
    let definition = &cx.definitions()[&named.id];
    let TypeKind::Enum { repr, variants } = &definition.definition else {
        panic!()
    };
    assert_eq!(repr, &EnumRepr::Untagged);
    assert_eq!(variants.len(), 2);
    assert_eq!(definition.diagnostics.len(), 1);
    let diagnostic = &definition.diagnostics[0];
    assert_eq!(diagnostic.severity, DiagnosticSeverity::Warning);
    assert_eq!(diagnostic.code, "PONS-W104");
    assert!(diagnostic.message.contains("Number, Text"));
    assert!(diagnostic.message.contains("no discriminator"));
    assert!(cx.diagnostics().is_empty());
    let (types, diagnostics) = cx.into_parts();
    assert!(document(types, diagnostics).validate().is_ok());
}

#[test]
fn a_unit_only_untagged_enum_is_describable_without_a_warning() {
    let mut cx = TypeCx::new();
    let reference = UntaggedUnits::schema(&mut cx, Direction::Output);
    let TypeRef::Named(named) = reference else {
        panic!()
    };
    let definition = &cx.definitions()[&named.id];
    let TypeKind::Enum { repr, variants } = &definition.definition else {
        panic!()
    };
    assert_eq!(repr, &EnumRepr::Untagged);
    assert_eq!(variants.len(), 2);
    assert!(definition.diagnostics.is_empty());
    assert!(cx.diagnostics().is_empty());
    let (types, diagnostics) = cx.into_parts();
    assert!(document(types, diagnostics).validate().is_ok());
}

#[test]
fn a_single_payload_untagged_enum_stays_describable() {
    let mut cx = TypeCx::new();
    let reference = UntaggedSingle::schema(&mut cx, Direction::Output);
    let TypeRef::Named(named) = reference else {
        panic!()
    };
    let TypeKind::Enum { repr, variants } = &cx.definitions()[&named.id].definition else {
        panic!()
    };
    assert_eq!(repr, &EnumRepr::Untagged);
    assert_eq!(variants.len(), 1);
    assert!(cx.diagnostics().is_empty());
    let (types, diagnostics) = cx.into_parts();
    assert!(document(types, diagnostics).validate().is_ok());
}

#[test]
fn deny_unknown_fields_on_an_enum_is_refused() {
    let mut cx = TypeCx::new();
    let _ = ClosedEnum::schema(&mut cx, Direction::Input);
    let diagnostic = error_coded(&cx, "PONS-E101", "deny_unknown_fields");
    assert!(diagnostic.message.contains("VariantPayload::Struct"));
    let (types, diagnostics) = cx.into_parts();
    assert!(document(types, diagnostics).validate().is_err());
}

#[test]
fn both_describes_the_representation_the_included_variants_share() {
    let mut cx = TypeCx::new();
    let both = ExternalShape::schema(&mut cx, Direction::Both);
    let TypeRef::Named(named) = both else {
        panic!()
    };
    let TypeKind::Enum { repr, variants } = &cx.definitions()[&named.id].definition else {
        panic!()
    };
    assert_eq!(
        repr,
        &EnumRepr::String,
        "the only variant Both carries is unit, and serde writes a bare unit variant as a string"
    );
    assert_eq!(
        variants
            .iter()
            .map(|variant| variant.wire.as_str())
            .collect::<Vec<_>>(),
        ["Plain"]
    );
    assert!(cx.diagnostics().is_empty());
    let (types, diagnostics) = cx.into_parts();
    assert!(document(types, diagnostics).validate().is_ok());
}
