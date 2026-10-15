use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use serde_json::Value;

use crate::{
    Doc, Document, ErrorResponse, Field, Operation, Response, StreamKind, TypeDef, TypeKind,
    TypeRef, VariantPayload,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EquivalenceError {
    pub path: String,
    pub message: String,
}

impl fmt::Display for EquivalenceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.path.is_empty() {
            formatter.write_str(&self.message)
        } else {
            write!(formatter, "{}: {}", self.path, self.message)
        }
    }
}

pub fn equivalent(left: &Document, right: &Document) -> Result<(), Vec<EquivalenceError>> {
    let left = normalized(left)?;
    let right = normalized(right)?;
    let left = serde_json::to_value(&left).map_err(|error| {
        vec![failure(
            "",
            format!("cannot serialize the left document: {error}"),
        )]
    })?;
    let right = serde_json::to_value(&right).map_err(|error| {
        vec![failure(
            "",
            format!("cannot serialize the right document: {error}"),
        )]
    })?;
    let mut differences = Vec::new();
    difference("", &left, &right, &mut differences);
    if differences.is_empty() {
        Ok(())
    } else {
        Err(differences)
    }
}

fn normalized(document: &Document) -> Result<Document, Vec<EquivalenceError>> {
    let mut document = document.clone();
    document.canonicalize();
    document.diagnostics.clear();
    document.info.description = normalize_text(document.info.description.take());
    let ids = display_ids(&document)?;
    normalize_errors(&mut document.common_errors, &ids)?;
    for operation in &mut document.operations {
        normalize_operation(operation, &ids)?;
    }
    document.types = normalize_types(document.types, &ids)?;
    Ok(document)
}

fn display_ids(document: &Document) -> Result<BTreeMap<String, String>, Vec<EquivalenceError>> {
    let mut ids = BTreeMap::new();
    let mut names = BTreeMap::new();
    for (id, definition) in &document.types {
        let key = format!(
            "{} ({:?})",
            definition.name.join("::"),
            definition.direction
        );
        if let Some(previous) = names.get(&key) {
            return Err(vec![failure(
                "types",
                format!(
                    "the display name {key} is used by both {previous} and {id}; equivalence cannot rename it"
                ),
            )]);
        }
        names.insert(key.clone(), id.clone());
        ids.insert(id.clone(), key);
    }
    Ok(ids)
}

fn normalize_types(
    types: BTreeMap<String, TypeDef>,
    ids: &BTreeMap<String, String>,
) -> Result<BTreeMap<String, TypeDef>, Vec<EquivalenceError>> {
    let mut normalized = BTreeMap::new();
    for (id, mut definition) in types {
        normalize_kind(&mut definition.definition, ids)?;
        normalize_doc(&mut definition.doc);
        definition.origin = None;
        definition.diagnostics.clear();
        let key = ids
            .get(&id)
            .cloned()
            .expect("every type already has a display name");
        normalized.insert(key, definition);
    }
    Ok(normalized)
}

fn normalize_kind(
    kind: &mut TypeKind,
    ids: &BTreeMap<String, String>,
) -> Result<(), Vec<EquivalenceError>> {
    match kind {
        TypeKind::Struct {
            fields,
            flattened,
            openness,
        } => {
            for field in fields {
                normalize_field(field, ids)?;
            }
            for ty in flattened {
                normalize_ref(ty, ids)?;
            }
            if let crate::ObjectOpenness::PreserveTyped { value } = openness {
                normalize_ref(value, ids)?;
            }
        }
        TypeKind::Enum { variants, .. } => {
            for variant in variants {
                normalize_doc(&mut variant.doc);
                variant.diagnostics.clear();
                match &mut variant.payload {
                    VariantPayload::Unit => {}
                    VariantPayload::Newtype { ty } => normalize_ref(ty, ids)?,
                    VariantPayload::Tuple { items } => {
                        for ty in items {
                            normalize_ref(ty, ids)?;
                        }
                    }
                    VariantPayload::Struct { fields } => {
                        for field in fields {
                            normalize_field(field, ids)?;
                        }
                    }
                }
            }
        }
        TypeKind::Newtype { inner } => normalize_ref(inner, ids)?,
    }
    Ok(())
}

fn normalize_field(
    field: &mut Field,
    ids: &BTreeMap<String, String>,
) -> Result<(), Vec<EquivalenceError>> {
    normalize_ref(&mut field.ty, ids)?;
    normalize_doc(&mut field.doc);
    field.diagnostics.clear();
    Ok(())
}

