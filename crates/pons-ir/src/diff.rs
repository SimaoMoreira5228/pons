use std::collections::{BTreeMap, BTreeSet};

use crate::{
    AuthScheme, Body, ContentKind, Diagnostic, DiagnosticSeverity, Direction, Doc, Document,
    ErrorResponse, Field, Info, InlineType, OAuthFlow, Operation, Presence, Response, TypeDef,
    TypeKind, TypeRef, ValidationError,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ChangeLevel {
    Info,
    NonBreaking,
    Warning,
    Breaking,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    pub level: ChangeLevel,
    pub path: String,
    pub detail: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Diff {
    pub changes: Vec<Change>,

    pub new_document_errors: Option<String>,
}

impl Diff {
    pub fn worst(&self) -> Option<ChangeLevel> {
        self.changes.iter().map(|change| change.level).max()
    }

    pub fn has_breaking_changes(&self) -> bool {
        self.worst() == Some(ChangeLevel::Breaking)
    }
}

impl Document {
    pub fn diff(&self, newer: &Self) -> Diff {
        let mut old = self.clone();
        let mut new = newer.clone();
        old.canonicalize();
        new.canonicalize();

        let mut diff = Diff::default();
        diff_validity(&mut diff, &old, &new);
        diff_diagnostics(
            &mut diff,
            "document.diagnostics",
            &old.diagnostics,
            &new.diagnostics,
        );
        if old.ext != new.ext {
            add(
                &mut diff,
                ChangeLevel::Warning,
                "ext",
                "document metadata changed",
            );
        }
        diff_info(&mut diff, &old.info, &new.info);
        diff_types(&mut diff, &old, &new);
        diff_auth_schemes(&mut diff, &old, &new);
        let mut paths = BTreeSet::new();
        diff_errors(
            &mut diff,
            &mut paths,
            "common_errors",
            &old.common_errors,
            &new.common_errors,
        );
        diff_operations(&mut diff, &mut paths, &old, &new);
        diff
    }
}

fn diff_validity(diff: &mut Diff, old: &Document, new: &Document) {
    let old = old.validate().err().map(validation_message);
    let new = new.validate().err().map(validation_message);
    diff.new_document_errors.clone_from(&new);
    match (&old, &new) {
        (None, None) => {}
        (Some(old), None) => add(
            diff,
            ChangeLevel::Info,
            "document",
            format!("the previous document was invalid: {old}"),
        ),
        (None, Some(new)) => add(
            diff,
            ChangeLevel::Breaking,
            "document",
            format!("the new document is invalid: {new}"),
        ),
        (Some(old), Some(new)) => add(
            diff,
            ChangeLevel::Warning,
            "document",
            format!(
                "both documents are invalid: the previous document: {old}; the new document: {new}"
            ),
        ),
    }
}

fn validation_message(errors: Vec<ValidationError>) -> String {
    errors
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("; ")
}

fn add(diff: &mut Diff, level: ChangeLevel, path: impl Into<String>, detail: impl Into<String>) {
    diff.changes.push(Change {
        level,
        path: path.into(),
        detail: detail.into(),
    });
}

fn diff_info(diff: &mut Diff, old: &Info, new: &Info) {
    if old.title != new.title {
        add(diff, ChangeLevel::Info, "info.title", "API title changed");
    }
    if old.version != new.version {
        add(
            diff,
            ChangeLevel::NonBreaking,
            "info.version",
            "API version changed",
        );
    }
    if old.description != new.description {
        add(
            diff,
            ChangeLevel::Info,
            "info.description",
            "API description changed",
        );
    }
    if old.servers != new.servers {
        add(
            diff,
            ChangeLevel::NonBreaking,
            "info.servers",
            "API servers changed",
        );
    }
    if old.ext != new.ext {
        add(
            diff,
            ChangeLevel::Warning,
            "info.ext",
            "document metadata changed",
        );
    }
}

fn key_path(prefix: &str, id: &str) -> String {
    format!("{prefix}.{}", id.replace('%', "%25").replace('.', "%2E"))
}

fn diff_types(diff: &mut Diff, old: &Document, new: &Document) {
    let refused = refuses_required_read_only(new);
    for id in old.types.keys() {
        if !new.types.contains_key(id) {
            add(
                diff,
                ChangeLevel::Breaking,
                key_path("types", id),
                "type removed",
            );
        }
    }
    for (id, definition) in &new.types {
        let Some(previous) = old.types.get(id) else {
            add(
                diff,
                ChangeLevel::NonBreaking,
                key_path("types", id),
                "type added",
            );
            continue;
        };
        diff_type(diff, &refused, id, previous, definition);
    }
}

fn refuses_required_read_only(document: &Document) -> BTreeSet<String> {
    let mut refused = BTreeSet::new();
    for operation in &document.operations {
        let carriers = operation
            .query
            .iter()
            .chain(operation.path.iter().filter_map(|segment| match segment {
                crate::PathSegment::Param { ty, .. } => Some(ty),
                crate::PathSegment::Literal { .. } => None,
            }))
            .chain(
                operation
                    .body
                    .as_ref()
                    .filter(|body| body.content == ContentKind::Form)
                    .map(|body| &body.ty),
            );
        for ty in carriers {
            if let TypeRef::Named(named) = ty
                && crate::query::required_read_only(document, ty).is_some()
            {
                refused.insert(named.id.clone());
            }
        }
    }
    refused
}

fn diff_auth_schemes(diff: &mut Diff, old: &Document, new: &Document) {
    for id in old.auth_schemes.keys() {
        if !new.auth_schemes.contains_key(id) {
            add(
                diff,
                ChangeLevel::Breaking,
                key_path("auth_schemes", id),
                "authentication scheme removed",
            );
        }
    }
    for (id, scheme) in &new.auth_schemes {
        let path = key_path("auth_schemes", id);
        let Some(previous) = old.auth_schemes.get(id) else {
            add(
                diff,
                ChangeLevel::NonBreaking,
                &path,
                "authentication scheme added",
            );
            continue;
        };
        match (previous, scheme) {
            (AuthScheme::Bearer { format: old }, AuthScheme::Bearer { format: new }) => {
                if old != new {
                    add(
                        diff,
                        ChangeLevel::Info,
                        &path,
                        "bearer token format documentation changed",
                    );
                }
            }
            (
                AuthScheme::ApiKey {
                    location: old_location,
                    name: old_name,
                },
                AuthScheme::ApiKey { location, name },
            ) => {
                if old_location != location {
                    add(
                        diff,
                        ChangeLevel::Breaking,
                        &path,
                        "API key location changed",
                    );
                }
                if old_name != name {
                    add(diff, ChangeLevel::Breaking, &path, "API key name changed");
                }
            }
            (AuthScheme::CookieSession { cookie: old }, AuthScheme::CookieSession { cookie }) => {
                if old != cookie {
                    add(
                        diff,
                        ChangeLevel::Breaking,
                        &path,
                        "session cookie name changed",
                    );
                }
            }
            (AuthScheme::OAuth2 { flows: old }, AuthScheme::OAuth2 { flows }) => {
                diff_oauth_flows(diff, &path, old, flows);
            }
            (
                AuthScheme::Custom {
                    description: old,
                    ext: old_ext,
                },
                AuthScheme::Custom { description, ext },
            ) => {
                if old != description {
                    add(
                        diff,
                        ChangeLevel::Info,
                        &path,
                        "custom scheme description changed",
                    );
                }
                if old_ext != ext {
                    add(
                        diff,
                        ChangeLevel::Warning,
                        &path,
                        "custom scheme metadata changed",
                    );
                }
            }
            _ => add(
                diff,
                ChangeLevel::Breaking,
                &path,
                "authentication scheme kind changed",
            ),
        }
    }
}

fn diff_oauth_flows(diff: &mut Diff, path: &str, old: &[OAuthFlow], new: &[OAuthFlow]) {
    let old: BTreeMap<_, _> = old.iter().map(|flow| (flow.kind, flow)).collect();
    let new: BTreeMap<_, _> = new.iter().map(|flow| (flow.kind, flow)).collect();
    for kind in old.keys() {
        if !new.contains_key(kind) {
            add(
                diff,
                ChangeLevel::Breaking,
                format!("{path}.flows.{kind:?}"),
                "OAuth2 flow removed",
            );
        }
    }
    for (kind, flow) in &new {
        let flow_path = format!("{path}.flows.{kind:?}");
        let Some(previous) = old.get(kind) else {
            add(
                diff,
                ChangeLevel::NonBreaking,
                &flow_path,
                "OAuth2 flow added",
            );
            continue;
        };
        if previous.authorization_url != flow.authorization_url {
            add(
                diff,
                ChangeLevel::Breaking,
                &flow_path,
                "OAuth2 authorization URL changed",
            );
        }
        if previous.token_url != flow.token_url {
            add(
                diff,
                ChangeLevel::Breaking,
                &flow_path,
                "OAuth2 token URL changed",
            );
        }
        if previous.scopes != flow.scopes {
            add(
                diff,
                ChangeLevel::Info,
                &flow_path,
                "OAuth2 scope documentation changed",
            );
        }
        if previous.ext != flow.ext {
            add(
                diff,
                ChangeLevel::Warning,
                &flow_path,
                "OAuth2 flow metadata changed",
            );
        }
    }
}

/// a deprecation flip earns a `Warning`.
fn add_doc_change(diff: &mut Diff, path: &str, old: Option<&Doc>, new: Option<&Doc>, label: &str) {
    if old == new {
        return;
    }
    let (level, change) =
        if old.and_then(|doc| doc.deprecated) != new.and_then(|doc| doc.deprecated) {
            (ChangeLevel::Warning, "deprecation changed")
        } else {
            (ChangeLevel::Info, "documentation changed")
        };
    add(diff, level, path, format!("{label} {change}"));
}

fn diff_diagnostics(diff: &mut Diff, path: &str, old: &[Diagnostic], new: &[Diagnostic]) {
    let mut unmatched = old.iter().collect::<Vec<_>>();
    for diagnostic in new {
        let found = unmatched.iter().position(|previous| {
            previous.code == diagnostic.code && previous.message == diagnostic.message
        });
        let Some(index) = found else { continue };
        let previous = unmatched.remove(index);
        if previous.severity == diagnostic.severity {
            continue;
        }
        let (level, change) = if diagnostic.severity == DiagnosticSeverity::Error {
            (ChangeLevel::Breaking, "escalated")
        } else {
            (ChangeLevel::NonBreaking, "lowered")
        };
        add(
            diff,
            level,
            path,
            format!("error diagnostic severity {change}: {}", diagnostic.code),
        );
    }
    for previous in unmatched {
        if previous.severity == DiagnosticSeverity::Error {
            add(
                diff,
                ChangeLevel::NonBreaking,
                path,
                format!("error diagnostic removed: {}", previous.code),
            );
        }
    }
}

fn diff_type(diff: &mut Diff, refused: &BTreeSet<String>, id: &str, old: &TypeDef, new: &TypeDef) {
    let path = key_path("types", id);
    if old.name != new.name {
        add(diff, ChangeLevel::Breaking, &path, "type name changed");
    }
    if old.direction != new.direction {
        add(diff, ChangeLevel::Breaking, &path, "type direction changed");
    }
    if old.ext != new.ext {
        add(diff, ChangeLevel::Warning, &path, "type metadata changed");
    }
    match (&old.definition, &new.definition) {
        (
            TypeKind::Struct {
                fields: old_fields,
                flattened: old_flattened,
                openness: old_openness,
            },
            TypeKind::Struct {
                fields: new_fields,
                flattened: new_flattened,
                openness: new_openness,
            },
        ) => {
            diff_fields(
                diff,
                &path,
                new.direction,
                matches!(new.direction, Direction::Both)
                    || (new.direction == Direction::Input && refused.contains(id)),
                old_fields,
                new_fields,
            );
            if old_flattened != new_flattened {
                add(
                    diff,
                    ChangeLevel::Breaking,
                    &path,
                    "flattened types changed",
                );
            }
            if old_openness != new_openness {
                add(
                    diff,
                    ChangeLevel::Breaking,
                    &path,
                    "object openness policy changed",
                );
            }
        }
        (
            TypeKind::Enum {
                repr: old_repr,
                variants: old_variants,
            },
            TypeKind::Enum {
                repr: new_repr,
                variants: new_variants,
            },
        ) => {
            if old_repr != new_repr {
                add(
                    diff,
                    ChangeLevel::Breaking,
                    &path,
                    "enum representation changed",
                );
            }
            let old_variants: BTreeMap<_, _> = old_variants
                .iter()
                .map(|variant| (&variant.wire, variant))
                .collect();
            let new_variants: BTreeMap<_, _> = new_variants
                .iter()
                .map(|variant| (&variant.wire, variant))
                .collect();
            for wire in old_variants.keys() {
                if !new_variants.contains_key(wire) {
                    add(
                        diff,
                        ChangeLevel::Breaking,
                        format!("{path}.variants.{wire}"),
                        "enum variant removed",
                    );
                }
            }
            for (wire, variant) in new_variants {
                if let Some(previous) = old_variants.get(wire) {
                    if previous.payload != variant.payload {
                        add(
                            diff,
                            ChangeLevel::Breaking,
                            format!("{path}.variants.{wire}"),
                            "enum variant payload changed",
                        );
                    }
                    if previous.ident != variant.ident {
                        add(
                            diff,
                            ChangeLevel::Breaking,
                            format!("{path}.variants.{wire}"),
                            "enum variant identifier changed",
                        );
                    }
                    if previous.ext != variant.ext {
                        add(
                            diff,
                            ChangeLevel::Warning,
                            format!("{path}.variants.{wire}"),
                            "enum variant metadata changed",
                        );
                    }
                    add_doc_change(
                        diff,
                        &format!("{path}.variants.{wire}"),
                        previous.doc.as_ref(),
                        variant.doc.as_ref(),
                        "enum variant",
                    );
                    diff_diagnostics(
                        diff,
                        &format!("{path}.variants.{wire}"),
                        &previous.diagnostics,
                        &variant.diagnostics,
                    );
                } else {
                    let level = if new.direction == Direction::Input {
                        ChangeLevel::NonBreaking
                    } else {
                        ChangeLevel::Breaking
                    };
                    add(
                        diff,
                        level,
                        format!("{path}.variants.{wire}"),
                        "enum variant added",
                    );
                }
            }
        }
        (TypeKind::Newtype { inner: old_inner }, TypeKind::Newtype { inner: new_inner }) => {
            if old_inner != new_inner {
                add(diff, ChangeLevel::Breaking, &path, "newtype value changed");
            }
        }
        _ => add(diff, ChangeLevel::Breaking, &path, "type kind changed"),
    }
    add_doc_change(diff, &path, old.doc.as_ref(), new.doc.as_ref(), "type");
    diff_diagnostics(diff, &path, &old.diagnostics, &new.diagnostics);
}

fn diff_fields(
    diff: &mut Diff,
    owner: &str,
    direction: Direction,
    sent: bool,
    old: &[Field],
    new: &[Field],
) {
    let old: BTreeMap<_, _> = old.iter().map(|field| (&field.wire, field)).collect();
    let new: BTreeMap<_, _> = new.iter().map(|field| (&field.wire, field)).collect();
    for wire in old.keys() {
        if !new.contains_key(wire) {
            add(
                diff,
                ChangeLevel::Breaking,
                format!("{owner}.fields.{wire}"),
                "field removed",
            );
        }
    }
    for (wire, field) in new {
        let path = format!("{owner}.fields.{wire}");
        let Some(previous) = old.get(wire) else {
            let input_required = matches!(direction, Direction::Input | Direction::Both)
                && field.presence == Presence::Required
                && (sent || field.access != Some(crate::Access::ReadOnly));
            add(
                diff,
                if input_required {
                    ChangeLevel::Breaking
                } else {
                    ChangeLevel::NonBreaking
                },
                path,
                "field added",
            );
            continue;
        };
        if previous.ident != field.ident {
            add(
                diff,
                ChangeLevel::Breaking,
                &path,
                "field identifier changed",
            );
        }
        let input = matches!(direction, Direction::Input | Direction::Both)
            && field.access != Some(crate::Access::ReadOnly);
        let output = matches!(direction, Direction::Output | Direction::Both)
            && field.access != Some(crate::Access::WriteOnly);
        if previous.presence != field.presence {
            let breaking = (input
                && previous.presence == Presence::Optional
                && field.presence == Presence::Required)
                || (output
                    && previous.presence == Presence::Required
                    && field.presence == Presence::Optional);
            add(
                diff,
                if breaking {
                    ChangeLevel::Breaking
                } else {
                    ChangeLevel::NonBreaking
                },
                &path,
                "field presence changed",
            );
        }
        if previous.ty != field.ty {
            let widens_input_nullability = input
                && !output
                && !is_nullable(&previous.ty)
                && is_nullable(&field.ty)
                && inner_type(&previous.ty) == inner_type(&field.ty);
            add(
                diff,
                if widens_input_nullability {
                    ChangeLevel::NonBreaking
                } else {
                    ChangeLevel::Breaking
                },
                &path,
                "field type changed",
            );
        }
        if previous.access != field.access {
            add(diff, ChangeLevel::Breaking, &path, "field access changed");
        }
        if previous.default != field.default {
            add(
                diff,
                if input {
                    ChangeLevel::Breaking
                } else {
                    ChangeLevel::NonBreaking
                },
                &path,
                "field default changed",
            );
        }
        if previous.ext != field.ext {
            add(diff, ChangeLevel::Warning, &path, "field metadata changed");
        }
        add_doc_change(
            diff,
            &path,
            previous.doc.as_ref(),
            field.doc.as_ref(),
            "field",
        );
        diff_diagnostics(diff, &path, &previous.diagnostics, &field.diagnostics);
    }
}

fn is_nullable(ty: &TypeRef) -> bool {
    matches!(ty, TypeRef::Inline(InlineType::Nullable { .. }))
}

fn inner_type(ty: &TypeRef) -> &TypeRef {
    match ty {
        TypeRef::Inline(InlineType::Nullable { inner }) => inner,
        ty => ty,
    }
}

mod operations;

use operations::{diff_errors, diff_operations};

#[cfg(test)]
mod tests;
