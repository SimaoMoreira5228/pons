use std::collections::BTreeMap;

use pons_ir::{
    CoverageLevel, Document, TypeRef, ValidationCoverage, coverage_subjects, operation_subject,
    type_subject,
};

pub(crate) fn propagate(
    document: &Document,
    seeds: &BTreeMap<String, CoverageLevel>,
) -> ValidationCoverage {
    let mut levels = coverage_subjects(document)
        .into_iter()
        .map(|subject| (subject, CoverageLevel::Exact))
        .collect::<BTreeMap<_, _>>();
    for (subject, level) in seeds {
        if let Some(current) = levels.get_mut(subject) {
            *current = (*current).max(*level);
        }
    }

    let dependencies = document
        .types
        .iter()
        .map(|(id, definition)| {
            let mut referenced = Vec::new();
            definition
                .definition
                .visit_refs(&mut |reference| referenced.push(reference.clone()));
            (id.clone(), referenced)
        })
        .collect::<BTreeMap<_, _>>();

    loop {
        let mut changed = false;
        for (id, referenced) in &dependencies {
            let subject = type_subject(id);
            let mut level = levels[&subject];
            for reference in referenced {
                level = level.max(reference_level(&levels, reference));
            }
            if level > levels[&subject] {
                levels.insert(subject, level);
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }

    for operation in &document.operations {
        let subject = format!("operations.{}", operation_subject(operation));
        let mut level = levels[&subject];
        operation.visit_refs(&mut |reference| {
            level = level.max(reference_level(&levels, reference));
        });
        if operation.errors_inherit {
            for error in &document.common_errors {
                if let Some(body) = &error.body {
                    level = level.max(reference_level(&levels, body));
                }
                for header in &error.headers {
                    level = level.max(reference_level(&levels, &header.ty));
                }
            }
        }
        levels.insert(subject, level);
    }

    let mut coverage = ValidationCoverage::default();
    for (subject, level) in levels {
        match level {
            CoverageLevel::Exact => {
                coverage.exact.insert(subject);
            }
            CoverageLevel::Opaque | CoverageLevel::Lossy => {
                coverage.lossy.insert(subject);
            }
            CoverageLevel::Unsupported => {
                coverage.unsupported.insert(subject);
            }
        }
    }
    coverage
}

fn reference_level(levels: &BTreeMap<String, CoverageLevel>, reference: &TypeRef) -> CoverageLevel {
    let mut level = CoverageLevel::Exact;
    reference.visit_refs(&mut |reference| {
        if let TypeRef::Named(named) = reference {
            level = level.max(
                levels
                    .get(&type_subject(&named.id))
                    .copied()
                    .unwrap_or(CoverageLevel::Exact),
            );
        }
    });
    level
}
