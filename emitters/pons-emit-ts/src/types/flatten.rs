use std::collections::{BTreeMap, BTreeSet};

use pons_ir::{Document, InlineType, ObjectOpenness, TypeKind, TypeRef};

use crate::{EmitError, TsOptions};

use super::field::field_is_visible;
use super::inline::local_type_ref;

pub(super) fn openness_value<'a>(ty: &'a TypeRef, document: &'a Document) -> &'a TypeRef {
    let mut current = ty;
    let mut visited = BTreeSet::new();
    loop {
        match current {
            TypeRef::Inline(InlineType::Map { value, .. }) => return value,
            TypeRef::Named(named) if visited.insert(named.id.clone()) => {
                let Some(TypeKind::Newtype { inner }) = document
                    .types
                    .get(&named.id)
                    .map(|definition| &definition.definition)
                else {
                    return current;
                };
                current = inner;
            }
            _ => return current,
        }
    }
}

pub(super) fn flattened_field_types(
    references: &[TypeRef],
    document: &Document,
    names: &BTreeMap<String, String>,
    options: &TsOptions,
) -> Result<Vec<String>, EmitError> {
    let mut result = Vec::new();
    let mut visited = BTreeSet::new();
    let mut visiting = Vec::new();
    for reference in references {
        collect_flattened_field_types(
            reference,
            document,
            names,
            options,
            &mut visited,
            &mut visiting,
            &mut result,
        )?;
    }
    Ok(result)
}

fn collect_flattened_field_types(
    reference: &TypeRef,
    document: &Document,
    names: &BTreeMap<String, String>,
    options: &TsOptions,
    visited: &mut BTreeSet<String>,
    visiting: &mut Vec<String>,
    result: &mut Vec<String>,
) -> Result<(), EmitError> {
    let TypeRef::Named(named) = reference else {
        if matches!(reference, TypeRef::Inline(InlineType::Opaque { .. })) {
            return Ok(());
        }
        return Err(not_a_struct_member(reference));
    };
    if visiting.contains(&named.id) {
        return Err(EmitError(format!(
            "the type {} cannot be inlined because flattened members form a cycle",
            named.id
        )));
    }
    if !visited.insert(named.id.clone()) {
        return Ok(());
    }
    let definition = document
        .types
        .get(&named.id)
        .ok_or_else(|| EmitError(format!("unknown type {}", named.id)))?;
    visiting.push(named.id.clone());
    match &definition.definition {
        TypeKind::Struct {
            fields,
            flattened,
            openness,
        } => {
            for field in fields
                .iter()
                .filter(|field| field_is_visible(field, definition.direction))
            {
                result.push(local_type_ref(&field.ty, document, names, options)?);
            }
            if let ObjectOpenness::PreserveTyped { value } = openness {
                result.push(local_type_ref(
                    openness_value(value, document),
                    document,
                    names,
                    options,
                )?);
            }
            for flattened in flattened {
                collect_flattened_field_types(
                    flattened, document, names, options, visited, visiting, result,
                )?;
            }
        }
        TypeKind::Newtype { inner } => collect_flattened_field_types(
            inner, document, names, options, visited, visiting, result,
        )?,
        TypeKind::Enum { .. } => return Err(not_a_struct_member(reference)),
    }
    visiting.pop();
    Ok(())
}

fn not_a_struct_member(ty: &TypeRef) -> EmitError {
    let name = match ty {
        TypeRef::Named(named) => match named.variant.as_deref() {
            Some(variant) => format!("the {variant} variant of the type {}", named.id),
            None => format!("the type {}", named.id),
        },
        TypeRef::Inline(InlineType::Map { .. }) => "an inline map".into(),
        TypeRef::Inline(_) => "an inline type".into(),
    };
    EmitError(format!(
        "{name} cannot be inlined into a struct because only struct members are representable"
    ))
}

pub(super) fn flattened_opaque<'a>(
    references: &'a [TypeRef],
    document: &Document,
) -> Option<&'a TypeRef> {
    let mut visited = BTreeSet::new();
    references
        .iter()
        .find(|reference| resolves_to_opaque(reference, document, &mut visited))
}

fn resolves_to_opaque(
    reference: &TypeRef,
    document: &Document,
    visited: &mut BTreeSet<String>,
) -> bool {
    let TypeRef::Named(named) = reference else {
        return matches!(reference, TypeRef::Inline(InlineType::Opaque { .. }));
    };
    if named.variant.is_some() || !visited.insert(named.id.clone()) {
        return false;
    }
    let Some(definition) = document
        .types
        .get(&named.id)
        .map(|definition| &definition.definition)
    else {
        return false;
    };
    match definition {
        TypeKind::Newtype { inner } => resolves_to_opaque(inner, document, visited),
        TypeKind::Struct { flattened, .. } => flattened
            .iter()
            .any(|reference| resolves_to_opaque(reference, document, visited)),
        TypeKind::Enum { .. } => false,
    }
}
