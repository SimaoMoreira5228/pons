use std::collections::BTreeSet;

use pons_ir::{
    Document, ErrorContent, ErrorResponse, Method, ResponseHeader, TypeKind, TypeRef,
    ValidationError,
};

use crate::{error, operation_cx, routing};

pub(crate) fn checked(mut document: Document) -> Result<Document, Vec<ValidationError>> {
    document.canonicalize();
    document.validate()?;
    Ok(document)
}

pub(crate) fn prune_unreferenced_types(document: &mut Document, carriers: &[TypeRef]) {
    let mut reachable = BTreeSet::new();
    let mut pending = root_references(document);
    pending.extend_from_slice(carriers);
    while let Some(ty) = pending.pop() {
        let TypeRef::Named(named) = &ty else {
            pending.extend(inline_references(&ty));
            continue;
        };
        if !reachable.insert(named.id.clone()) {
            continue;
        }
        if let Some(definition) = document.types.get(&named.id) {
            pending.extend(definition_references(&definition.definition));
        }
    }
    document.types.retain(|id, _| reachable.contains(id));
}

pub(crate) fn settle(common_errors: &[ErrorResponse], operations: &mut [pons_ir::Operation]) {
    reuse_inherited_slots(common_errors, operations);
    declare_degraded_headers(common_errors, operations);
    settle_head_answers(operations);
}

pub(crate) fn declares_status(operation: &pons_ir::Operation, status: u16) -> bool {
    let exact = pons_ir::ErrorStatus::Exact(status);
    operation
        .errors
        .iter()
        .any(|error| error.status.covers(&exact))
        || operation
            .responses
            .iter()
            .any(|response| match response.status {
                pons_ir::ResponseStatus::Exact(published) => published == status,
                pons_ir::ResponseStatus::Class(class) => u16::from(class) == status / 100,
                pons_ir::ResponseStatus::Default | pons_ir::ResponseStatus::Unknown => false,
            })
}

fn root_references(document: &Document) -> Vec<TypeRef> {
    let mut roots = Vec::new();
    let push = |body: &Option<TypeRef>, headers: &[ResponseHeader], roots: &mut Vec<TypeRef>| {
        roots.extend(body.clone());
        roots.extend(headers.iter().map(|header| header.ty.clone()));
    };
    for operation in &document.operations {
        operation.path.iter().for_each(|segment| {
            if let pons_ir::PathSegment::Param { ty, .. } = segment {
                roots.push(ty.clone());
            }
        });
        roots.extend(operation.query.clone());
        operation
            .headers
            .iter()
            .for_each(|header| roots.push(header.ty.clone()));
        if let Some(body) = &operation.body {
            roots.push(body.ty.clone());
        }
        operation
            .responses
            .iter()
            .for_each(|response| push(&response.body, &response.headers, &mut roots));
        operation
            .errors
            .iter()
            .for_each(|error| push(&error.body, &error.headers, &mut roots));
        if let Some(stream) = &operation.stream {
            match &stream.kind {
                pons_ir::StreamKind::Sse { events } => events
                    .iter()
                    .for_each(|event| roots.push(event.data.clone())),
                pons_ir::StreamKind::Ndjson { item } => roots.push(item.clone()),
                pons_ir::StreamKind::Bytes { .. } => {}
                pons_ir::StreamKind::Websocket { client, server } => {
                    roots.push(client.clone());
                    roots.push(server.clone());
                }
            }
        }
    }
    document
        .common_errors
        .iter()
        .for_each(|error| push(&error.body, &error.headers, &mut roots));
    roots
}

fn definition_references(definition: &TypeKind) -> Vec<TypeRef> {
    match definition {
        TypeKind::Struct {
            fields,
            flattened,
            openness,
        } => {
            let mut references = fields
                .iter()
                .map(|field| field.ty.clone())
                .collect::<Vec<_>>();
            references.extend(flattened.clone());
            if let pons_ir::ObjectOpenness::PreserveTyped { value } = openness {
                references.push(value.clone());
            }
            references
        }
        TypeKind::Enum { variants, .. } => variants
            .iter()
            .flat_map(|variant| match &variant.payload {
                pons_ir::VariantPayload::Unit => Vec::new(),
                pons_ir::VariantPayload::Newtype { ty } => vec![ty.clone()],
                pons_ir::VariantPayload::Tuple { items } => items.clone(),
                pons_ir::VariantPayload::Struct { fields } => {
                    fields.iter().map(|field| field.ty.clone()).collect()
                }
            })
            .collect(),
        TypeKind::Newtype { inner } => vec![inner.clone()],
    }
}

fn inline_references(ty: &TypeRef) -> Vec<TypeRef> {
    match ty {
        TypeRef::Inline(pons_ir::InlineType::Nullable { inner }) => vec![(**inner).clone()],
        TypeRef::Inline(pons_ir::InlineType::Array { item, .. }) => vec![(**item).clone()],
        TypeRef::Inline(pons_ir::InlineType::Map { key, value }) => {
            vec![(**key).clone(), (**value).clone()]
        }
        TypeRef::Inline(pons_ir::InlineType::Tuple { items }) => items.clone(),
        _ => Vec::new(),
    }
}

