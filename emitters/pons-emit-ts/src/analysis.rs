use std::collections::{BTreeMap, BTreeSet};

use pons_ir::{
    Coverage, DecimalWire, Document, Field, FieldFormat, InlineType, IntegerWire, ObjectOpenness,
    PathSegment, StreamKind, TypeDef, TypeKind, TypeRef, ValidationCoverage, VariantPayload,
    operation_subject, strict_violation_message, type_subject,
};

use crate::EmitError;
use crate::naming::camel;
use crate::options::{EmitMode, ValidationBackend};

pub(crate) fn strict_validation_error(
    mode: EmitMode,
    backend: ValidationBackend,
    coverage: &ValidationCoverage,
) -> Result<(), EmitError> {
    if mode == EmitMode::Lossy {
        return Ok(());
    }
    let violations = coverage.strict_violations();
    if violations.is_empty() {
        return Ok(());
    }
    let messages = violations
        .into_iter()
        .flat_map(|(level, subjects)| {
            subjects.into_iter().map(move |subject| {
                format!(
                    "the {} validation backend cannot check {subject} exactly, because validation \
                     coverage is {level}; choose another backend, or run the target in lossy mode",
                    backend.name()
                )
            })
        })
        .collect::<Vec<_>>();
    Err(EmitError(messages.join("\n")))
}

pub(crate) fn validation_warnings(
    backend: ValidationBackend,
    coverage: &ValidationCoverage,
) -> Vec<String> {
    coverage
        .strict_violations()
        .into_iter()
        .flat_map(|(level, subjects)| {
            subjects.into_iter().map(move |subject| {
                format!(
                    "the {} validation backend emits {level} validation coverage for {subject}",
                    backend.name()
                )
            })
        })
        .collect()
}

pub(crate) fn strict_coverage_error(mode: EmitMode, coverage: &Coverage) -> Result<(), EmitError> {
    match mode {
        EmitMode::Strict => strict_violation_message(coverage)
            .map(|violations| EmitError(format!("the TypeScript emitter reported {violations}")))
            .map_or(Ok(()), Err),
        EmitMode::Lossy => Ok(()),
    }
}

#[derive(Default)]
pub(crate) struct TypeScan {
    pub(crate) wide_paths: Vec<String>,
    pub(crate) opaque_paths: Vec<String>,
    pub(crate) decimal_paths: Vec<String>,
    pub(crate) fractional_constraint_paths: Vec<String>,
    pub(crate) date_paths: Vec<String>,
    pub(crate) decimal_owners: BTreeSet<String>,
    pub(crate) references: BTreeMap<String, Vec<String>>,
}

const COMMON_ERRORS: &str = "common_errors";

pub(crate) fn subjects_reaching_a_decimal(
    scan: &TypeScan,
    document: &Document,
) -> BTreeSet<String> {
    let mut subjects = scan.decimal_owners.clone();
    loop {
        let reached = scan
            .references
            .iter()
            .filter(|(owner, references)| {
                !subjects.contains(*owner)
                    && references.iter().any(|target| subjects.contains(target))
            })
            .map(|(owner, _)| owner.clone())
            .collect::<Vec<_>>();
        if reached.is_empty() {
            break;
        }
        subjects.extend(reached);
    }
    if subjects.remove(COMMON_ERRORS) {
        subjects.extend(
            document
                .operations
                .iter()
                .filter(|operation| operation.errors_inherit)
                .map(|operation| format!("operations.{}", operation_subject(operation))),
        );
    }
    subjects
}

