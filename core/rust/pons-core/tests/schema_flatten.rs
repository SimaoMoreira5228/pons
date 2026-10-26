mod support;

use pons_core::{Schema, TypeCx};
use pons_ir::{
    DiagnosticSeverity, Direction, InlineType, OpaqueWire, TypeKind, TypeRef, VariantPayload,
};
use std::collections::BTreeMap;
use support::{AuditFields, FlattenedMap, FlattenedStruct, document, struct_shape};

#[derive(pons_core::Schema)]
#[allow(dead_code)]
enum FlattenedCustomEnumEncoding {
    Struct {
        #[serde(flatten, with = "custom_wire")]
        value: String,
    },
}

#[derive(pons_core::Schema)]
#[allow(dead_code)]
enum FlattenedEnumPayload {
    Struct {
        id: u64,
        #[serde(flatten)]
        audit: AuditFields,
    },
}

#[derive(pons_core::Schema)]
#[allow(dead_code)]
enum FlattenedMapEnumPayload {
    Struct {
        id: u64,
        #[serde(flatten)]
        extra: BTreeMap<String, String>,
    },
}

#[derive(pons_core::Schema)]
#[allow(dead_code)]
struct AuditFieldsWithMap {
    created_by: String,
    #[serde(flatten)]
    extra: BTreeMap<String, String>,
}

#[derive(pons_core::Schema)]
#[allow(dead_code)]
enum NestedFlattenedEnumPayload {
    Struct {
        id: u64,
        #[serde(flatten)]
        audit: AuditFieldsWithMap,
    },
}

#[derive(pons_core::Schema)]
#[allow(dead_code)]
struct FlattenedCustomEncoding {
    #[serde(flatten, with = "custom_wire")]
    value: String,
}

#[derive(pons_core::Schema)]
#[serde(deny_unknown_fields)]
#[allow(dead_code)]
struct ClosedFlattenedCustomEncoding {
    id: u64,
    #[serde(flatten, with = "custom_wire")]
    value: String,
}

#[derive(pons_core::Schema)]
#[allow(dead_code)]
struct NullableFlatten {
    #[serde(flatten)]
    inner: Option<AuditFields>,
}

#[derive(pons_core::Schema)]
#[allow(dead_code)]
struct FlattenedNestedMapStruct {
    id: u64,
    #[serde(flatten)]
    audit: AuditFieldsWithMap,
}

#[derive(pons_core::Schema)]
#[allow(dead_code)]
struct OneSidedCodecFlatten {
    id: u64,
    #[serde(flatten, serialize_with = "encode_value")]
    audit: AuditFields,
}

#[test]
fn flattened_custom_enum_payload_fails_validation_instead_of_naming_a_field() {
    let mut cx = TypeCx::new();
    let reference = FlattenedCustomEnumEncoding::schema(&mut cx, Direction::Output);
    let TypeRef::Named(named) = reference else {
        panic!()
    };
    let TypeKind::Enum { variants, .. } = &cx.definitions()[&named.id].definition else {
        panic!()
    };
    let VariantPayload::Struct { fields } = &variants[0].payload else {
        panic!()
    };
    assert_eq!(fields.len(), 1);
    assert!(matches!(
        fields[0].ty,
        TypeRef::Inline(InlineType::Opaque { .. })
    ));
    let error = &fields[0].diagnostics[0];
    assert_eq!(error.severity, DiagnosticSeverity::Error);
    assert_eq!(error.code, "PONS-E101");
    assert!(error.message.contains("flatten"));

    let (types, diagnostics) = cx.into_parts();
    let document = document(types, diagnostics);
    let errors = document.validate().unwrap_err();
    assert!(
        errors
            .iter()
            .any(|error| error.to_string().contains("PONS-E101"))
    );
    assert!(document.has_opaque());
}

#[test]
fn struct_flatten_in_enum_payload_expands_to_named_fields() {
    let mut cx = TypeCx::new();
    let reference = FlattenedEnumPayload::schema(&mut cx, Direction::Output);
    let TypeRef::Named(named) = reference else {
        panic!()
    };
    let TypeKind::Enum { variants, .. } = &cx.definitions()[&named.id].definition else {
        panic!()
    };
    let VariantPayload::Struct { fields } = &variants[0].payload else {
        panic!()
    };
    assert_eq!(
        fields
            .iter()
            .map(|field| field.wire.as_str())
            .collect::<Vec<_>>(),
        ["id", "created_by"]
    );
    assert!(cx.diagnostics().is_empty());

    let (types, diagnostics) = cx.into_parts();
    assert!(document(types, diagnostics).validate().is_ok());
}

