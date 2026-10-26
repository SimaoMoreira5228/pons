mod support;

use pons_core::{Schema, TypeCx};
use pons_ir::{Direction, InlineType, TypeKind, TypeRef, VariantPayload};
use support::{DirectionalFields, LoginRequest, SharedBoth, document, error_coded, struct_shape};

const ALIAS_KEY: &str = "com.pons.alias/v1";

#[derive(pons_core::Schema)]
#[serde(default)]
#[allow(dead_code)]
struct ContainerDefaults {
    value: String,
    count: u8,
    notes: Option<String>,
}

#[derive(pons_core::Schema)]
#[allow(dead_code)]
enum RequiredPayloads {
    Struct {
        value: String,
        notes: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        tag: Option<String>,
    },
}

#[derive(pons_core::Schema)]
#[allow(dead_code)]
struct PresenceMatrix {
    value: String,
    nickname: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tag: Option<String>,
}

#[derive(pons_core::Schema)]
#[allow(dead_code)]
struct DefaultedOptions {
    #[serde(default)]
    nickname: Option<String>,
    email: Option<String>,
}

#[derive(pons_core::Schema)]
#[allow(dead_code)]
struct SkippedOptionBoth {
    #[serde(skip_serializing_if = "Option::is_none")]
    tag: Option<String>,
}

#[derive(pons_core::Schema)]
#[allow(dead_code)]
struct RenamedSides {
    #[serde(rename(serialize = "from_server", deserialize = "to_server"))]
    value: String,
}

#[derive(pons_core::Schema)]
#[allow(dead_code)]
struct AliasedField {
    #[serde(alias = "zeta", alias = "alpha")]
    value: String,
    plain: String,
}

#[derive(pons_core::Schema)]
#[allow(dead_code)]
enum AliasedVariant {
    #[serde(alias = "zulu", alias = "alpha")]
    Primary,
    Secondary,
}

mod user_named_option {
    pub mod bare {
        #[derive(pons_core::Schema)]
        #[allow(dead_code)]
        pub struct Option {
            pub value: String,
        }
    }

    pub mod pair {
        #[derive(pons_core::Schema)]
        #[allow(dead_code)]
        pub struct Option<T, U> {
            pub value: T,
            pub other: U,
        }
    }

    #[derive(pons_core::Schema)]
    #[allow(dead_code)]
    pub struct Shapes {
        pub plain: Option<u8>,
        pub qualified: std::option::Option<u8>,
        pub core: core::option::Option<u8>,
        pub bare: bare::Option,
        pub pair: pair::Option<u8, String>,
    }
}

fn struct_payload_fields<'cx>(cx: &'cx TypeCx, reference: &TypeRef) -> &'cx [pons_ir::Field] {
    let TypeRef::Named(named) = reference else {
        panic!()
    };
    let TypeKind::Enum { variants, .. } = &cx.definitions()[&named.id].definition else {
        panic!()
    };
    let VariantPayload::Struct { fields } = &variants[0].payload else {
        panic!()
    };
    fields
}

#[test]
fn derives_directional_struct_fields_from_serde_attributes() {
    let mut cx = TypeCx::new();
    let reference = LoginRequest::schema(&mut cx, Direction::Input);
    let TypeRef::Named(named) = reference else {
        panic!();
    };
    let TypeKind::Struct { fields, .. } = &cx.definitions()[&named.id].definition else {
        panic!();
    };

    assert_eq!(fields[0].wire, "email");
    assert_eq!(fields[0].presence, pons_ir::Presence::Required);
    assert_eq!(fields[1].wire, "nickname");
    assert_eq!(fields[1].presence, pons_ir::Presence::Optional);
    assert!(matches!(
        fields[1].ty,
        TypeRef::Inline(InlineType::Nullable { .. })
    ));
    let doc = cx.definitions()[&named.id].doc.as_ref().unwrap();
    assert_eq!(
        doc.summary.as_deref(),
        Some("Request payload for an account login.")
    );
    assert_eq!(
        doc.description.as_deref(),
        Some("Request payload for an account login.")
    );
}