fn reuse_inherited_slots(common: &[ErrorResponse], operations: &mut [pons_ir::Operation]) {
    if common.is_empty() {
        return;
    }
    for operation in operations {
        if operation.method == Method::Head && common.iter().any(|error| error.body.is_some()) {
            operation.errors_inherit = false;
            operation_cx::push_diagnostic(
                &mut operation.diagnostics,
                "PONS-W101",
                "a HEAD response carries no body, so a common error that carries one is not inherited by a HEAD operation",
            );
            continue;
        }
        let mut collisions = Vec::new();
        operation.errors.retain(|error| {
            if error.name != error::REJECTED_NAME {
                return true;
            }
            let Some(inherited) = common
                .iter()
                .find(|inherited| inherited.status.covers(&error.status))
            else {
                return true;
            };
            let differences = wire_differences(inherited, error);
            if differences.is_empty() {
                return false;
            }
            collisions.push(Collision {
                status: error.status.clone(),
                name: inherited.name.clone(),
                inherited_status: inherited.status.clone(),
                declared_type: error_type_id(error),
                inherited_type: error_type_id(inherited),
                differences: differences.join(" and "),
            });
            true
        });
        collisions.into_iter().for_each(|collision| {
            operation_cx::push_diagnostic(
                &mut operation.diagnostics,
                "PONS-E102",
                collision.message(),
            );
        });
    }
}

struct Collision {
    status: pons_ir::ErrorStatus,
    name: Vec<String>,
    inherited_status: pons_ir::ErrorStatus,
    declared_type: String,
    inherited_type: String,
    differences: String,
}

impl Collision {
    fn message(&self) -> String {
        format!(
            "a request is rejected as {:?} by the error of type {}, but the common error {:?} of type {} answers {:?} with a different {}",
            self.status,
            self.declared_type,
            self.name,
            self.inherited_type,
            self.inherited_status,
            self.differences
        )
    }
}

fn error_type_id(error: &ErrorResponse) -> String {
    match &error.body {
        Some(TypeRef::Named(named)) => named.id.clone(),
        _ => "no body type".to_owned(),
    }
}

fn wire_differences(inherited: &ErrorResponse, declared: &ErrorResponse) -> Vec<&'static str> {
    [
        (
            "body",
            both_spelled_differ(inherited.body.as_ref(), declared.body.as_ref()),
        ),
        ("content", inherited.content != declared.content),
        (
            "media type",
            effective_media_type(inherited) != effective_media_type(declared),
        ),
        (
            "response headers",
            !same_headers(&inherited.headers, &declared.headers),
        ),
        ("retryable", inherited.retryable != declared.retryable),
        ("extensions", inherited.ext != declared.ext),
    ]
    .into_iter()
    .filter_map(|(field, differing)| differing.then_some(field))
    .collect()
}

fn both_spelled_differ<T: PartialEq>(inherited: Option<&T>, declared: Option<&T>) -> bool {
    match (inherited, declared) {
        (Some(inherited), Some(declared)) => inherited != declared,
        _ => false,
    }
}

fn effective_media_type(error: &ErrorResponse) -> Option<String> {
    match &error.media {
        Some(declared) => Some(declared.to_string()),
        None => match error.content {
            ErrorContent::Json => Some("application/json".into()),
            ErrorContent::ProblemJson => Some("application/problem+json".into()),
            ErrorContent::Text => Some("text/plain".into()),
            ErrorContent::Empty => None,
        },
    }
}

fn same_headers(inherited: &[ResponseHeader], declared: &[ResponseHeader]) -> bool {
    inherited.len() == declared.len()
        && inherited.iter().all(|left| {
            declared.iter().any(|right| {
                left.wire.eq_ignore_ascii_case(&right.wire)
                    && left.ty == right.ty
                    && left.presence == right.presence
                    && left.ext == right.ext
            })
        })
}

fn declare_degraded_headers(common: &[ErrorResponse], operations: &mut [pons_ir::Operation]) {
    let exact = pons_ir::ErrorStatus::Exact(error::DEGRADED);
    for operation in operations {
        let header_wrapped = operation
            .responses
            .iter()
            .any(|response| !response.headers.is_empty());
        let inherited =
            operation.errors_inherit && common.iter().any(|error| error.status.covers(&exact));
        if header_wrapped && !inherited && !declares_status(operation, error::DEGRADED) {
            operation.errors.push(error::degraded_error());
        }
    }
}

fn settle_head_answers(operations: &mut [pons_ir::Operation]) {
    let heads: Vec<Vec<pons_ir::PathSegment>> = operations
        .iter()
        .filter(|operation| operation.method == Method::Head)
        .map(|operation| operation.path.clone())
        .collect();
    operations
        .iter_mut()
        .filter(|operation| operation.method == Method::Get)
        .filter(|operation| heads.contains(&operation.path))
        .for_each(|operation| {
            operation
                .diagnostics
                .retain(|entry| entry.code != routing::HEAD_OVER_GET);
        });
}