pub(crate) fn scan_type_references(document: &Document) -> TypeScan {
    let mut scan = TypeScan::default();
    for (id, definition) in &document.types {
        let subject = type_subject(id);
        scan_definition(&subject, &subject, definition, &mut scan);
    }
    for (index, error) in document.common_errors.iter().enumerate() {
        if let Some(body) = &error.body {
            scan_type_ref(
                body,
                &format!("common_errors[{index}].body"),
                COMMON_ERRORS,
                &mut scan,
            );
        }
        for (header_index, header) in error.headers.iter().enumerate() {
            scan_type_ref(
                &header.ty,
                &format!("common_errors[{index}].headers[{header_index}].ty"),
                COMMON_ERRORS,
                &mut scan,
            );
        }
    }
    for (index, operation) in document.operations.iter().enumerate() {
        let prefix = format!("operations[{index}]");
        let subject = format!("operations.{}", operation_subject(operation));
        for (segment_index, segment) in operation.path.iter().enumerate() {
            if let PathSegment::Param { ty, .. } = segment {
                scan_type_ref(
                    ty,
                    &format!("{prefix}.path[{segment_index}].ty"),
                    &subject,
                    &mut scan,
                );
            }
        }
        if let Some(query) = &operation.query {
            scan_type_ref(query, &format!("{prefix}.query"), &subject, &mut scan);
        }
        for (header_index, header) in operation.headers.iter().enumerate() {
            scan_type_ref(
                &header.ty,
                &format!("{prefix}.headers[{header_index}].ty"),
                &subject,
                &mut scan,
            );
        }
        if let Some(body) = &operation.body {
            scan_type_ref(&body.ty, &format!("{prefix}.body.ty"), &subject, &mut scan);
        }
        for (response_index, response) in operation.responses.iter().enumerate() {
            if let Some(body) = &response.body {
                scan_type_ref(
                    body,
                    &format!("{prefix}.responses[{response_index}].body"),
                    &subject,
                    &mut scan,
                );
            }
            for (header_index, header) in response.headers.iter().enumerate() {
                scan_type_ref(
                    &header.ty,
                    &format!("{prefix}.responses[{response_index}].headers[{header_index}].ty"),
                    &subject,
                    &mut scan,
                );
            }
        }
        for (error_index, error) in operation.errors.iter().enumerate() {
            if let Some(body) = &error.body {
                scan_type_ref(
                    body,
                    &format!("{prefix}.errors[{error_index}].body"),
                    &subject,
                    &mut scan,
                );
            }
            for (header_index, header) in error.headers.iter().enumerate() {
                scan_type_ref(
                    &header.ty,
                    &format!("{prefix}.errors[{error_index}].headers[{header_index}].ty"),
                    &subject,
                    &mut scan,
                );
            }
        }
        if let Some(stream) = &operation.stream {
            match &stream.kind {
                StreamKind::Sse { events } => {
                    for (event_index, event) in events.iter().enumerate() {
                        scan_type_ref(
                            &event.data,
                            &format!("{prefix}.stream.events[{event_index}].data"),
                            &subject,
                            &mut scan,
                        );
                    }
                }
                StreamKind::Ndjson { item } => {
                    scan_type_ref(item, &format!("{prefix}.stream.item"), &subject, &mut scan);
                }
                StreamKind::Websocket { client, server } => {
                    scan_type_ref(
                        client,
                        &format!("{prefix}.stream.client"),
                        &subject,
                        &mut scan,
                    );
                    scan_type_ref(
                        server,
                        &format!("{prefix}.stream.server"),
                        &subject,
                        &mut scan,
                    );
                }
                StreamKind::Bytes { .. } => {}
            }
        }
    }
    scan
}

fn scan_definition(path: &str, owner: &str, definition: &TypeDef, scan: &mut TypeScan) {
    match &definition.definition {
        TypeKind::Struct {
            fields,
            flattened,
            openness,
        } => {
            for (index, field) in fields.iter().enumerate() {
                scan_type_ref(
                    &field.ty,
                    &format!("{path}.fields[{index}].ty"),
                    owner,
                    scan,
                );
            }
            for (index, reference) in flattened.iter().enumerate() {
                scan_type_ref(
                    reference,
                    &format!("{path}.flattened[{index}]"),
                    owner,
                    scan,
                );
            }
            if let ObjectOpenness::PreserveTyped { value } = openness {
                scan_type_ref(value, &format!("{path}.openness.value"), owner, scan);
            }
        }
        TypeKind::Enum { variants, .. } => {
            for (variant_index, variant) in variants.iter().enumerate() {
                scan_payload(
                    &variant.payload,
                    &format!("{path}.variants[{variant_index}].payload"),
                    owner,
                    scan,
                );
            }
        }
        TypeKind::Newtype { inner } => scan_type_ref(inner, &format!("{path}.inner"), owner, scan),
    }
}