fn normalize_operation(
    operation: &mut Operation,
    ids: &BTreeMap<String, String>,
) -> Result<(), Vec<EquivalenceError>> {
    for segment in &mut operation.path {
        if let crate::PathSegment::Param { ty, .. } = segment {
            normalize_ref(ty, ids)?;
        }
    }
    if let Some(query) = &mut operation.query {
        normalize_ref(query, ids)?;
    }
    for header in &mut operation.headers {
        normalize_ref(&mut header.ty, ids)?;
    }
    if let Some(body) = &mut operation.body {
        normalize_ref(&mut body.ty, ids)?;
    }
    for response in &mut operation.responses {
        normalize_response(response, ids)?;
    }
    normalize_errors(&mut operation.errors, ids)?;
    if let Some(stream) = &mut operation.stream {
        match &mut stream.kind {
            StreamKind::Sse { events } => {
                for event in events {
                    normalize_ref(&mut event.data, ids)?;
                }
            }
            StreamKind::Ndjson { item } => normalize_ref(item, ids)?,
            StreamKind::Bytes { .. } => {}
            StreamKind::Websocket { client, server } => {
                normalize_ref(client, ids)?;
                normalize_ref(server, ids)?;
            }
        }
    }
    normalize_doc(&mut operation.doc);
    operation.origin = None;
    operation.diagnostics.clear();
    Ok(())
}

fn normalize_response(
    response: &mut Response,
    ids: &BTreeMap<String, String>,
) -> Result<(), Vec<EquivalenceError>> {
    if let Some(body) = &mut response.body {
        normalize_ref(body, ids)?;
    }
    for header in &mut response.headers {
        normalize_ref(&mut header.ty, ids)?;
    }
    Ok(())
}

fn normalize_errors(
    errors: &mut [ErrorResponse],
    ids: &BTreeMap<String, String>,
) -> Result<(), Vec<EquivalenceError>> {
    for error in errors {
        if let Some(body) = &mut error.body {
            normalize_ref(body, ids)?;
        }
        for header in &mut error.headers {
            normalize_ref(&mut header.ty, ids)?;
        }
        normalize_doc(&mut error.doc);
    }
    Ok(())
}

fn normalize_ref(
    ty: &mut TypeRef,
    ids: &BTreeMap<String, String>,
) -> Result<(), Vec<EquivalenceError>> {
    match ty {
        TypeRef::Named(named) => {
            let display = ids.get(&named.id).cloned().ok_or_else(|| {
                vec![failure(
                    "types",
                    format!(
                        "the reference {} names a type the document does not define",
                        named.id
                    ),
                )]
            })?;
            named.id = display;
        }
        TypeRef::Inline(inline) => normalize_inline(inline, ids)?,
    }
    Ok(())
}

fn normalize_inline(
    inline: &mut crate::InlineType,
    ids: &BTreeMap<String, String>,
) -> Result<(), Vec<EquivalenceError>> {
    match inline {
        crate::InlineType::Nullable { inner } => normalize_ref(inner, ids)?,
        crate::InlineType::Array { item, .. } => normalize_ref(item, ids)?,
        crate::InlineType::Map { key, value } => {
            normalize_ref(key, ids)?;
            normalize_ref(value, ids)?;
        }
        crate::InlineType::Tuple { items } => {
            for item in items {
                normalize_ref(item, ids)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn normalize_doc(option: &mut Option<Doc>) {
    let Some(doc) = option.as_mut() else {
        return;
    };
    doc.summary = normalize_text(doc.summary.take());
    doc.description = normalize_text(doc.description.take());
    doc.tags = doc
        .tags
        .iter()
        .filter_map(|tag| normalize_text(Some(tag.clone())))
        .collect();
    if doc.summary.is_none()
        && doc.description.is_none()
        && doc.deprecated.is_none()
        && doc.tags.is_empty()
    {
        *option = None;
    }
}

fn normalize_text(text: Option<String>) -> Option<String> {
    let text = text?;
    let normalized = text.split_whitespace().collect::<Vec<_>>().join(" ");
    (!normalized.is_empty()).then_some(normalized)
}

fn difference(path: &str, left: &Value, right: &Value, differences: &mut Vec<EquivalenceError>) {
    match (left, right) {
        (Value::Object(left), Value::Object(right)) => {
            let keys: BTreeSet<&String> = left.keys().chain(right.keys()).collect();
            for key in keys {
                let child = format!("{path}/{key}");
                match (left.get(key), right.get(key)) {
                    (Some(left), Some(right)) => difference(&child, left, right, differences),
                    (Some(left), None) => differences.push(failure(
                        &child,
                        format!("only the left document has {}", compact(left)),
                    )),
                    (None, Some(right)) => differences.push(failure(
                        &child,
                        format!("only the right document has {}", compact(right)),
                    )),
                    (None, None) => {}
                }
            }
        }
        (Value::Array(left), Value::Array(right)) => {
            if left.len() != right.len() {
                differences.push(failure(
                    path,
                    format!("the arrays have {} and {} items", left.len(), right.len()),
                ));
            }
            for (index, (left, right)) in left.iter().zip(right).enumerate() {
                difference(&format!("{path}[{index}]"), left, right, differences);
            }
        }
        (left, right) if left != right => differences.push(failure(
            path,
            format!(
                "the left document has {} and the right has {}",
                compact(left),
                compact(right)
            ),
        )),
        _ => {}
    }
}

fn compact(value: &Value) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "<unprintable>".to_owned())
}

fn failure(path: &str, message: impl Into<String>) -> EquivalenceError {
    EquivalenceError {
        path: path.to_owned(),
        message: message.into(),
    }
}
