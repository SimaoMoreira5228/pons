use std::collections::BTreeSet;

use pons_ir::{
    ContentKind, Document, InlineType, Operation, PaginationRequest, ResponseStatus, TypeKind,
    TypeRef, VariantPayload, operation_subject, query_fields,
};

use crate::EmitError;
use crate::render::string;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Carrier {
    Query,
    Body,
}

impl Carrier {
    pub(super) fn name(self) -> &'static str {
        match self {
            Self::Query => "query",
            Self::Body => "body",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum RequestKind {
    Cursor,
    Offset,
}

impl RequestKind {
    pub(super) fn name(self) -> &'static str {
        match self {
            Self::Cursor => "cursor",
            Self::Offset => "offset",
        }
    }
}

pub(super) struct PageSizePlan {
    pub(super) field: String,
    pub(super) carrier: Carrier,
    pub(super) default: Option<u32>,
}

pub(super) struct Plan {
    pub(super) kind: RequestKind,
    pub(super) field: String,
    pub(super) carrier: Carrier,
    pub(super) items: String,
    pub(super) next: Option<String>,
    pub(super) page_size: Option<PageSizePlan>,
    pub(super) item: TypeRef,
    pub(super) wrap_status: bool,
    pub(super) wrap_headers: bool,
}

pub(super) enum Planning {
    Absent,
    Emitted(Plan),
    Lossy { pointer: String, reason: String },
}

pub(super) fn plan(document: &Document, operation: &Operation) -> Result<Planning, EmitError> {
    let Some(pagination) = operation.semantics.pagination.as_ref() else {
        return Ok(Planning::Absent);
    };
    let field = pagination.request.field().to_owned();
    let request_carrier = carrier(document, operation, &field)?;
    let page_size = match &pagination.page_size {
        Some(size) => Some(PageSizePlan {
            field: size.field.clone(),
            carrier: carrier(document, operation, &size.field)?,
            default: size.default,
        }),
        None => None,
    };
    let pointer = pagination.items.as_str().to_owned();
    if operation.stream.is_some() {
        return Ok(Planning::Lossy {
            pointer,
            reason: "the operation streams its response".into(),
        });
    }
    let (body, wrap_status, wrap_headers) = match success_body(operation) {
        Ok(body) => body,
        Err(reason) => return Ok(Planning::Lossy { pointer, reason }),
    };
    let item = match resolve_item(document, body, &pointer) {
        Ok(item) => item,
        Err(reason) => return Ok(Planning::Lossy { pointer, reason }),
    };
    Ok(Planning::Emitted(Plan {
        kind: match pagination.request {
            PaginationRequest::Cursor { .. } => RequestKind::Cursor,
            PaginationRequest::Offset { .. } => RequestKind::Offset,
        },
        field,
        carrier: request_carrier,
        items: pointer,
        next: pagination
            .next
            .as_ref()
            .map(|next| next.as_str().to_owned()),
        page_size,
        item,
        wrap_status,
        wrap_headers,
    }))
}

fn carrier(document: &Document, operation: &Operation, field: &str) -> Result<Carrier, EmitError> {
    let query = operation.query.as_ref().is_some_and(|query| {
        query_fields(document, query)
            .is_ok_and(|fields| fields.iter().any(|declared| declared.wire == field))
    });
    let body = operation.body.as_ref().is_some_and(|body| {
        field_in_type(document, &body.ty, field)
            .ok()
            .flatten()
            .is_some()
    });
    match (query, body) {
        (true, true) => Err(EmitError(format!(
            "the TypeScript emitter cannot choose where the pagination field {} is sent for \
             operations.{}, because the operation declares it on both the query and the body",
            string(field),
            operation_subject(operation)
        ))),
        (true, false) => Ok(Carrier::Query),
        (false, true) => Ok(Carrier::Body),
        (false, false) => Err(EmitError(format!(
            "the TypeScript emitter cannot find the pagination field {} on the query or body of \
             operations.{}",
            string(field),
            operation_subject(operation)
        ))),
    }
}

fn success_body(operation: &Operation) -> Result<(&TypeRef, bool, bool), String> {
    if operation.responses.len() != 1 {
        return Err("the operation does not declare exactly one success response".into());
    }
    let response = &operation.responses[0];
    if !matches!(
        response.content,
        ContentKind::Json | ContentKind::ProblemJson
    ) {
        return Err("the success response is not a JSON body".into());
    }
    let Some(body) = &response.body else {
        return Err("the success response declares no body".into());
    };
    Ok((
        body,
        matches!(
            response.status,
            ResponseStatus::Default | ResponseStatus::Unknown
        ),
        !response.headers.is_empty(),
    ))
}

fn resolve_item(document: &Document, body: &TypeRef, pointer: &str) -> Result<TypeRef, String> {
    let mut current = body.clone();
    for segment in pointer_segments(pointer)? {
        current = step(document, &current, &segment)?;
    }
    match strip_nullable(&current) {
        TypeRef::Inline(InlineType::Array { item, .. }) => Ok(item.as_ref().clone()),
        _ => Err("does not end at an array".into()),
    }
}

fn step(document: &Document, ty: &TypeRef, segment: &str) -> Result<TypeRef, String> {
    match ty {
        TypeRef::Named(named) => {
            let Some(definition) = document.types.get(&named.id) else {
                return Err(format!("names the unknown type {}", named.id));
            };
            if let Some(variant) = &named.variant {
                let TypeKind::Enum { variants, .. } = &definition.definition else {
                    return Err(format!(
                        "names the variant {} of a type that is not an enum",
                        string(variant)
                    ));
                };
                let Some(found) = variants.iter().find(|candidate| &candidate.wire == variant)
                else {
                    return Err(format!("names the unknown variant {}", string(variant)));
                };
                return match &found.payload {
                    VariantPayload::Newtype { ty } => step(document, ty, segment),
                    VariantPayload::Struct { fields } => {
                        field_in_fields(document, fields, &[], segment, &mut BTreeSet::new())?
                            .ok_or_else(|| format!("does not find the field {}", string(segment)))
                    }
                    _ => Err(format!(
                        "reaches the variant {} that has no named fields",
                        string(variant)
                    )),
                };
            }
            match &definition.definition {
                TypeKind::Newtype { inner } => step(document, inner, segment),
                TypeKind::Struct {
                    fields, flattened, ..
                } => field_in_fields(document, fields, flattened, segment, &mut BTreeSet::new())?
                    .ok_or_else(|| format!("does not find the field {}", string(segment))),
                TypeKind::Enum { .. } => Err("reaches an enum".into()),
            }
        }
        TypeRef::Inline(InlineType::Nullable { inner }) => step(document, inner, segment),
        TypeRef::Inline(InlineType::Array { item, .. }) => {
            if is_index(segment) {
                Ok(item.as_ref().clone())
            } else {
                Err(format!(
                    "reaches an array but the segment {} is not an index",
                    string(segment)
                ))
            }
        }
        TypeRef::Inline(InlineType::Map { .. }) => Err("reaches a map".into()),
        TypeRef::Inline(InlineType::Any) => Err("reaches any".into()),
        TypeRef::Inline(InlineType::Opaque { .. }) => Err("reaches an opaque value".into()),
        _ => Err(format!("ends before the segment {}", string(segment))),
    }
}

fn field_in_type(document: &Document, ty: &TypeRef, wire: &str) -> Result<Option<TypeRef>, String> {
    field_in_visited(document, ty, wire, &mut BTreeSet::new())
}

fn field_in_visited(
    document: &Document,
    ty: &TypeRef,
    wire: &str,
    visited: &mut BTreeSet<String>,
) -> Result<Option<TypeRef>, String> {
    match ty {
        TypeRef::Named(named) => {
            let Some(definition) = document.types.get(&named.id) else {
                return Err(format!("names the unknown type {}", named.id));
            };
            if let Some(variant) = &named.variant {
                let TypeKind::Enum { variants, .. } = &definition.definition else {
                    return Err(format!(
                        "names the variant {} of a type that is not an enum",
                        string(variant)
                    ));
                };
                let Some(found) = variants.iter().find(|candidate| &candidate.wire == variant)
                else {
                    return Err(format!("names the unknown variant {}", string(variant)));
                };
                return match &found.payload {
                    VariantPayload::Newtype { ty } => field_in_visited(document, ty, wire, visited),
                    VariantPayload::Struct { fields } => {
                        field_in_fields(document, fields, &[], wire, visited)
                    }
                    _ => Err(format!(
                        "reaches the variant {} that has no named fields",
                        string(variant)
                    )),
                };
            }
            if !visited.insert(named.id.clone()) {
                return Ok(None);
            }
            let result = match &definition.definition {
                TypeKind::Newtype { inner } => field_in_visited(document, inner, wire, visited),
                TypeKind::Struct {
                    fields, flattened, ..
                } => field_in_fields(document, fields, flattened, wire, visited),
                TypeKind::Enum { .. } => Err("reaches an enum".into()),
            };
            visited.remove(&named.id);
            result
        }
        TypeRef::Inline(InlineType::Nullable { inner }) => {
            field_in_visited(document, inner, wire, visited)
        }
        TypeRef::Inline(InlineType::Map { .. }) => Err("reaches a map".into()),
        TypeRef::Inline(InlineType::Any) => Err("reaches any".into()),
        TypeRef::Inline(InlineType::Opaque { .. }) => Err("reaches an opaque value".into()),
        _ => Ok(None),
    }
}

fn field_in_fields(
    document: &Document,
    fields: &[pons_ir::Field],
    flattened: &[TypeRef],
    wire: &str,
    visited: &mut BTreeSet<String>,
) -> Result<Option<TypeRef>, String> {
    if let Some(field) = fields.iter().find(|field| field.wire == wire) {
        return Ok(Some(field.ty.clone()));
    }
    for reference in flattened {
        if let Some(found) = field_in_visited(document, reference, wire, visited)? {
            return Ok(Some(found));
        }
    }
    Ok(None)
}

fn strip_nullable(mut ty: &TypeRef) -> &TypeRef {
    while let TypeRef::Inline(InlineType::Nullable { inner }) = ty {
        ty = inner.as_ref();
    }
    ty
}

fn pointer_segments(pointer: &str) -> Result<Vec<String>, String> {
    if pointer.is_empty() {
        return Ok(Vec::new());
    }
    let Some(rest) = pointer.strip_prefix('/') else {
        return Err("is not a JSON pointer".into());
    };
    rest.split('/').map(decode_segment).collect()
}

fn decode_segment(segment: &str) -> Result<String, String> {
    let mut decoded = String::new();
    let mut characters = segment.chars();
    while let Some(character) = characters.next() {
        if character == '~' {
            match characters.next() {
                Some('0') => decoded.push('~'),
                Some('1') => decoded.push('/'),
                _ => return Err("has an invalid escape".into()),
            }
        } else {
            decoded.push(character);
        }
    }
    Ok(decoded)
}

fn is_index(segment: &str) -> bool {
    if segment == "0" {
        return true;
    }
    !segment.is_empty()
        && !segment.starts_with('0')
        && segment.bytes().all(|byte| byte.is_ascii_digit())
}
