mod support;

use pons_core::{Schema, TypeCx};
use pons_ir::{Direction, InlineType, OpaqueWire, TypeKind, TypeRef, VariantPayload};
use support::{
    AuditFields, DirectionalFields, ExternalShape, FlattenedMap, FlattenedStruct, LoginRequest,
    LoginState, SharedBoth, document, error_coded, struct_shape,
};

#[derive(pons_core::Schema)]
#[serde(deny_unknown_fields)]
#[allow(dead_code)]
struct ClosedRequest {
    value: String,
}

#[derive(pons_core::Schema)]
struct Envelope<T> {
    payload: T,
}

#[derive(pons_core::Schema)]
struct CustomEncoding {
    #[serde(with = "custom_wire")]
    value: String,
}

#[derive(pons_core::Schema)]
#[allow(dead_code)]
struct NewtypeCustomEncoding(#[serde(with = "custom_wire")] String);

#[derive(pons_core::Schema)]
#[serde(transparent)]
#[allow(dead_code)]
struct TransparentCustomEncoding(#[serde(with = "custom_wire")] String);

#[derive(pons_core::Schema)]
#[allow(dead_code)]
struct TupleCustomEncoding(#[serde(with = "custom_wire")] String, u64);

#[derive(pons_core::Schema)]
#[allow(dead_code)]
enum CustomEnumEncoding {
    Newtype(#[serde(with = "custom_wire")] String),
    Tuple(#[serde(with = "custom_wire")] String, u64),
    Struct {
        #[serde(with = "custom_wire")]
        value: String,
    },
}

#[derive(pons_core::Schema)]
#[serde(from = "String", into = "u64")]
struct Converted(u64);

#[derive(pons_core::Schema)]
struct Node {
    next: Option<Box<Node>>,
}

#[derive(pons_core::Schema)]
#[allow(dead_code)]
struct OneSidedCodecs {
    #[serde(serialize_with = "encode_value")]
    encoded: Vec<u8>,
    #[serde(deserialize_with = "decode_value")]
    decoded: Vec<u8>,
    #[serde(with = "custom_wire")]
    mirrored: String,
}

#[derive(pons_core::Schema)]
#[serde(transparent)]
#[allow(dead_code)]
struct TransparentMarked {
    #[serde(skip_serializing, skip_deserializing)]
    ignored: (),
    value: String,
}

#[derive(pons_core::Schema)]
#[serde(transparent)]
#[allow(dead_code)]
struct TransparentSplit {
    #[serde(default)]
    preferred: String,
    #[serde(skip_serializing)]
    fallback: u32,
}

#[derive(pons_core::Schema)]
#[serde(default)]
#[allow(dead_code)]
struct TupleDefaults(String, u32);

#[derive(pons_core::Schema)]
#[allow(dead_code)]
struct PlatformSizedIntegers {
    count: usize,
    offset: isize,
}

#[test]
fn derives_object_openness_from_serde_attributes() {
    let mut cx = TypeCx::new();
    let open = LoginRequest::schema(&mut cx, Direction::Input);
    let closed = ClosedRequest::schema(&mut cx, Direction::Input);
    let TypeRef::Named(open) = open else { panic!() };
    let TypeRef::Named(closed) = closed else {
        panic!()
    };
    let TypeKind::Struct { openness, .. } = &cx.definitions()[&open.id].definition else {
        panic!()
    };
    assert_eq!(openness, &pons_ir::ObjectOpenness::PreserveOpaque);
    let TypeKind::Struct { openness, .. } = &cx.definitions()[&closed.id].definition else {
        panic!()
    };
    assert_eq!(openness, &pons_ir::ObjectOpenness::Reject);
}

#[test]
fn custom_serde_functions_produce_a_diagnostic_and_opaque_type() {
    let mut cx = TypeCx::new();
    let reference = CustomEncoding::schema(&mut cx, Direction::Output);
    let TypeRef::Named(named) = reference else {
        panic!()
    };
    let TypeKind::Struct { fields, .. } = &cx.definitions()[&named.id].definition else {
        panic!()
    };
    let TypeRef::Inline(InlineType::Opaque { codec, wire, .. }) = &fields[0].ty else {
        panic!()
    };
    assert!(codec.contains("custom_wire"));
    assert_eq!(*wire, OpaqueWire::Unknown);
    assert_eq!(cx.diagnostics()[0].code, "PONS-W101");
}

#[test]
fn custom_serde_handlers_keep_composite_shapes_opaque() {
    let mut cx = TypeCx::new();
    let newtype = NewtypeCustomEncoding::schema(&mut cx, Direction::Output);
    let transparent = TransparentCustomEncoding::schema(&mut cx, Direction::Output);
    let tuple = TupleCustomEncoding::schema(&mut cx, Direction::Output);
    let enumeration = CustomEnumEncoding::schema(&mut cx, Direction::Output);

    let TypeRef::Named(newtype) = newtype else {
        panic!()
    };
    let TypeRef::Named(transparent) = transparent else {
        panic!()
    };
    let TypeRef::Named(tuple) = tuple else {
        panic!()
    };
    let TypeRef::Named(enumeration) = enumeration else {
        panic!()
    };
    let TypeKind::Newtype { inner: newtype } = &cx.definitions()[&newtype.id].definition else {
        panic!()
    };
    let TypeKind::Newtype { inner: transparent } = &cx.definitions()[&transparent.id].definition
    else {
        panic!()
    };
    let TypeKind::Newtype { inner: tuple } = &cx.definitions()[&tuple.id].definition else {
        panic!()
    };
    let TypeKind::Enum { variants, .. } = &cx.definitions()[&enumeration.id].definition else {
        panic!()
    };

    assert!(matches!(
        newtype,
        TypeRef::Inline(InlineType::Opaque { .. })
    ));
    assert!(matches!(
        transparent,
        TypeRef::Inline(InlineType::Opaque { .. })
    ));
    let TypeRef::Inline(InlineType::Tuple { items }) = tuple else {
        panic!()
    };
    assert!(matches!(
        items[0],
        TypeRef::Inline(InlineType::Opaque { .. })
    ));
    let VariantPayload::Newtype { ty } = &variants[0].payload else {
        panic!()
    };
    assert!(matches!(ty, TypeRef::Inline(InlineType::Opaque { .. })));
    let VariantPayload::Tuple { items } = &variants[1].payload else {
        panic!()
    };
    assert!(matches!(
        items[0],
        TypeRef::Inline(InlineType::Opaque { .. })
    ));
    let VariantPayload::Struct { fields } = &variants[2].payload else {
        panic!()
    };
    assert!(matches!(
        fields[0].ty,
        TypeRef::Inline(InlineType::Opaque { .. })
    ));

    let (types, diagnostics) = cx.into_parts();
    let document = document(types, diagnostics);
    assert!(document.validate().is_ok());
    assert!(document.has_opaque());
}

#[test]
fn serde_conversions_use_the_direction_specific_wire_types() {
    let mut cx = TypeCx::new();
    let input = Converted::schema(&mut cx, Direction::Input);
    let output = Converted::schema(&mut cx, Direction::Output);
    let TypeRef::Named(input) = input else {
        panic!()
    };
    let TypeRef::Named(output) = output else {
        panic!()
    };
    let TypeKind::Newtype { inner: input } = &cx.definitions()[&input.id].definition else {
        panic!()
    };
    let TypeKind::Newtype { inner: output } = &cx.definitions()[&output.id].definition else {
        panic!()
    };
    assert!(matches!(input, TypeRef::Inline(InlineType::String { .. })));
    assert!(matches!(
        output,
        TypeRef::Inline(InlineType::Int {
            bits: 64,
            signed: false,
            ..
        })
    ));
}

#[test]
fn recursive_named_types_resolve_to_the_registered_id() {
    let mut cx = TypeCx::new();
    let reference = Node::schema(&mut cx, Direction::Output);
    let TypeRef::Named(named) = reference else {
        panic!();
    };
    let TypeKind::Struct { fields, .. } = &cx.definitions()[&named.id].definition else {
        panic!();
    };
    let TypeRef::Inline(InlineType::Nullable { inner }) = &fields[0].ty else {
        panic!();
    };
    assert_eq!(
        inner.as_ref(),
        &TypeRef::Named(pons_ir::NamedRef {
            id: named.id,
            variant: None,
        })
    );
    let (types, diagnostics) = cx.into_parts();
    let document = document(types, diagnostics);
    assert!(document.validate().is_ok());
}

#[test]
fn source_types_can_be_used_by_their_normal_rust_api() {
    let value = LoginRequest {
        email: "person@example.test".into(),
        nickname: None,
    };
    assert_eq!(value.email, "person@example.test");
    assert!(value.nickname.is_none());
    let _ = LoginState::Ready;
    let LoginState::Locked { retry_after } = (LoginState::Locked { retry_after: 1 }) else {
        panic!();
    };
    assert_eq!(retry_after, 1);
    let node = Node { next: None };
    assert!(node.next.is_none());
    let _ = ExternalShape::Plain;
    let ExternalShape::InputOnly { value } = (ExternalShape::InputOnly {
        value: String::new(),
    }) else {
        panic!();
    };
    assert!(value.is_empty());
    let Envelope { payload } = Envelope {
        payload: String::new(),
    };
    assert!(payload.is_empty());
    let DirectionalFields {
        value,
        count,
        input_only,
    } = DirectionalFields {
        value: String::new(),
        count: 0,
        input_only: false,
    };
    assert!(value.is_empty());
    assert_eq!(count, 0);
    assert!(!input_only);
    let CustomEncoding { value } = CustomEncoding {
        value: String::new(),
    };
    assert!(value.is_empty());
    let Converted(value) = Converted(4);
    assert_eq!(value, 4);
    let FlattenedStruct {
        id,
        audit: AuditFields { created_by },
    } = FlattenedStruct {
        id: 1,
        audit: AuditFields {
            created_by: String::new(),
        },
    };
    assert_eq!(id, 1);
    assert!(created_by.is_empty());
    let FlattenedMap { attributes } = FlattenedMap {
        attributes: std::collections::HashMap::new(),
    };
    assert!(attributes.is_empty());
}

#[test]
fn generic_types_can_be_registered() {
    let mut cx = TypeCx::new();
    let _ = Envelope::<String>::schema(&mut cx, Direction::Output);
    assert_eq!(cx.definitions().len(), 1);
    let Envelope { payload } = Envelope {
        payload: "value".to_owned(),
    };
    assert_eq!(payload, "value");
}

#[test]
fn schema_implementations_cover_collections_and_tuples() {
    let mut cx = TypeCx::new();
    let sequence = Vec::<u64>::schema(&mut cx, Direction::Input);
    let set = std::collections::BTreeSet::<String>::schema(&mut cx, Direction::Input);
    let tuple = <(bool, String)>::schema(&mut cx, Direction::Input);
    let map = std::collections::HashMap::<String, i32>::schema(&mut cx, Direction::Input);

    assert!(matches!(
        sequence,
        TypeRef::Inline(InlineType::Array { unique: None, .. })
    ));
    assert!(matches!(
        set,
        TypeRef::Inline(InlineType::Array {
            unique: Some(true),
            ..
        })
    ));
    assert!(matches!(tuple, TypeRef::Inline(InlineType::Tuple { items }) if items.len() == 2));
    assert!(matches!(map, TypeRef::Inline(InlineType::Map { .. })));
}

#[test]
fn one_sided_codecs_only_make_their_own_direction_opaque() {
    let mut cx = TypeCx::new();
    let input = OneSidedCodecs::schema(&mut cx, Direction::Input);
    let output = OneSidedCodecs::schema(&mut cx, Direction::Output);
    let both = OneSidedCodecs::schema(&mut cx, Direction::Both);

    let (input, _) = struct_shape(&cx, &input);
    let (output, _) = struct_shape(&cx, &output);
    let (both, _) = struct_shape(&cx, &both);

    assert!(matches!(
        input[0].ty,
        TypeRef::Inline(InlineType::Array { .. })
    ));
    let TypeRef::Inline(InlineType::Opaque { codec, wire, .. }) = &input[1].ty else {
        panic!()
    };
    let labels: Vec<&str> = codec.split(',').collect();
    assert_eq!(labels.len(), 1);
    assert!(labels[0].starts_with("deserialize_with"));
    assert!(labels[0].contains("decode_value"));
    assert_eq!(*wire, OpaqueWire::Unknown);

    let TypeRef::Inline(InlineType::Opaque { codec, .. }) = &output[0].ty else {
        panic!()
    };
    let labels: Vec<&str> = codec.split(',').collect();
    assert_eq!(labels.len(), 1);
    assert!(labels[0].starts_with("serialize_with"));
    assert!(labels[0].contains("encode_value"));
    assert!(matches!(
        output[1].ty,
        TypeRef::Inline(InlineType::Array { .. })
    ));

    for (fields, expected) in [(input, 1), (output, 1), (both, 2)] {
        let TypeRef::Inline(InlineType::Opaque { codec, .. }) = &fields[2].ty else {
            panic!()
        };
        let labels: Vec<&str> = codec.split(',').collect();
        assert_eq!(labels.len(), expected);
        assert!(labels.iter().all(|label| label.contains("custom_wire")));
    }

    assert_eq!(cx.diagnostics().len(), 1);
    assert_eq!(cx.diagnostics()[0].code, "PONS-W101");
    assert!(cx.diagnostics()[0].message.contains("custom serde handler"));

    let (types, diagnostics) = cx.into_parts();
    let document = document(types, diagnostics);
    assert!(document.validate().is_ok());
    assert!(document.has_opaque());
}

#[test]
fn transparent_uses_the_field_serde_marks_instead_of_the_first() {
    let mut cx = TypeCx::new();
    let reference = TransparentMarked::schema(&mut cx, Direction::Both);
    let TypeRef::Named(named) = reference else {
        panic!()
    };
    let TypeKind::Newtype { inner } = &cx.definitions()[&named.id].definition else {
        panic!()
    };
    assert!(matches!(inner, TypeRef::Inline(InlineType::String { .. })));
    assert!(cx.diagnostics().is_empty());
    let (types, diagnostics) = cx.into_parts();
    assert!(document(types, diagnostics).validate().is_ok());
}

#[test]
fn a_transparent_field_that_derives_disagree_on_is_refused() {
    let mut cx = TypeCx::new();
    let _ = TransparentSplit::schema(&mut cx, Direction::Input);
    let diagnostic = error_coded(&cx, "PONS-E101", "preferred");
    assert!(diagnostic.message.contains("Deserialize"));
    let (types, diagnostics) = cx.into_parts();
    assert!(document(types, diagnostics).validate().is_err());
}

#[test]
fn serde_default_on_a_tuple_struct_is_refused() {
    let mut cx = TypeCx::new();
    let _ = TupleDefaults::schema(&mut cx, Direction::Input);
    let diagnostic = error_coded(&cx, "PONS-E101", "tuple struct");
    assert!(diagnostic.message.contains("InlineType::Tuple"));
    let (types, diagnostics) = cx.into_parts();
    assert!(document(types, diagnostics).validate().is_err());
}

#[test]
fn register_as_publishes_one_type_under_a_caller_chosen_id() {
    let mut cx = TypeCx::new();
    let derived = LoginRequest::schema(&mut cx, Direction::Input);
    let carrier = cx.register_as("carrier::login-request", |cx| {
        LoginRequest::schema(cx, Direction::Input)
    });
    let again = cx.register_as("carrier::login-request", |cx| {
        LoginRequest::schema(cx, Direction::Input)
    });
    let TypeRef::Named(derived) = derived else {
        panic!()
    };
    let TypeRef::Named(carrier) = carrier else {
        panic!()
    };
    let TypeRef::Named(again) = again else {
        panic!()
    };

    assert_eq!(
        derived.id,
        format!("{}#input", std::any::type_name::<LoginRequest>()),
        "the derived path keeps keying definitions by type_name"
    );
    assert_eq!(carrier.id, "carrier::login-request");
    assert_eq!(again.id, carrier.id);
    assert_eq!(cx.definitions().len(), 2, "one Rust type, two ids");

    let (mut types, diagnostics) = cx.into_parts();
    assert_eq!(types[&carrier.id].name, types[&derived.id].name);
    let TypeKind::Struct { fields, .. } = &mut types.get_mut(&carrier.id).unwrap().definition
    else {
        panic!()
    };
    let nullable =
        |field: &pons_ir::Field| matches!(field.ty, TypeRef::Inline(InlineType::Nullable { .. }));
    assert!(nullable(&fields[1]), "the carrier starts out a copy");
    for field in fields.iter_mut() {
        if let TypeRef::Inline(InlineType::Nullable { inner }) = &field.ty {
            field.ty = *inner.clone();
        }
    }
    let TypeKind::Struct { fields, .. } = &types[&derived.id].definition else {
        panic!()
    };
    assert!(
        nullable(&fields[1]),
        "a rewrite of one id leaves the other definition alone"
    );
    assert!(document(types, diagnostics).validate().is_ok());
}

#[test]
fn platform_sized_integers_describe_as_64_bit_numbers() {
    let mut cx = TypeCx::new();
    assert!(matches!(
        usize::schema(&mut cx, Direction::Input),
        TypeRef::Inline(InlineType::Int {
            bits: 64,
            signed: false,
            ..
        })
    ));
    assert!(matches!(
        isize::schema(&mut cx, Direction::Input),
        TypeRef::Inline(InlineType::Int {
            bits: 64,
            signed: true,
            ..
        })
    ));

    let reference = PlatformSizedIntegers::schema(&mut cx, Direction::Input);
    let (fields, _) = struct_shape(&cx, &reference);
    assert_eq!(fields.len(), 2);
    assert!(cx.diagnostics().is_empty());
    let (types, diagnostics) = cx.into_parts();
    assert!(document(types, diagnostics).validate().is_ok());
}

#[test]
fn a_direction_pair_shares_one_origin_and_a_both_type_keeps_its_own() {
    let mut cx = TypeCx::new();
    let origin = std::any::type_name::<LoginRequest>().to_owned();
    let pair = [
        (Direction::Input, format!("{origin}#input")),
        (Direction::Output, format!("{origin}#output")),
    ];
    for (direction, id) in pair {
        let TypeRef::Inline(InlineType::Nullable { inner, .. }) =
            Option::<LoginRequest>::schema(&mut cx, direction)
        else {
            panic!("an Option<T> keeps the named type under its nullable wrapper")
        };
        let TypeRef::Named(named) = *inner else {
            panic!()
        };
        assert_eq!(named.id, id, "the side suffix stays on the id");
        assert_eq!(
            cx.definitions()[&named.id].origin.as_deref(),
            Some(origin.as_str()),
            "both halves of the pair name the one origin"
        );
    }

    let both = SharedBoth::schema(&mut cx, Direction::Both);
    let TypeRef::Named(named) = &both else {
        panic!()
    };
    assert_eq!(
        cx.definitions()[&named.id].origin.as_deref(),
        Some(named.id.as_str()),
        "a both-direction type is the whole origin group, so it names itself"
    );

    let _ = Envelope::<String>::schema(&mut cx, Direction::Input);
    let _ = Envelope::<u32>::schema(&mut cx, Direction::Input);
    let instantiations = [
        std::any::type_name::<Envelope<String>>(),
        std::any::type_name::<Envelope<u32>>(),
    ];
    for base in instantiations {
        assert_eq!(
            cx.definitions()[&format!("{base}#input")].origin.as_deref(),
            Some(base),
            "one origin per monomorphised instantiation"
        );
    }

    assert!(cx.diagnostics().is_empty());
    let (types, diagnostics) = cx.into_parts();
    assert!(document(types, diagnostics).validate().is_ok());
}
