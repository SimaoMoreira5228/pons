use std::collections::BTreeSet;

use pons_ir::{
    ByteEncoding, DecimalWire, Direction, Document, EnumRepr, Field, InlineType, NamedRef,
    Presence, TypeKind, TypeRef, validate_value,
};
use serde_json::{Map, Value};

use crate::example::crosses;

pub(crate) struct BodyMutation {
    pub(crate) wire: String,
    pub(crate) value: Value,
    pub(crate) detail: String,
}

pub(crate) fn body(
    document: &Document,
    ty: &TypeRef,
    direction: Direction,
    body: &Value,
) -> Option<BodyMutation> {
    let Value::Object(entries) = body else {
        return None;
    };
    let mut fields = Vec::new();
    collect_fields(document, ty, &mut BTreeSet::new(), &mut fields);
    for field in fields {
        if field.presence != Presence::Required || !crosses(field, direction) {
            continue;
        }
        let Some(value) = invalid_value(document, &field.ty, direction) else {
            continue;
        };
        let mut mutated = Map::clone(entries);
        mutated.insert(field.wire.clone(), value.clone());
        let candidate = Value::Object(mutated);
        if validate_value(document, ty, direction, &candidate).is_err() {
            return Some(BodyMutation {
                wire: field.wire.clone(),
                value,
                detail: format!("body field {}", field.wire),
            });
        }
    }
    None
}

pub(crate) fn invalid_value(
    document: &Document,
    ty: &TypeRef,
    direction: Direction,
) -> Option<Value> {
    let value = wrong(document, ty, direction)?;
    validate_value(document, ty, direction, &value)
        .is_err()
        .then_some(value)
}

pub(crate) fn invalid_text(document: &Document, ty: &TypeRef) -> Option<String> {
    match ty {
        TypeRef::Named(named) => match document.types.get(&named.id)?.definition {
            TypeKind::Newtype { ref inner } => invalid_text(document, inner),
            TypeKind::Enum {
                repr: EnumRepr::String,
                ..
            } => Some("__pons_invalid__".into()),
            _ => None,
        },
        TypeRef::Inline(InlineType::Nullable { inner }) => invalid_text(document, inner),
        TypeRef::Inline(InlineType::Bool) => Some("not-a-boolean".into()),
        TypeRef::Inline(InlineType::Int { .. }) => Some("not-an-integer".into()),
        TypeRef::Inline(InlineType::Float { .. }) => Some("not-a-number".into()),
        TypeRef::Inline(InlineType::Decimal { .. }) => Some("not-a-decimal".into()),
        TypeRef::Inline(InlineType::Bytes { encoding }) if *encoding != ByteEncoding::Array => {
            Some("!!".into())
        }
        TypeRef::Inline(InlineType::Bytes { .. }) => Some("not-bytes".into()),
        _ => None,
    }
}

fn wrong(document: &Document, ty: &TypeRef, direction: Direction) -> Option<Value> {
    match ty {
        TypeRef::Named(named) => named_wrong(document, named, direction),
        TypeRef::Inline(inline) => inline_wrong(document, inline, direction),
    }
}

fn named_wrong(document: &Document, named: &NamedRef, direction: Direction) -> Option<Value> {
    let definition = document.types.get(&named.id)?;
    match &definition.definition {
        TypeKind::Newtype { inner } => wrong(document, inner, direction),
        TypeKind::Struct { .. } => Some(Value::String("__pons_invalid__".into())),
        TypeKind::Enum { repr, variants } => enum_wrong(repr, variants, named.variant.as_deref()),
    }
}

fn inline_wrong(document: &Document, inline: &InlineType, direction: Direction) -> Option<Value> {
    Some(match inline {
        InlineType::Bool => Value::from(0),
        InlineType::Int { .. } => Value::String("not-an-integer".into()),
        InlineType::Float { .. } => Value::String("not-a-number".into()),
        InlineType::Decimal { wire } => Value::String(match wire {
            DecimalWire::String => "not-a-decimal".into(),
            DecimalWire::Number => "not-a-number".into(),
        }),
        InlineType::String { .. } => Value::from(0),
        InlineType::Bytes { encoding } if *encoding == ByteEncoding::Array => {
            Value::Array(vec![Value::from(256)])
        }
        InlineType::Bytes { .. } => Value::String("!!".into()),
        InlineType::Null => Value::Bool(true),
        InlineType::Any | InlineType::Opaque { .. } => return None,
        InlineType::Nullable { inner } => wrong(document, inner, direction)?,
        InlineType::Array { .. } => Value::String("not-an-array".into()),
        InlineType::Map { .. } => Value::Array(Vec::new()),
        InlineType::Tuple { items } if items.is_empty() => Value::String("not-a-tuple".into()),
        InlineType::Tuple { .. } => Value::Array(Vec::new()),
    })
}

fn enum_wrong(
    repr: &EnumRepr,
    variants: &[pons_ir::Variant],
    selected: Option<&str>,
) -> Option<Value> {
    if selected.is_some_and(|selected| !variants.iter().any(|variant| variant.wire == selected)) {
        return None;
    }
    Some(match repr {
        EnumRepr::String | EnumRepr::External => Value::String("__pons_invalid__".into()),
        EnumRepr::Internal { tag } | EnumRepr::Adjacent { tag, .. } => {
            let mut object = Map::new();
            object.insert(tag.clone(), Value::String("__pons_invalid__".into()));
            Value::Object(object)
        }
        EnumRepr::Untagged => return None,
    })
}

fn collect_fields<'a>(
    document: &'a Document,
    ty: &'a TypeRef,
    seen: &mut BTreeSet<String>,
    fields: &mut Vec<&'a Field>,
) {
    let TypeRef::Named(named) = ty else {
        return;
    };
    if !seen.insert(named.id.clone()) {
        return;
    }
    let Some(definition) = document.types.get(&named.id) else {
        return;
    };
    match &definition.definition {
        TypeKind::Newtype { inner } => collect_fields(document, inner, seen, fields),
        TypeKind::Struct {
            fields: own,
            flattened,
            ..
        } => {
            fields.extend(own);
            for flat in flattened {
                collect_fields(document, flat, seen, fields);
            }
        }
        TypeKind::Enum { .. } => {}
    }
    seen.remove(&named.id);
}