#[test]
fn applies_serde_field_renames_defaults_and_direction_skips() {
    let mut cx = TypeCx::new();
    let input = DirectionalFields::schema(&mut cx, Direction::Input);
    let output = DirectionalFields::schema(&mut cx, Direction::Output);
    let TypeRef::Named(input) = input else {
        panic!()
    };
    let TypeRef::Named(output) = output else {
        panic!()
    };
    let TypeKind::Struct {
        fields: input_fields,
        ..
    } = &cx.definitions()[&input.id].definition
    else {
        panic!()
    };
    let TypeKind::Struct {
        fields: output_fields,
        ..
    } = &cx.definitions()[&output.id].definition
    else {
        panic!()
    };
    assert_eq!(
        input_fields
            .iter()
            .map(|field| field.wire.as_str())
            .collect::<Vec<_>>(),
        ["to_server", "count", "input_only"]
    );
    assert_eq!(input_fields[1].presence, pons_ir::Presence::Optional);
    assert_eq!(
        output_fields
            .iter()
            .map(|field| field.wire.as_str())
            .collect::<Vec<_>>(),
        ["from_server", "count"]
    );
    assert!(cx.diagnostics().is_empty());
    assert_eq!(
        input_fields[0].ext[ALIAS_KEY],
        serde_json::json!(["legacy_server"])
    );
    assert!(!output_fields[0].ext.contains_key(ALIAS_KEY));
}

#[test]
fn carries_a_serde_alias_on_a_field_and_on_a_variant() {
    let mut cx = TypeCx::new();
    let field_type = AliasedField::schema(&mut cx, Direction::Input);
    let enum_type = AliasedVariant::schema(&mut cx, Direction::Input);
    let TypeRef::Named(field_type) = field_type else {
        panic!()
    };
    let TypeRef::Named(enum_type) = enum_type else {
        panic!()
    };
    let TypeKind::Struct { fields, .. } = &cx.definitions()[&field_type.id].definition else {
        panic!()
    };
    let TypeKind::Enum { variants, .. } = &cx.definitions()[&enum_type.id].definition else {
        panic!()
    };

    assert_eq!(fields.len(), 2);
    assert_eq!(fields[0].wire, "value");
    assert_eq!(
        fields[0].ext[ALIAS_KEY],
        serde_json::json!(["alpha", "zeta"])
    );
    assert!(!fields[1].ext.contains_key(ALIAS_KEY));
    assert_eq!(variants[0].wire, "Primary");
    assert_eq!(
        variants[0].ext[ALIAS_KEY],
        serde_json::json!(["alpha", "zulu"])
    );
    assert!(!variants[1].ext.contains_key(ALIAS_KEY));
    assert!(cx.diagnostics().is_empty());
    let (types, diagnostics) = cx.into_parts();
    assert!(document(types, diagnostics).validate().is_ok());
}

#[test]
fn a_serde_alias_is_carried_only_on_the_side_that_accepts_it() {
    let mut cx = TypeCx::new();
    let field_type = AliasedField::schema(&mut cx, Direction::Input);
    let output_type = AliasedField::schema(&mut cx, Direction::Output);
    let both_type = AliasedField::schema(&mut cx, Direction::Both);
    let TypeRef::Named(field_type) = field_type else {
        panic!()
    };
    let TypeRef::Named(output_type) = output_type else {
        panic!()
    };
    let TypeRef::Named(both_type) = both_type else {
        panic!()
    };
    let alias_of = |cx: &TypeCx, id: &str| -> Option<serde_json::Value> {
        let TypeKind::Struct { fields, .. } = &cx.definitions()[id].definition else {
            panic!()
        };
        fields[0].ext.get(ALIAS_KEY).cloned()
    };
    assert_eq!(
        alias_of(&cx, &field_type.id),
        Some(serde_json::json!(["alpha", "zeta"]))
    );
    assert_eq!(alias_of(&cx, &output_type.id), None);
    assert_eq!(
        alias_of(&cx, &both_type.id),
        Some(serde_json::json!(["alpha", "zeta"]))
    );
    assert!(cx.diagnostics().is_empty());
}

