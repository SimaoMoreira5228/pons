use std::collections::{BTreeMap, BTreeSet};

use pons_ir::{
    AuthScheme, Document, ErrorResponse, Info, Operation, ResponseHeader, TypeDef, TypeKind,
    TypeRef, ValidationError,
};

pub(crate) fn assemble(
    info: Info,
    types: BTreeMap<String, TypeDef>,
    carriers: Vec<TypeRef>,
    common_errors: Vec<ErrorResponse>,
    auth_schemes: BTreeMap<String, AuthScheme>,
    operations: Vec<Operation>,
    diagnostics: Vec<pons_ir::Diagnostic>,
) -> Document {
    let mut document = Document {
        ir_version: pons_ir::IR_VERSION.to_owned(),
        info,
        types,
        auth_schemes,
        common_errors,
        operations,
        ext: BTreeMap::new(),
        diagnostics,
    };
    prune_unreferenced_types(&mut document, &carriers);
    document
}

pub(crate) fn checked(mut document: Document) -> Result<Document, Vec<ValidationError>> {
    document.canonicalize();
    document.validate()?;
    Ok(document)
}

fn prune_unreferenced_types(document: &mut Document, carriers: &[TypeRef]) {
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