fn scan_payload(payload: &VariantPayload, path: &str, owner: &str, scan: &mut TypeScan) {
    match payload {
        VariantPayload::Unit => {}
        VariantPayload::Newtype { ty } => scan_type_ref(ty, &format!("{path}.ty"), owner, scan),
        VariantPayload::Tuple { items } => {
            for (index, item) in items.iter().enumerate() {
                scan_type_ref(item, &format!("{path}.items[{index}]"), owner, scan);
            }
        }
        VariantPayload::Struct { fields } => {
            for (index, field) in fields.iter().enumerate() {
                scan_type_ref(
                    &field.ty,
                    &format!("{path}.fields[{index}].ty"),
                    owner,
                    scan,
                );
            }
        }
    }
}

fn scan_type_ref(reference: &TypeRef, path: &str, owner: &str, scan: &mut TypeScan) {
    if let TypeRef::Named(named) = reference {
        scan.references
            .entry(owner.to_owned())
            .or_default()
            .push(type_subject(&named.id));
        return;
    }
    let TypeRef::Inline(inline) = reference else {
        return;
    };
    match inline {
        InlineType::Int {
            bits,
            wire,
            minimum,
            maximum,
            multiple_of,
            ..
        } => {
            if *wire == IntegerWire::Number && *bits > 53 {
                scan.wide_paths.push(path.to_owned());
            }
            for (name, value) in [
                ("minimum", minimum),
                ("maximum", maximum),
                ("multiple_of", multiple_of),
            ] {
                if value
                    .as_ref()
                    .is_some_and(|value| !is_integer_number(value))
                {
                    scan.fractional_constraint_paths
                        .push(format!("{path}.{name}"));
                }
            }
        }
        InlineType::Decimal {
            wire: DecimalWire::Number,
        } => {
            scan.decimal_paths.push(path.to_owned());
            scan.decimal_owners.insert(owner.to_owned());
        }
        InlineType::Opaque { .. } => scan.opaque_paths.push(path.to_owned()),
        InlineType::String { format, .. } => {
            if format == &Some(FieldFormat::Date) {
                scan.date_paths.push(path.to_owned());
            }
        }
        InlineType::Nullable { inner } => {
            scan_type_ref(inner, &format!("{path}.inner"), owner, scan)
        }
        InlineType::Array { item, .. } => scan_type_ref(item, &format!("{path}.item"), owner, scan),
        InlineType::Map { key, value } => {
            scan_type_ref(key, &format!("{path}.key"), owner, scan);
            scan_type_ref(value, &format!("{path}.value"), owner, scan);
        }
        InlineType::Tuple { items } => {
            for (index, item) in items.iter().enumerate() {
                scan_type_ref(item, &format!("{path}.items[{index}]"), owner, scan);
            }
        }
        _ => {}
    }
}

pub(crate) fn camel_field_keys(document: &Document) -> Result<(), EmitError> {
    for (id, definition) in &document.types {
        collect_camel_field_keys(
            id,
            definition,
            document,
            &mut BTreeMap::new(),
            &mut BTreeSet::new(),
        )?;
    }
    Ok(())
}