#[test]
fn container_level_serde_default_makes_every_field_optional_on_input() {
    let mut cx = TypeCx::new();
    let input = ContainerDefaults::schema(&mut cx, Direction::Input);
    let output = ContainerDefaults::schema(&mut cx, Direction::Output);
    let _ = ContainerDefaults::schema(&mut cx, Direction::Both);

    let (input, _) = struct_shape(&cx, &input);
    let (output, _) = struct_shape(&cx, &output);
    assert_eq!(input.len(), 3);
    assert!(
        input
            .iter()
            .all(|field| field.presence == pons_ir::Presence::Optional)
    );
    assert!(
        output
            .iter()
            .all(|field| field.presence == pons_ir::Presence::Required)
    );

    let refusal = error_coded(&cx, "PONS-E103", "count");
    assert!(
        refusal.message.contains("container #[serde(default)]"),
        "the reason is named: {}",
        refusal.message
    );
    assert!(refusal.message.contains("per direction"));
    let (types, diagnostics) = cx.into_parts();
    assert!(document(types, diagnostics).validate().is_err());
}

#[test]
fn enum_payload_presence_stays_direction_specific_without_a_container_default() {
    let mut cx = TypeCx::new();
    let input = RequiredPayloads::schema(&mut cx, Direction::Input);
    let output = RequiredPayloads::schema(&mut cx, Direction::Output);

    for (reference, expected) in [
        (
            input,
            [
                pons_ir::Presence::Required,
                pons_ir::Presence::Optional,
                pons_ir::Presence::Optional,
            ],
        ),
        (
            output,
            [
                pons_ir::Presence::Required,
                pons_ir::Presence::Required,
                pons_ir::Presence::Optional,
            ],
        ),
    ] {
        let fields = struct_payload_fields(&cx, &reference);
        assert_eq!(
            fields
                .iter()
                .map(|field| field.wire.as_str())
                .collect::<Vec<_>>(),
            ["value", "notes", "tag"]
        );
        assert_eq!(
            fields
                .iter()
                .map(|field| field.presence)
                .collect::<Vec<_>>(),
            expected
        );
    }
    assert!(cx.diagnostics().is_empty());

    let (types, diagnostics) = cx.into_parts();
    assert!(document(types, diagnostics).validate().is_ok());
}

#[test]
fn both_refuses_an_option_field_the_two_sides_do_not_share() {
    let mut cx = TypeCx::new();
    let _ = RequiredPayloads::schema(&mut cx, Direction::Both);
    let refusal = error_coded(&cx, "PONS-E103", "notes");
    assert!(
        refusal
            .message
            .contains("optional on input (an Option key may be missing)"),
        "{}",
        refusal.message
    );
    assert!(
        refusal.message.contains("required on output"),
        "{}",
        refusal.message
    );
    assert_eq!(
        cx.diagnostics().len(),
        1,
        "tag is omitted by both sides, so only notes is refused"
    );
    let (types, diagnostics) = cx.into_parts();
    assert!(document(types, diagnostics).validate().is_err());
}

