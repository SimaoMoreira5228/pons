use pons_core::{Schema, TypeCx};
use pons_ir::{
    DiagnosticSeverity, Direction, Document, EnumRepr, IR_VERSION, Info, InlineType, OpaqueWire,
    TypeKind, TypeRef, VariantPayload,
};
use std::collections::BTreeMap;

const ALIAS_KEY: &str = "com.pons.alias/v1";
const OPEN_VARIANT_KEY: &str = "com.pons.open_variant/v1";

#[derive(pons_core::Schema)]
#[serde(deny_unknown_fields)]
#[allow(dead_code)]
struct ClosedRequest {
    value: String,
}

#[derive(pons_core::Schema)]
#[doc = "Request payload for an account login."]
struct LoginRequest {
    #[doc = "Account email address."]
    email: String,
    nickname: Option<String>,
}

#[derive(pons_core::Schema)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum LoginState {
    Ready,
    Locked { retry_after: u32 },
}

#[derive(pons_core::Schema)]
enum ExternalShape {
    Plain,
    #[serde(skip_serializing)]
    InputOnly {
        value: String,
    },
}

#[derive(pons_core::Schema)]
struct Envelope<T> {
    payload: T,
}

#[derive(pons_core::Schema)]
struct DirectionalFields {
    #[serde(
        rename(serialize = "from_server", deserialize = "to_server"),
        alias = "legacy_server"
    )]
    value: String,
    #[serde(default)]
    count: u8,
    #[serde(skip_serializing)]
    input_only: bool,
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
#[serde(from = "String", into = "u64")]
struct Converted(u64);

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
struct SharedBoth {
    value: String,
    count: u8,
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
struct OneSidedCodecs {
    #[serde(serialize_with = "encode_value")]
    encoded: Vec<u8>,
    #[serde(deserialize_with = "decode_value")]
    decoded: Vec<u8>,
    #[serde(with = "custom_wire")]
    mirrored: String,
}

#[derive(pons_core::Schema)]
#[allow(dead_code)]
struct OneSidedCodecFlatten {
    id: u64,
    #[serde(flatten, serialize_with = "encode_value")]
    audit: AuditFields,
}

#[derive(pons_core::Schema)]
struct AuditFields {
    created_by: String,
}

#[derive(pons_core::Schema)]
struct FlattenedStruct {
    id: u64,
    #[serde(flatten)]
    audit: AuditFields,
}

#[derive(pons_core::Schema)]
#[allow(dead_code)]
struct NullableFlatten {
    #[serde(flatten)]
    inner: Option<AuditFields>,
}

#[derive(pons_core::Schema)]
struct FlattenedMap {
    #[serde(flatten)]
    attributes: std::collections::HashMap<String, String>,
}

#[derive(pons_core::Schema)]
#[allow(dead_code)]
struct FlattenedNestedMapStruct {
    id: u64,
    #[serde(flatten)]
    audit: AuditFieldsWithMap,
}

#[derive(pons_core::Schema)]
struct Node {
    next: Option<Box<Node>>,
}

#[derive(pons_core::Schema, pons_core::ApiError)]
#[serde(tag = "code", rename_all = "snake_case")]
enum LoginError {
    #[pons(status = 401)]
    InvalidCredentials,
    #[pons(status = 423, retryable = true)]
    Locked { retry_after: u32 },
}

#[derive(pons_core::Schema, pons_core::ApiError)]
enum ContentError {
    #[pons(status = 400, content = "text")]
    Text,
    #[pons(status = 400, content = "empty")]
    Empty,
}

#[derive(pons_core::Schema, pons_core::ApiError)]
enum StatusError {
    #[pons(status = "4xx")]
    Client,
    #[pons(status = "default")]
    Fallback,
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

#[derive(pons_core::Schema)]
#[allow(dead_code)]
enum OpenVariant {
    Known,
    #[serde(other)]
    Unknown,
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
#[serde(deny_unknown_fields)]
#[allow(dead_code)]
enum ClosedEnum {
    Closed { value: u32 },
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

fn document(
    types: BTreeMap<String, pons_ir::TypeDef>,
    diagnostics: Vec<pons_ir::Diagnostic>,
) -> Document {
    Document {
        ir_version: IR_VERSION.to_owned(),
        info: Info {
            title: "Schema test".into(),
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
fn api_errors_register_typed_variant_responses() {
    let mut cx = TypeCx::new();
    let errors = <LoginError as pons_core::ErrorSpec>::errors(&mut cx);
    assert_eq!(errors.len(), 2);
    assert_eq!(errors[0].status, pons_ir::ErrorStatus::Exact(401));
    assert_eq!(errors[0].name, ["invalid", "credentials"]);
    assert_eq!(errors[1].status, pons_ir::ErrorStatus::Exact(423));
    assert_eq!(errors[1].name, ["locked"]);
    assert_eq!(errors[1].retryable, Some(true));
    let TypeRef::Named(body) = errors[1].body.as_ref().unwrap() else {
        panic!()
    };
    assert_eq!(body.variant.as_deref(), Some("locked"));
    assert!(matches!(
        &cx.definitions()[&body.id].definition,
        TypeKind::Enum { variants, .. } if variants.len() == 2
    ));
    let _ = LoginError::InvalidCredentials;
    let LoginError::Locked { retry_after } = (LoginError::Locked { retry_after: 1 }) else {
        panic!()
    };
    assert_eq!(retry_after, 1);
}

#[test]
fn api_error_honors_declared_content() {
    let mut cx = TypeCx::new();
    let errors = <ContentError as pons_core::ErrorSpec>::errors(&mut cx);
    assert_eq!(errors[0].content, pons_ir::ErrorContent::Text);
    assert!(errors[0].body.is_some());
    assert_eq!(errors[1].content, pons_ir::ErrorContent::Empty);
    assert!(errors[1].body.is_none());
    let _ = ContentError::Text;
    let _ = ContentError::Empty;
}

#[test]
fn api_error_accepts_status_classes_and_default() {
    let mut cx = TypeCx::new();
    let errors = <StatusError as pons_core::ErrorSpec>::errors(&mut cx);
    assert_eq!(errors[0].status, pons_ir::ErrorStatus::Class(4));
    assert_eq!(errors[1].status, pons_ir::ErrorStatus::Default);
    let _ = StatusError::Client;
    let _ = StatusError::Fallback;
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

fn struct_shape<'cx>(
    cx: &'cx TypeCx,
    reference: &TypeRef,
) -> (&'cx [pons_ir::Field], &'cx [TypeRef]) {
    let TypeRef::Named(named) = reference else {
        panic!()
    };
    let TypeKind::Struct {
        fields, flattened, ..
    } = &cx.definitions()[&named.id].definition
    else {
        panic!()
    };
    (fields, flattened)
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

fn error_coded<'a>(cx: &'a TypeCx, code: &str, needle: &str) -> &'a pons_ir::Diagnostic {
    cx.diagnostics()
        .iter()
        .find(|diagnostic| {
            diagnostic.code == code
                && diagnostic.severity == DiagnosticSeverity::Error
                && diagnostic.message.contains(needle)
        })
        .unwrap_or_else(|| panic!("{code} mentioning {needle} in {:?}", cx.diagnostics()))
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
fn deny_unknown_fields_on_an_enum_is_refused() {
    let mut cx = TypeCx::new();
    let _ = ClosedEnum::schema(&mut cx, Direction::Input);
    let diagnostic = error_coded(&cx, "PONS-E101", "deny_unknown_fields");
    assert!(diagnostic.message.contains("VariantPayload::Struct"));
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
