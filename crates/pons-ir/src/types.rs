use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::operation::Origin;

pub type Name = Vec<String>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct TypeDef {
    pub name: Name,
    pub direction: Direction,
    #[serde(flatten)]
    pub definition: TypeKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub doc: Option<Doc>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<Diagnostic>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub ext: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TypeKind {
    Struct {
        #[serde(default)]
        fields: Vec<Field>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        flattened: Vec<TypeRef>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        additional: Option<Box<TypeRef>>,
    },
    Enum {
        repr: EnumRepr,
        variants: Vec<Variant>,
    },
    Newtype {
        inner: TypeRef,
    },
}

impl TypeKind {
    pub(crate) fn visit_refs(&self, visit: &mut impl FnMut(&TypeRef)) {
        match self {
            Self::Struct {
                fields,
                flattened,
                additional,
            } => {
                fields.iter().for_each(|field| field.ty.visit_refs(visit));
                flattened.iter().for_each(|ty| ty.visit_refs(visit));
                if let Some(ty) = additional {
                    ty.visit_refs(visit);
                }
            }
            Self::Enum { variants, .. } => variants
                .iter()
                .for_each(|variant| variant.payload.visit_refs(visit)),
            Self::Newtype { inner } => inner.visit_refs(visit),
        }
    }

    pub(crate) fn canonicalize(&mut self) {
        match self {
            Self::Struct { fields, .. } => {
                fields.sort_by(|left, right| left.wire.cmp(&right.wire));
            }
            Self::Enum { variants, .. } => {
                variants.sort_by(|left, right| left.wire.cmp(&right.wire));
                variants
                    .iter_mut()
                    .for_each(|variant| variant.payload.canonicalize());
            }
            Self::Newtype { .. } => {}
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EnumRepr {
    String,
    External,
    Internal { tag: String },
    Adjacent { tag: String, content: String },
    Untagged,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Variant {
    pub wire: String,
    pub ident: Name,
    pub payload: VariantPayload,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub doc: Option<Doc>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<Diagnostic>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub ext: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum VariantPayload {
    Unit,
    Newtype { ty: TypeRef },
    Tuple { items: Vec<TypeRef> },
    Struct { fields: Vec<Field> },
}

impl VariantPayload {
    pub(crate) fn visit_refs(&self, visit: &mut impl FnMut(&TypeRef)) {
        match self {
            Self::Unit => {}
            Self::Newtype { ty } => ty.visit_refs(visit),
            Self::Tuple { items } => items.iter().for_each(|ty| ty.visit_refs(visit)),
            Self::Struct { fields } => fields.iter().for_each(|field| field.ty.visit_refs(visit)),
        }
    }

    pub(crate) fn canonicalize(&mut self) {
        if let Self::Struct { fields } = self {
            fields.sort_by(|left, right| left.wire.cmp(&right.wire));
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Field {
    pub wire: String,
    pub ident: Name,
    pub ty: TypeRef,
    pub presence: Presence,
    #[schemars(with = "Value")]
    #[serde(default, skip_serializing_if = "FieldDefault::is_unspecified")]
    pub default: FieldDefault,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub access: Option<Access>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub doc: Option<Doc>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<Diagnostic>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub ext: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub enum FieldDefault {
    #[default]
    Unspecified,
    Value(Value),
}

impl FieldDefault {
    fn is_unspecified(&self) -> bool {
        matches!(self, Self::Unspecified)
    }
}

impl Serialize for FieldDefault {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match self {
            Self::Unspecified => serializer.serialize_unit(),
            Self::Value(value) => value.serialize(serializer),
        }
    }
}

impl<'de> Deserialize<'de> for FieldDefault {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        Value::deserialize(deserializer).map(Self::Value)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Presence {
    Required,
    Optional,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Access {
    ReadWrite,
    ReadOnly,
    WriteOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    Input,
    Output,
    Both,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(untagged)]
pub enum TypeRef {
    Named(NamedRef),
    Inline(InlineType),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct NamedRef {
    #[serde(rename = "ref")]
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub variant: Option<String>,
}

impl TypeRef {
    pub(crate) fn visit_refs(&self, visit: &mut impl FnMut(&TypeRef)) {
        visit(self);
        match self {
            Self::Named(_) => {}
            Self::Inline(InlineType::Nullable { inner }) => inner.visit_refs(visit),
            Self::Inline(InlineType::Array { item, .. }) => item.visit_refs(visit),
            Self::Inline(InlineType::Map { key, value }) => {
                key.visit_refs(visit);
                value.visit_refs(visit);
            }
            Self::Inline(InlineType::Tuple { items }) => {
                items.iter().for_each(|item| item.visit_refs(visit));
            }
            Self::Inline(_) => {}
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum InlineType {
    Bool,
    Int {
        bits: u16,
        signed: bool,
        wire: IntegerWire,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        minimum: Option<Value>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        maximum: Option<Value>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        multiple_of: Option<Value>,
    },
    Float {
        bits: u8,
    },
    Decimal {
        wire: DecimalWire,
    },
    String {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        format: Option<FieldFormat>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        min_length: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max_length: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pattern: Option<String>,
    },
    Bytes {
        encoding: ByteEncoding,
    },
    Null,
    Any,
    Nullable {
        inner: Box<TypeRef>,
    },
    Array {
        item: Box<TypeRef>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        unique: Option<bool>,
    },
    Map {
        key: Box<TypeRef>,
        value: Box<TypeRef>,
    },
    Tuple {
        items: Vec<TypeRef>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum IntegerWire {
    Number,
    String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DecimalWire {
    Number,
    String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ByteEncoding {
    Base64,
    Base64url,
    Hex,
    Array,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum FieldFormat {
    Uuid,
    DateTime,
    Date,
    Time,
    Duration,
    Uri,
    Email,
    Ipv4,
    Ipv6,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Doc {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deprecated: Option<bool>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Diagnostic {
    pub severity: DiagnosticSeverity,
    pub code: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<Origin>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub ext: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticSeverity {
    Error,
    Warning,
    Info,
}