#[test]
fn enum_payload_map_flatten_is_an_error_instead_of_a_narrowed_payload() {
    let mut cx = TypeCx::new();
    let reference = FlattenedMapEnumPayload::schema(&mut cx, Direction::Output);
    let TypeRef::Named(named) = reference else {
        panic!()
    };
    let TypeKind::Enum { variants, .. } = &cx.definitions()[&named.id].definition else {
        panic!()
    };
    let VariantPayload::Struct { fields } = &variants[0].payload else {
        panic!()
    };

    assert_eq!(
        fields
            .iter()
            .map(|field| field.wire.as_str())
            .collect::<Vec<_>>(),
        ["id"]
    );
    let diagnostic = &cx.diagnostics()[0];
    assert_eq!(diagnostic.severity, DiagnosticSeverity::Error);
    assert_eq!(diagnostic.code, "PONS-E101");
    assert!(diagnostic.message.contains("flatten"));
    assert!(diagnostic.message.contains("VariantPayload::Struct"));

    let (types, diagnostics) = cx.into_parts();
    let errors = document(types, diagnostics).validate().unwrap_err();
    assert!(
        errors
            .iter()
            .any(|error| error.to_string().contains("PONS-E101"))
    );
}

#[test]
fn nested_flatten_inside_an_enum_payload_is_reported_instead_of_dropped() {
    let mut cx = TypeCx::new();
    let reference = NestedFlattenedEnumPayload::schema(&mut cx, Direction::Output);
    let TypeRef::Named(named) = reference else {
        panic!()
    };
    let TypeKind::Enum { variants, .. } = &cx.definitions()[&named.id].definition else {
        panic!()
    };
    let VariantPayload::Struct { fields } = &variants[0].payload else {
        panic!()
    };

    assert_eq!(
        fields
            .iter()
            .map(|field| field.wire.as_str())
            .collect::<Vec<_>>(),
        ["id"]
    );
    let refused: Vec<&str> = cx
        .diagnostics()
        .iter()
        .map(|diagnostic| diagnostic.code.as_str())
        .collect();
    assert_eq!(refused, ["PONS-E104", "PONS-E101"]);
    assert!(
        cx.diagnostics()
            .iter()
            .all(|diagnostic| diagnostic.severity == DiagnosticSeverity::Error)
    );

    let (types, diagnostics) = cx.into_parts();
    assert!(document(types, diagnostics).validate().is_err());
}

#[test]
fn a_flattened_map_is_refused_instead_of_claiming_a_member_shape() {
    let mut cx = TypeCx::new();
    let reference = FlattenedNestedMapStruct::schema(&mut cx, Direction::Output);
    let TypeRef::Named(named) = reference else {
        panic!()
    };
    let TypeKind::Struct {
        fields, flattened, ..
    } = &cx.definitions()[&named.id].definition
    else {
        panic!()
    };
    assert_eq!(
        fields
            .iter()
            .map(|field| field.wire.as_str())
            .collect::<Vec<_>>(),
        ["id", "created_by"]
    );
    assert!(matches!(
        flattened.as_slice(),
        [TypeRef::Inline(InlineType::Map { .. })]
    ));
    assert!(
        cx.diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.code == "PONS-E104")
    );
    let (types, diagnostics) = cx.into_parts();
    assert!(document(types, diagnostics).validate().is_err());
}

#[test]
fn custom_serde_functions_with_flatten_are_diagnosed_and_opaque() {
    let mut cx = TypeCx::new();
    let reference = FlattenedCustomEncoding::schema(&mut cx, Direction::Output);
    let TypeRef::Named(named) = reference else {
        panic!()
    };
    let TypeKind::Struct {
        fields, flattened, ..
    } = &cx.definitions()[&named.id].definition
    else {
        panic!()
    };
    assert!(fields.is_empty());
    let TypeRef::Inline(InlineType::Opaque { codec, wire, .. }) = &flattened[0] else {
        panic!()
    };
    assert!(codec.contains("custom_wire"));
    assert_eq!(*wire, OpaqueWire::Unknown);
    assert!(cx.diagnostics().iter().any(|diagnostic| {
        diagnostic.code == "PONS-W101"
            && diagnostic.message.contains("custom serde handler")
            && diagnostic.message.contains("flatten")
    }));
}

