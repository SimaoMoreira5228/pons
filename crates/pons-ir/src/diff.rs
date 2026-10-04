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

    pub validity: Validity,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Validity {
    pub previous: Vec<ValidationError>,

    pub new: Vec<ValidationError>,
}

impl Validity {
    pub fn previous_is_valid(&self) -> bool {
        self.previous.is_empty()
    }

    pub fn new_is_valid(&self) -> bool {
        self.new.is_empty()
    }
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

        let mut used = BTreeSet::new();
        diff_types(&mut diff, &mut used, &old, &new);
        diff_auth_schemes(&mut diff, &mut used, &old, &new);
        diff_errors(
            &mut diff,
            &mut used,
            "common_errors",
            &old.common_errors,
            &new.common_errors,
        );
        diff_operations(&mut diff, &mut used, &old, &new);
        diff
    }
}

fn diff_validity(diff: &mut Diff, old: &Document, new: &Document) {
    let previous = old.validate().err().unwrap_or_default();
    let new = new.validate().err().unwrap_or_default();
    match (previous.is_empty(), new.is_empty()) {
        (true, true) => {}
        (false, true) => add(
            diff,
            ChangeLevel::Info,
            "document",
            format!(
                "the previous document was invalid: {}",
                validation_message(&previous)
            ),
        ),
        (true, false) => add(
            diff,
            ChangeLevel::Breaking,
            "document",
            format!("the new document is invalid: {}", validation_message(&new)),
        ),
        (false, false) => add(
            diff,
            ChangeLevel::Warning,
            "document",
            format!(
                "both documents are invalid: the previous document: {}; the new document: {}",
                validation_message(&previous),
                validation_message(&new)
            ),
        ),
    }
    diff.validity = Validity { previous, new };
}

fn validation_message(errors: &[ValidationError]) -> String {
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

struct Path {
    readable: String,
    escaped: String,
}

impl Path {
    fn key(prefix: &str, key: &str) -> Self {
        Self {
            readable: format!("{prefix}.{key}"),
            escaped: format!("{prefix}.{}", escape(key)),
        }
    }

    fn child(&self, segment: &str, key: &str) -> Self {
        Self {
            readable: format!("{}.{segment}.{key}", self.readable),
            escaped: format!("{}.{segment}.{}", self.escaped, escape(key)),
        }
    }

    fn claim(&self, used: &mut BTreeSet<String>) -> String {
        if used.insert(self.readable.clone()) {
            return self.readable.clone();
        }
        let mut path = self.escaped.clone();
        let mut suffix = 2;
        while !used.insert(path.clone()) {
            path = format!("{}#{suffix}", self.readable);
            suffix += 1;
        }
        path
    }
}

fn escape(key: &str) -> String {
    crate::operation_path(&[vec![key.to_owned()]])
}

fn diff_types(diff: &mut Diff, used: &mut BTreeSet<String>, old: &Document, new: &Document) {
    let refused = refuses_required_read_only(new);
    for id in old.types.keys() {
        if !new.types.contains_key(id) {
            add(
                diff,
                ChangeLevel::Breaking,
                Path::key("types", id).claim(used),
                "type removed",
            );
        }
    }
    for (id, definition) in &new.types {
        let path = Path::key("types", id);
        let Some(previous) = old.types.get(id) else {
            add(
                diff,
                ChangeLevel::NonBreaking,
                path.claim(used),
                "type added",
            );
            continue;
        };
        diff_type(diff, used, &refused, &path, id, previous, definition);
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
            record_refusing(document, ty, &mut BTreeSet::new(), &mut refused);
        }
    }
    refused
}

fn record_refusing(
    document: &Document,
    ty: &TypeRef,
    visiting: &mut BTreeSet<String>,
    refused: &mut BTreeSet<String>,
) -> bool {
    let TypeRef::Named(named) = ty else {
        return false;
    };
    if named.variant.is_some() || !visiting.insert(named.id.clone()) {
        return false;
    }
    let refusing = match document.types.get(&named.id).map(|found| &found.definition) {
        Some(TypeKind::Struct {
            fields, flattened, ..
        }) => {
            fields.iter().any(|field| {
                field.presence == Presence::Required
                    && field.access == Some(crate::Access::ReadOnly)
            }) || flattened
                .iter()
                .any(|member| record_refusing(document, member, visiting, refused))
        }
        Some(TypeKind::Newtype { inner }) => record_refusing(document, inner, visiting, refused),
        _ => false,
    };
    visiting.remove(&named.id);
    if refusing {
        refused.insert(named.id.clone());
    }
    refusing
}