fn collect_camel_field_keys(
    id: &str,
    definition: &TypeDef,
    document: &Document,
    claimed: &mut BTreeMap<String, String>,
    visited: &mut BTreeSet<String>,
) -> Result<(), EmitError> {
    if !visited.insert(id.to_owned()) {
        return Ok(());
    }
    match &definition.definition {
        TypeKind::Struct {
            fields, flattened, ..
        } => {
            for field in fields {
                claim_camel_key(id, field, claimed)?;
            }
            for reference in flattened {
                if let TypeRef::Named(named) = reference
                    && let Some(flattened) = document.types.get(&named.id)
                {
                    collect_camel_field_keys(&named.id, flattened, document, claimed, visited)?;
                }
            }
        }
        TypeKind::Enum { variants, .. } => {
            for variant in variants {
                let VariantPayload::Struct { fields } = &variant.payload else {
                    continue;
                };
                let mut payload_keys = BTreeMap::new();
                for field in fields {
                    claim_camel_key(id, field, &mut payload_keys)?;
                }
            }
        }
        TypeKind::Newtype { inner } => {
            if let TypeRef::Named(named) = inner
                && let Some(inner) = document.types.get(&named.id)
            {
                collect_camel_field_keys(&named.id, inner, document, claimed, visited)?;
            }
        }
    }
    Ok(())
}

fn claim_camel_key(
    id: &str,
    field: &Field,
    claimed: &mut BTreeMap<String, String>,
) -> Result<(), EmitError> {
    let key = camel(&field.ident);
    if let Some(other) = claimed.get(&key) {
        if other != &field.wire {
            return Err(EmitError(format!(
                "the TypeScript emitter cannot spell the fields of the type {id} in camelCase, \
                 because the wire names {other} and {} both spell {key} in the emitted client",
                field.wire
            )));
        }
    } else {
        claimed.insert(key, field.wire.clone());
    }
    Ok(())
}

fn is_integer_number(value: &serde_json::Value) -> bool {
    value.as_number().is_some_and(|number| {
        number.as_i128().is_some()
            || number.as_u128().is_some()
            || number.as_f64().is_some_and(|value| value.fract() == 0.0)
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn strict_validation_mode_names_the_backend_subject_and_both_fixes() {
        let coverage = super::ValidationCoverage {
            lossy: ["types.User".into()].into_iter().collect(),
            unsupported: ["operations.auth.login".into()].into_iter().collect(),
            ..super::ValidationCoverage::default()
        };
        let error = super::strict_validation_error(
            super::EmitMode::Strict,
            super::ValidationBackend::Yup,
            &coverage,
        )
        .unwrap_err()
        .0;
        assert_eq!(
            error,
            "the yup validation backend cannot check operations.auth.login exactly, because \
             validation coverage is unsupported; choose another backend, or run the target in \
             lossy mode\n\
             the yup validation backend cannot check types.User exactly, because validation \
             coverage is lossy; choose another backend, or run the target in lossy mode"
        );
        assert!(
            super::strict_validation_error(
                super::EmitMode::Lossy,
                super::ValidationBackend::Yup,
                &coverage
            )
            .is_ok()
        );
        assert!(
            super::strict_validation_error(
                super::EmitMode::Strict,
                super::ValidationBackend::Yup,
                &super::ValidationCoverage::default()
            )
            .is_ok()
        );
    }

    #[test]
    fn strict_mode_names_every_violating_coverage_class() {
        let coverage = super::Coverage {
            lossy: ["types.User".into()].into_iter().collect(),
            unsupported: ["operations.auth.login".into()].into_iter().collect(),
            ..super::Coverage::default()
        };
        let error = super::strict_coverage_error(super::EmitMode::Strict, &coverage)
            .unwrap_err()
            .0;
        assert_eq!(
            error,
            "the TypeScript emitter reported unsupported coverage in strict mode: \
             operations.auth.login; lossy coverage in strict mode: types.User",
            "every violating class is named, worst class first"
        );
        assert!(super::strict_coverage_error(super::EmitMode::Lossy, &coverage).is_ok());
        assert!(
            super::strict_coverage_error(super::EmitMode::Strict, &super::Coverage::default())
                .is_ok()
        );
    }
}
