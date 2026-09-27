use pons_core::{Schema, TypeCx};
use pons_ir::{
    Direction, Document, EnumRepr, IR_VERSION, Info, InlineType, TypeKind, TypeRef, VariantPayload,
};
use std::collections::BTreeMap;

#[derive(pons_core::Schema)]
#[serde(deny_unknown_fields)]
#[allow(dead_code)]
struct ClosedRequest {
    value: String,
}

#[derive(pons_core::Schema)]
/// Request payload for an account login.
struct LoginRequest {
    /// Account email address.
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
#[serde(from = "String", into = "u64")]
struct Converted(u64);

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
struct FlattenedMap {
    #[serde(flatten)]
    attributes: std::collections::HashMap<String, String>,
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
    assert_eq!(cx.diagnostics().len(), 1);
    assert!(cx.diagnostics()[0].message.contains("legacy_server"));
}

#[test]
fn custom_serde_functions_produce_a_diagnostic_and_any_type() {
    let mut cx = TypeCx::new();
    let reference = CustomEncoding::schema(&mut cx, Direction::Output);
    let TypeRef::Named(named) = reference else {
        panic!()
    };
    let TypeKind::Struct { fields, .. } = &cx.definitions()[&named.id].definition else {
        panic!()
    };
    assert!(matches!(fields[0].ty, TypeRef::Inline(InlineType::Any)));
    assert_eq!(cx.diagnostics()[0].code, "PONS-W101");
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
    let document = Document {
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
    };
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