#[test]
fn serde_accepts_a_missing_option_key_but_not_a_missing_plain_key() {
    #[derive(serde::Deserialize, Debug, PartialEq)]
    struct Payload {
        value: String,
        nickname: Option<String>,
    }

    let omitted: Payload = serde_json::from_str(r#"{"value":"a"}"#)
        .expect("a derived visitor accepts a missing Option key");
    assert_eq!(
        omitted,
        Payload {
            value: "a".to_owned(),
            nickname: None,
        }
    );
    let present: Payload = serde_json::from_str(r#"{"value":"a","nickname":"b"}"#).unwrap();
    assert_eq!(present.nickname.as_deref(), Some("b"));
    assert!(
        serde_json::from_str::<Payload>(r#"{"nickname":"b"}"#).is_err(),
        "a non-Option field is still required, so Presence::Required is right for it"
    );
}

#[test]
fn serde_writes_an_option_key_on_output_even_when_the_value_is_none() {
    #[derive(serde::Serialize)]
    struct Payload {
        value: String,
        nickname: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        tag: Option<String>,
    }

    let none = serde_json::to_value(Payload {
        value: "a".to_owned(),
        nickname: None,
        tag: None,
    })
    .unwrap();
    assert_eq!(none["value"], "a");
    assert_eq!(
        none["nickname"],
        serde_json::Value::Null,
        "a Serialize derive writes the key for None, so Presence::Required is right for output"
    );
    assert!(
        none.get("tag").is_none(),
        "skip_serializing_if is the only reason a written key is missing"
    );

    let some = serde_json::to_value(Payload {
        value: "a".to_owned(),
        nickname: Some("b".to_owned()),
        tag: Some("c".to_owned()),
    })
    .unwrap();
    assert_eq!(some["nickname"], "b");
    assert_eq!(some["tag"], "c");
}

#[test]
fn field_presence_is_resolved_by_the_side_the_serde_rule_belongs_to() {
    let mut cx = TypeCx::new();
    let input = PresenceMatrix::schema(&mut cx, Direction::Input);
    let output = PresenceMatrix::schema(&mut cx, Direction::Output);

    for (reference, expected) in [
        (
            input,
            [
                pons_ir::Presence::Required,
                pons_ir::Presence::Optional,
                pons_ir::Presence::Optional,
            ],
        ),
        (
            output,
            [
                pons_ir::Presence::Required,
                pons_ir::Presence::Required,
                pons_ir::Presence::Optional,
            ],
        ),
    ] {
        let (fields, _) = struct_shape(&cx, &reference);
        assert_eq!(
            fields
                .iter()
                .map(|field| field.wire.as_str())
                .collect::<Vec<_>>(),
            ["value", "nickname", "tag"]
        );
        assert_eq!(
            fields
                .iter()
                .map(|field| field.presence)
                .collect::<Vec<_>>(),
            expected
        );
        assert!(
            fields[1..]
                .iter()
                .all(|field| matches!(field.ty, TypeRef::Inline(InlineType::Nullable { .. }))),
            "an omitted key is still a nullable value, not a missing one"
        );
    }
    assert!(cx.diagnostics().is_empty());

    let (types, diagnostics) = cx.into_parts();
    assert!(document(types, diagnostics).validate().is_ok());
}

#[test]
fn an_option_field_is_optional_on_input_and_keeps_its_nullable_type() {
    let mut cx = TypeCx::new();
    let input = DefaultedOptions::schema(&mut cx, Direction::Input);
    let output = DefaultedOptions::schema(&mut cx, Direction::Output);
    let (input, _) = struct_shape(&cx, &input);
    let (output, _) = struct_shape(&cx, &output);

    assert_eq!(input[0].wire, "nickname");
    assert_eq!(input[0].presence, pons_ir::Presence::Optional);
    assert_eq!(input[1].wire, "email");
    assert_eq!(input[1].presence, pons_ir::Presence::Optional);
    assert!(
        output
            .iter()
            .all(|field| field.presence == pons_ir::Presence::Required)
    );
    for direction in [input, output] {
        assert!(
            direction
                .iter()
                .all(|field| matches!(field.ty, TypeRef::Inline(InlineType::Nullable { .. })))
        );
    }
    assert!(cx.diagnostics().is_empty());

    let (types, diagnostics) = cx.into_parts();
    assert!(document(types, diagnostics).validate().is_ok());
}

#[test]
fn both_refuses_a_field_that_serde_defaults() {
    let mut cx = TypeCx::new();
    let _ = DefaultedOptions::schema(&mut cx, Direction::Both);
    let refusal = error_coded(&cx, "PONS-E103", "nickname");
    assert!(
        refusal
            .message
            .contains("#[serde(default)] fills a missing key"),
        "{}",
        refusal.message
    );
    assert_eq!(
        cx.diagnostics().len(),
        2,
        "both Option fields are only optional to a reader, so neither key survives a shared definition"
    );
    let (types, diagnostics) = cx.into_parts();
    assert!(document(types, diagnostics).validate().is_err());
}

#[test]
fn a_shared_type_stays_one_definition_for_both_and_agrees_with_each_side() {
    let mut cx = TypeCx::new();
    let both = SharedBoth::schema(&mut cx, Direction::Both);
    let input = SharedBoth::schema(&mut cx, Direction::Input);
    let output = SharedBoth::schema(&mut cx, Direction::Output);
    let TypeRef::Named(named) = &both else {
        panic!()
    };
    assert_eq!(
        named.id,
        std::any::type_name::<SharedBoth>(),
        "Both keys one definition, without a side suffix"
    );
    assert_eq!(cx.definitions().len(), 3, "one per direction asked for");

    let shape = |reference: &TypeRef| {
        let (fields, _) = struct_shape(&cx, reference);
        fields
            .iter()
            .map(|field| (field.wire.as_str(), field.presence))
            .collect::<Vec<_>>()
    };
    let both_shape = shape(&both);
    assert_eq!(
        both_shape,
        [
            ("value", pons_ir::Presence::Required),
            ("count", pons_ir::Presence::Required)
        ]
    );
    assert_eq!(shape(&input), both_shape);
    assert_eq!(shape(&output), both_shape);
    assert!(cx.diagnostics().is_empty());

    let (types, diagnostics) = cx.into_parts();
    assert!(document(types, diagnostics).validate().is_ok());
}

#[test]
fn an_option_the_two_sides_omit_alike_is_still_one_definition_for_both() {
    let mut cx = TypeCx::new();
    let both = SkippedOptionBoth::schema(&mut cx, Direction::Both);
    let (fields, _) = struct_shape(&cx, &both);
    assert_eq!(fields[0].wire, "tag");
    assert_eq!(
        fields[0].presence,
        pons_ir::Presence::Optional,
        "an Option key may be missing on input and skip_serializing_if may omit it on output, so Optional is true of both"
    );
    assert!(cx.diagnostics().is_empty());
    let (types, diagnostics) = cx.into_parts();
    assert!(document(types, diagnostics).validate().is_ok());
}

#[test]
fn both_refuses_a_field_named_differently_by_side() {
    let mut cx = TypeCx::new();
    let _ = RenamedSides::schema(&mut cx, Direction::Both);
    let refusal = error_coded(&cx, "PONS-E103", "value");
    assert!(refusal.message.contains("to_server"), "{}", refusal.message);
    assert!(
        refusal.message.contains("from_server"),
        "{}",
        refusal.message
    );
    let (types, diagnostics) = cx.into_parts();
    assert!(document(types, diagnostics).validate().is_err());
}

#[test]
fn only_a_unary_option_type_path_is_optional_on_input() {
    let mut cx = TypeCx::new();
    let reference = user_named_option::Shapes::schema(&mut cx, Direction::Input);
    let (fields, _) = struct_shape(&cx, &reference);

    assert_eq!(fields.len(), 5);
    for (index, wire, presence) in [
        (0, "plain", pons_ir::Presence::Optional),
        (1, "qualified", pons_ir::Presence::Optional),
        (2, "core", pons_ir::Presence::Optional),
        (3, "bare", pons_ir::Presence::Required),
        (4, "pair", pons_ir::Presence::Required),
    ] {
        assert_eq!(fields[index].wire, wire);
        assert_eq!(fields[index].presence, presence, "presence of {wire}");
    }
    for field in &fields[..3] {
        assert!(
            matches!(field.ty, TypeRef::Inline(InlineType::Nullable { .. })),
            "{} keeps the nullable type",
            field.wire
        );
    }
    assert!(cx.diagnostics().is_empty());

    let (types, diagnostics) = cx.into_parts();
    assert!(document(types, diagnostics).validate().is_ok());
}
