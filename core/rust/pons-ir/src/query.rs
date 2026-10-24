use std::collections::BTreeSet;

use crate::{
    Access, ByteEncoding, Document, EnumRepr, Field, FieldDefault, InlineType, ObjectOpenness,
    Presence, TypeKind, TypeRef,
};

pub(crate) struct Carrier {
    kind: &'static str,

    field: &'static str,

    subject: &'static str,

    record: &'static str,

    scalar: &'static str,

    nullable: &'static str,

    single: bool,

    array: bool,
}

pub(crate) const QUERY: Carrier = Carrier {
    kind: "query",
    field: "query field",
    subject: "an OpenAPI query parameter",
    record: "query parameters must be structs or transparent newtypes over structs",
    scalar: "query fields must use scalar types with an unambiguous encoding, and query field {wire:?} is not one",
    nullable: "query field {wire:?} is required and nullable; a query string has no encoding for a present null, so make the field optional or drop the nullable wrapper",
    single: false,
    array: false,
};

pub(crate) const FORM: Carrier = Carrier {
    kind: "form body",
    field: "form body field",
    subject: "a urlencoded form field",
    record: "a form body must be a struct or a transparent newtype over a struct, because a urlencoded form writes one key per field",
    scalar: "form body fields must use scalar types, or one array of them, with an unambiguous encoding, and form body field {wire:?} is not one",
    nullable: "form body field {wire:?} is required and nullable; a urlencoded form field has no encoding for a present null, so make the field optional or drop the nullable wrapper",
    single: false,
    array: true,
};

pub(crate) const PATH: Carrier = Carrier {
    kind: "path parameter",
    field: "path parameter",
    subject: "a path segment",
    record: "a path parameter must be a struct or a transparent newtype over a struct",
    scalar: "path parameters must use scalar types with an unambiguous encoding, and path parameter {wire:?} is not one",
    nullable: "required path parameter {wire} cannot be nullable, because a path segment is always there and the path has no encoding for a present null",
    single: true,
    array: false,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QueryRejection {
    NotAnObject,
    NarrowedVariant(String),
    Recursive(String),
    UnknownType(String),
    OpenObject(String),
    NotAStruct(String),
    NonScalarField(String),
    RequiredNullable(String),
    ReadOnly(String),
    DuplicateField(String),
}

impl QueryRejection {
    pub fn message(&self) -> String {
        self.in_carrier(&QUERY)
    }

    pub(crate) fn in_carrier(&self, carrier: &Carrier) -> String {
        match self {
            Self::NotAnObject => carrier.record.to_owned(),
            Self::NarrowedVariant(id) => {
                format!(
                    "{kind} type {id:?} cannot narrow an enum variant",
                    kind = carrier.kind
                )
            }
            Self::Recursive(id) => {
                format!(
                    "recursive transparent {kind} type {id:?}",
                    kind = carrier.kind
                )
            }
            Self::UnknownType(id) => format!("unknown {kind} type {id:?}", kind = carrier.kind),
            Self::OpenObject(id) => {
                format!(
                    "{kind} type {id:?} accepts additional keys",
                    kind = carrier.kind
                )
            }
            Self::NotAStruct(id) => {
                format!("{kind} type {id:?} is not a struct", kind = carrier.kind)
            }
            Self::NonScalarField(wire) => worded(carrier.scalar, wire),
            Self::RequiredNullable(wire) => worded(carrier.nullable, wire),
            Self::ReadOnly(wire) => dropped_key(
                carrier.field,
                wire,
                Access::ReadOnly,
                &format!("{} has no read/write direction", carrier.subject),
            ),
            Self::DuplicateField(wire) => {
                format!(
                    "{field} {wire:?} is declared more than once",
                    field = carrier.field
                )
            }
        }
    }

    fn at(self, wire: &str) -> Self {
        match self {
            Self::NonScalarField(named) if named.is_empty() => {
                Self::NonScalarField(wire.to_owned())
            }
            other => other,
        }
    }
}

fn worded(template: &str, wire: &str) -> String {
    template
        .replace("{wire:?}", &format!("{wire:?}"))
        .replace("{wire}", wire)
}

pub(crate) fn waits_for_a_dropped_key(field: &Field, access: Access) -> bool {
    field.presence == Presence::Required
        && matches!(field.default, FieldDefault::Unspecified)
        && field.access == Some(access)
}

pub(crate) fn dropped_key(field: &str, wire: &str, access: Access, subject: &str) -> String {
    match access {
        Access::ReadOnly => format!(
            "{field} {wire:?} is read_only, but {subject}: the client drops the field on encode and every server built from the document waits for a key the client cannot send"
        ),
        _ => format!(
            "{field} {wire:?} is write_only, but {subject}: the server drops the field on decode and every client built from the document waits for a field the server cannot send"
        ),
    }
}

pub fn query_fields<'a>(
    document: &'a Document,
    query: &'a TypeRef,
) -> Result<Vec<&'a Field>, QueryRejection> {
    flat_fields(document, &QUERY, "", query)
}

