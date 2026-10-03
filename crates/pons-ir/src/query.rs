use std::collections::BTreeSet;

use crate::{
    Access, ByteEncoding, Document, EnumRepr, Field, InlineType, ObjectOpenness, Presence,
    TypeKind, TypeRef,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QueryRejection {
    NotAnObject,
    NarrowedVariant(String),
    Recursive(String),
    UnknownType(String),
    OpenObject(String),
    NotAStruct(String),
    NonScalarField,
    RequiredNullable(String),
    ReadOnly(String),
    DuplicateField(String),
}

impl QueryRejection {
    pub fn message(&self) -> String {
        match self {
            Self::NotAnObject => {
                "query parameters must be structs or transparent newtypes over structs".into()
            }
            Self::NarrowedVariant(id) => format!("query type {id} cannot narrow an enum variant"),
            Self::Recursive(id) => format!("recursive transparent query type {id}"),
            Self::UnknownType(id) => format!("unknown query type {id}"),
            Self::OpenObject(id) => format!("query type {id} accepts additional keys"),
            Self::NotAStruct(id) => format!("query type {id} is not a struct"),
            Self::NonScalarField => {
                "query fields must use scalar types with an unambiguous encoding".into()
            }
            Self::RequiredNullable(wire) => format!(
                "query field {wire:?} is required and nullable; a query string has no encoding for a present null, so make the field optional or drop the nullable wrapper"
            ),
            Self::ReadOnly(wire) => format!(
                "query field {wire:?} is read_only, but an OpenAPI query parameter has no read/write direction: the client drops the field on encode and every server built from the document waits for a parameter the client cannot send"
            ),
            Self::DuplicateField(wire) => {
                format!("query field {wire:?} is declared more than once")
            }
        }
    }
}

pub fn query_fields<'a>(
    document: &'a Document,
    query: &'a TypeRef,
) -> Result<Vec<&'a Field>, QueryRejection> {
    let mut fields = Vec::new();
    let mut visiting = Vec::new();
    collect(document, query, &mut visiting, &mut fields)?;

    let mut seen = BTreeSet::new();
    for field in &fields {
        if !seen.insert(field.wire.as_str()) {
            return Err(QueryRejection::DuplicateField(field.wire.clone()));
        }
    }
    Ok(fields)
}

fn collect<'a>(
    document: &'a Document,
    query: &'a TypeRef,
    visiting: &mut Vec<&'a str>,
    fields: &mut Vec<&'a Field>,
) -> Result<(), QueryRejection> {
    let TypeRef::Named(named) = query else {
        return Err(QueryRejection::NotAnObject);
    };
    if named.variant.is_some() {
        return Err(QueryRejection::NarrowedVariant(named.id.clone()));
    }
    if visiting.contains(&named.id.as_str()) {
        return Err(QueryRejection::Recursive(named.id.clone()));
    }
    let definition = document
        .types
        .get(&named.id)
        .ok_or_else(|| QueryRejection::UnknownType(named.id.clone()))?;
    visiting.push(&named.id);
    let result = match &definition.definition {
        TypeKind::Newtype { inner } => collect(document, inner, visiting, fields),
        TypeKind::Struct {
            fields: own,
            flattened,
            openness,
        } => {
            if !matches!(openness, ObjectOpenness::Reject) {
                Err(QueryRejection::OpenObject(named.id.clone()))
            } else {
                own.iter()
                    .try_for_each(|field| {
                        is_query_field(document, field, &mut Vec::new())?;
                        fields.push(field);
                        Ok(())
                    })
                    .and_then(|()| {
                        flattened
                            .iter()
                            .try_for_each(|ty| collect(document, ty, visiting, fields))
                    })
            }
        }
        TypeKind::Enum { .. } => Err(QueryRejection::NotAStruct(named.id.clone())),
    };
    visiting.pop();
    result
}

pub(crate) fn encodes_nullable_as_absence(document: &Document, field: &Field) -> bool {
    field.presence == Presence::Optional
        || !document.is_nullable_type(&field.ty, &mut BTreeSet::new())
}

fn is_query_field(
    document: &Document,
    field: &Field,
    visited: &mut Vec<String>,
) -> Result<(), QueryRejection> {
    if field.access == Some(Access::ReadOnly) {
        return Err(QueryRejection::ReadOnly(field.wire.clone()));
    }
    match &field.ty {
        TypeRef::Inline(InlineType::Nullable { inner })
            if !encodes_nullable_as_absence(document, field) =>
        {
            Err(QueryRejection::RequiredNullable(field.wire.clone()))
        }
        TypeRef::Inline(InlineType::Nullable { inner }) => is_query_value(document, inner, visited),
        ty => is_query_value(document, ty, visited),
    }
}

fn is_query_value(
    document: &Document,
    ty: &TypeRef,
    visited: &mut Vec<String>,
) -> Result<(), QueryRejection> {
    let supported = match ty {
        TypeRef::Inline(
            InlineType::Bool
            | InlineType::Int { .. }
            | InlineType::Float { .. }
            | InlineType::Decimal { .. }
            | InlineType::String { .. },
        ) => true,
        TypeRef::Inline(InlineType::Bytes {
            encoding: ByteEncoding::Base64 | ByteEncoding::Base64url | ByteEncoding::Hex,
        }) => true,
        TypeRef::Named(named) if visited.contains(&named.id) => false,
        TypeRef::Named(named) => {
            let definition = document
                .types
                .get(&named.id)
                .ok_or_else(|| QueryRejection::UnknownType(named.id.clone()))?;
            match &definition.definition {
                TypeKind::Enum { repr, .. } => matches!(repr, EnumRepr::String),
                TypeKind::Newtype { inner } => {
                    visited.push(named.id.clone());
                    let result = is_query_value(document, inner, visited);
                    visited.pop();
                    result.is_ok()
                }
                TypeKind::Struct { .. } => false,
            }
        }
        TypeRef::Inline(_) => false,
    };
    if supported {
        Ok(())
    } else {
        Err(QueryRejection::NonScalarField)
    }
}

pub(crate) fn is_scalar_field(
    document: &Document,
    ty: &TypeRef,
    visited: &mut Vec<String>,
) -> bool {
    is_query_value(document, ty, visited).is_ok()
}