fn diff_auth_schemes(diff: &mut Diff, used: &mut BTreeSet<String>, old: &Document, new: &Document) {
    for id in old.auth_schemes.keys() {
        if !new.auth_schemes.contains_key(id) {
            add(
                diff,
                ChangeLevel::Breaking,
                Path::key("auth_schemes", id).claim(used),
                "authentication scheme removed",
            );
        }
    }
    for (id, scheme) in &new.auth_schemes {
        let owner = Path::key("auth_schemes", id);
        let Some(previous) = old.auth_schemes.get(id) else {
            add(
                diff,
                ChangeLevel::NonBreaking,
                owner.claim(used),
                "authentication scheme added",
            );
            continue;
        };
        let path = owner.claim(used);
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
                diff_oauth_flows(diff, used, &owner, old, flows);
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

fn diff_oauth_flows(
    diff: &mut Diff,
    used: &mut BTreeSet<String>,
    owner: &Path,
    old: &[OAuthFlow],
    new: &[OAuthFlow],
) {
    let old: BTreeMap<_, _> = old.iter().map(|flow| (flow.kind, flow)).collect();
    let new: BTreeMap<_, _> = new.iter().map(|flow| (flow.kind, flow)).collect();
    for kind in old.keys() {
        if !new.contains_key(kind) {
            add(
                diff,
                ChangeLevel::Breaking,
                owner.child("flows", &format!("{kind:?}")).claim(used),
                "OAuth2 flow removed",
            );
        }
    }
    for (kind, flow) in &new {
        let flow_path = owner.child("flows", &format!("{kind:?}")).claim(used);
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
        let found = unmatched
            .iter()
            .position(|previous| previous.code == diagnostic.code);
        let Some(index) = found else {
            if diagnostic.severity == DiagnosticSeverity::Error {
                add(
                    diff,
                    ChangeLevel::Breaking,
                    path,
                    format!("error diagnostic added: {}", diagnostic.code),
                );
            }
            continue;
        };
        let previous = unmatched.remove(index);
        if previous.severity != diagnostic.severity {
            let escalated = severity_rank(diagnostic.severity) > severity_rank(previous.severity);

            let level = if diagnostic.severity == DiagnosticSeverity::Error {
                ChangeLevel::Breaking
            } else {
                ChangeLevel::NonBreaking
            };
            add(
                diff,
                level,
                path,
                format!(
                    "diagnostic severity {}: {}",
                    if escalated { "escalated" } else { "lowered" },
                    diagnostic.code
                ),
            );
            continue;
        }
        if previous.message != diagnostic.message {
            add(
                diff,
                ChangeLevel::NonBreaking,
                path,
                format!("diagnostic message changed: {}", diagnostic.code),
            );
        }
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

const fn severity_rank(severity: DiagnosticSeverity) -> u8 {
    match severity {
        DiagnosticSeverity::Info => 0,
        DiagnosticSeverity::Warning => 1,
        DiagnosticSeverity::Error => 2,
    }
}

fn diff_type(
    diff: &mut Diff,
    used: &mut BTreeSet<String>,
    refused: &BTreeSet<String>,
    owner: &Path,
    id: &str,
    old: &TypeDef,
    new: &TypeDef,
) {
    let path = owner.claim(used);
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
                used,
                owner,
                new.direction,
                refused.contains(id),
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
                        owner.child("variants", wire).claim(used),
                        "enum variant removed",
                    );
                }
            }
            for (wire, variant) in new_variants {
                let variant_path = owner.child("variants", wire).claim(used);
                if let Some(previous) = old_variants.get(wire) {
                    if previous.payload != variant.payload {
                        add(
                            diff,
                            ChangeLevel::Breaking,
                            &variant_path,
                            "enum variant payload changed",
                        );
                    }
                    if previous.ident != variant.ident {
                        add(
                            diff,
                            ChangeLevel::Breaking,
                            &variant_path,
                            "enum variant identifier changed",
                        );
                    }
                    if previous.ext != variant.ext {
                        add(
                            diff,
                            ChangeLevel::Warning,
                            &variant_path,
                            "enum variant metadata changed",
                        );
                    }
                    add_doc_change(
                        diff,
                        &variant_path,
                        previous.doc.as_ref(),
                        variant.doc.as_ref(),
                        "enum variant",
                    );
                    diff_diagnostics(
                        diff,
                        &variant_path,
                        &previous.diagnostics,
                        &variant.diagnostics,
                    );
                } else {
                    let level = if new.direction == Direction::Input {
                        ChangeLevel::NonBreaking
                    } else {
                        ChangeLevel::Breaking
                    };
                    add(diff, level, variant_path, "enum variant added");
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
    used: &mut BTreeSet<String>,
    owner: &Path,
    direction: Direction,
    carrier: bool,
    old: &[Field],
    new: &[Field],
) {
    let sent = carrier || matches!(direction, Direction::Both);
    let old: BTreeMap<_, _> = old.iter().map(|field| (&field.wire, field)).collect();
    let new: BTreeMap<_, _> = new.iter().map(|field| (&field.wire, field)).collect();
    for wire in old.keys() {
        if !new.contains_key(wire) {
            add(
                diff,
                ChangeLevel::Breaking,
                owner.child("fields", wire).claim(used),
                "field removed",
            );
        }
    }
    for (wire, field) in new {
        let path = owner.child("fields", wire).claim(used);
        let read_only = field.access == Some(crate::Access::ReadOnly);
        let mut input = matches!(direction, Direction::Input | Direction::Both) || carrier;
        if read_only && !sent {
            input = false;
        }
        let output = matches!(direction, Direction::Output | Direction::Both)
            && field.access != Some(crate::Access::WriteOnly);
        let Some(previous) = old.get(wire) else {
            add(
                diff,
                if input && field.presence == Presence::Required {
                    ChangeLevel::Breaking
                } else {
                    ChangeLevel::NonBreaking
                },
                &path,
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