pub(crate) fn form_body_fields<'a>(
    document: &'a Document,
    body: &'a TypeRef,
) -> Result<Vec<&'a Field>, QueryRejection> {
    flat_fields(document, &FORM, "", body)
}

pub(crate) fn path_fields<'a>(
    document: &'a Document,
    param: &str,
    ty: &'a TypeRef,
) -> Result<Vec<&'a Field>, QueryRejection> {
    flat_fields(document, &PATH, param, ty)
}

fn flat_fields<'a>(
    document: &'a Document,
    carrier: &Carrier,

    segment: &str,
    ty: &'a TypeRef,
) -> Result<Vec<&'a Field>, QueryRejection> {
    if carrier.single && !carries_members(document, ty, &mut BTreeSet::new()) {
        if document.is_nullable_type(ty, &mut BTreeSet::new()) {
            return Err(QueryRejection::RequiredNullable(segment.to_owned()));
        }
        is_flat_value(document, carrier, ty, carrier.array, &mut Vec::new())
            .map_err(|refusal| refusal.at(segment))?;
        return Ok(Vec::new());
    }

    let mut fields = Vec::new();
    collect(document, carrier, ty, &mut Vec::new(), &mut fields)?;
    let mut seen = BTreeSet::new();
    for field in &fields {
        if !seen.insert(field.wire.as_str()) {
            return Err(QueryRejection::DuplicateField(field.wire.clone()));
        }
    }
    Ok(fields)
}

fn carries_members(document: &Document, ty: &TypeRef, seen: &mut BTreeSet<String>) -> bool {
    let TypeRef::Named(named) = ty else {
        return false;
    };
    if named.variant.is_some() || !seen.insert(named.id.clone()) {
        return false;
    }
    let carries = match document.types.get(&named.id).map(|found| &found.definition) {
        Some(TypeKind::Struct { .. }) => true,
        Some(TypeKind::Newtype { inner }) => carries_members(document, inner, seen),
        _ => false,
    };
    seen.remove(&named.id);
    carries
}

fn collect<'a>(
    document: &'a Document,
    carrier: &Carrier,
    ty: &'a TypeRef,
    visiting: &mut Vec<&'a str>,
    fields: &mut Vec<&'a Field>,
) -> Result<(), QueryRejection> {
    let TypeRef::Named(named) = ty else {
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
        TypeKind::Newtype { inner } => collect(document, carrier, inner, visiting, fields),
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
                        is_flat_field(document, carrier, field)?;
                        fields.push(field);
                        Ok(())
                    })
                    .and_then(|()| {
                        flattened
                            .iter()
                            .try_for_each(|ty| collect(document, carrier, ty, visiting, fields))
                    })
            }
        }
        TypeKind::Enum { .. } => Err(QueryRejection::NotAStruct(named.id.clone())),
    };
    visiting.pop();
    result
}

fn is_flat_field(
    document: &Document,
    carrier: &Carrier,
    field: &Field,
) -> Result<(), QueryRejection> {
    if waits_for_a_dropped_key(field, Access::ReadOnly) {
        return Err(QueryRejection::ReadOnly(field.wire.clone()));
    }
    if !encodes_nullable_as_absence(document, field) {
        return Err(QueryRejection::RequiredNullable(field.wire.clone()));
    }
    is_flat_value(document, carrier, &field.ty, carrier.array, &mut Vec::new())
        .map_err(|refusal| refusal.at(&field.wire))
}

fn encodes_nullable_as_absence(document: &Document, field: &Field) -> bool {
    field.presence == Presence::Optional
        || !document.is_nullable_type(&field.ty, &mut BTreeSet::new())
}

fn is_flat_value(
    document: &Document,
    carrier: &Carrier,
    ty: &TypeRef,
    array: bool,
    visited: &mut Vec<String>,
) -> Result<(), QueryRejection> {
    match ty {
        TypeRef::Inline(InlineType::Nullable { inner }) => {
            is_flat_value(document, carrier, inner, array, visited)
        }
        TypeRef::Inline(InlineType::Array { item, .. }) if array => {
            is_flat_value(document, carrier, item, false, visited)
        }
        ty => is_flat_scalar(document, carrier, ty, visited),
    }
}

fn is_flat_scalar(
    document: &Document,
    carrier: &Carrier,
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
                    let spelled = is_flat_value(document, carrier, inner, carrier.array, visited);
                    visited.pop();
                    spelled.is_ok()
                }
                TypeKind::Struct { .. } => false,
            }
        }
        TypeRef::Inline(_) => false,
    };
    if supported {
        Ok(())
    } else {
        Err(QueryRejection::NonScalarField(String::new()))
    }
}