#[test]
fn closed_custom_flatten_stays_opaque_instead_of_exact() {
    let mut cx = TypeCx::new();
    let reference = ClosedFlattenedCustomEncoding::schema(&mut cx, Direction::Input);
    let TypeRef::Named(named) = reference else {
        panic!()
    };
    let (types, diagnostics) = cx.into_parts();
    let document = document(types, diagnostics);
    assert!(document.validate().is_ok());
    let encoded = serde_json::to_value(&document).unwrap();
    let definition = &encoded["types"][named.id.as_str()];
    assert_eq!(definition["openness"], "reject");
    assert_eq!(definition["flattened"][0]["kind"], "opaque");
    assert_eq!(definition["flattened"][0]["wire"], "unknown");
}

#[test]
fn nullable_flatten_is_diagnosed_and_lowered_to_an_unknown_type() {
    let mut cx = TypeCx::new();
    let reference = NullableFlatten::schema(&mut cx, Direction::Input);
    let TypeRef::Named(named) = reference else {
        panic!()
    };
    let TypeKind::Struct {
        fields, flattened, ..
    } = &cx.definitions()[&named.id].definition
    else {
        panic!()
    };
    assert!(fields.is_empty());
    assert!(matches!(
        flattened.as_slice(),
        [TypeRef::Inline(InlineType::Any)]
    ));
    let diagnostic = cx
        .diagnostics()
        .iter()
        .find(|diagnostic| diagnostic.code == "PONS-E101")
        .expect("nullable flatten diagnostic");
    assert_eq!(diagnostic.severity, DiagnosticSeverity::Error);
    assert!(diagnostic.message.contains("nullable"));
    assert!(diagnostic.message.contains("flatten"));

    let (types, diagnostics) = cx.into_parts();
    assert!(document(types, diagnostics).validate().is_err());
}

#[test]
fn flattened_structs_inline_fields_and_maps_keep_their_type() {
    let mut cx = TypeCx::new();
    let fields = FlattenedStruct::schema(&mut cx, Direction::Output);
    let map = FlattenedMap::schema(&mut cx, Direction::Output);
    let TypeRef::Named(fields) = fields else {
        panic!()
    };
    let TypeRef::Named(map) = map else { panic!() };
    let TypeKind::Struct {
        fields: flattened_fields,
        flattened,
        ..
    } = &cx.definitions()[&fields.id].definition
    else {
        panic!()
    };
    let TypeKind::Struct {
        flattened: map_types,
        ..
    } = &cx.definitions()[&map.id].definition
    else {
        panic!()
    };
    assert_eq!(
        flattened_fields
            .iter()
            .map(|field| field.wire.as_str())
            .collect::<Vec<_>>(),
        ["id", "created_by"]
    );
    assert!(flattened.is_empty());
    assert!(matches!(
        map_types.as_slice(),
        [TypeRef::Inline(InlineType::Map { .. })]
    ));
}

#[test]
fn one_sided_codec_with_flatten_only_opaques_the_codec_direction() {
    let mut cx = TypeCx::new();
    let input = OneSidedCodecFlatten::schema(&mut cx, Direction::Input);
    let output = OneSidedCodecFlatten::schema(&mut cx, Direction::Output);

    let (input_fields, input_flattened) = struct_shape(&cx, &input);
    let (output_fields, output_flattened) = struct_shape(&cx, &output);
    assert_eq!(
        input_fields
            .iter()
            .map(|field| field.wire.as_str())
            .collect::<Vec<_>>(),
        ["id", "created_by"]
    );
    assert!(input_flattened.is_empty());
    assert_eq!(
        output_fields
            .iter()
            .map(|field| field.wire.as_str())
            .collect::<Vec<_>>(),
        ["id"]
    );
    assert!(matches!(
        output_flattened,
        [TypeRef::Inline(InlineType::Opaque { .. })]
    ));
    assert!(cx.diagnostics().iter().any(|diagnostic| {
        diagnostic.code == "PONS-W101" && diagnostic.message.contains("flatten")
    }));

    let (types, diagnostics) = cx.into_parts();
    assert!(document(types, diagnostics).validate().is_ok());
}
