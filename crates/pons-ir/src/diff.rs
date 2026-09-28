use std::collections::BTreeMap;

use crate::{
    Body, Direction, Document, ErrorResponse, Field, InlineType, Operation, Presence, TypeDef,
    TypeKind, TypeRef,
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
}

impl Diff {
    pub fn has_breaking_changes(&self) -> bool {
        self.changes
            .iter()
            .any(|change| change.level == ChangeLevel::Breaking)
    }
}

impl Document {
    pub fn diff(&self, newer: &Self) -> Diff {
        let mut old = self.clone();
        let mut new = newer.clone();
        old.canonicalize();
        new.canonicalize();

        let mut diff = Diff::default();
        diff_types(&mut diff, &old, &new);
        diff_auth_schemes(&mut diff, &old, &new);
        diff_errors(
            &mut diff,
            "common_errors",
            &old.common_errors,
            &new.common_errors,
        );
        diff_operations(&mut diff, &old, &new);
        diff
    }
}

fn add(diff: &mut Diff, level: ChangeLevel, path: impl Into<String>, detail: impl Into<String>) {
    diff.changes.push(Change {
        level,
        path: path.into(),
        detail: detail.into(),
    });
}

fn diff_types(diff: &mut Diff, old: &Document, new: &Document) {
    for id in old.types.keys() {
        if !new.types.contains_key(id) {
            add(
                diff,
                ChangeLevel::Breaking,
                format!("types.{id}"),
                "type removed",
            );
        }
    }
    for (id, definition) in &new.types {
        let Some(previous) = old.types.get(id) else {
            add(
                diff,
                ChangeLevel::NonBreaking,
                format!("types.{id}"),
                "type added",
            );
            continue;
        };
        diff_type(diff, id, previous, definition);
    }
}

fn diff_auth_schemes(diff: &mut Diff, old: &Document, new: &Document) {
    for id in old.auth_schemes.keys() {
        if !new.auth_schemes.contains_key(id) {
            add(
                diff,
                ChangeLevel::Breaking,
                format!("auth_schemes.{id}"),
                "authentication scheme removed",
            );
        }
    }
    for (id, scheme) in &new.auth_schemes {
        match old.auth_schemes.get(id) {
            Some(previous) if previous != scheme => add(
                diff,
                ChangeLevel::Breaking,
                format!("auth_schemes.{id}"),
                "authentication scheme changed",
            ),
            None => add(
                diff,
                ChangeLevel::NonBreaking,
                format!("auth_schemes.{id}"),
                "authentication scheme added",
            ),
            _ => {}
        }
    }
}

fn diff_type(diff: &mut Diff, id: &str, old: &TypeDef, new: &TypeDef) {
    let path = format!("types.{id}");
    if old.direction != new.direction {
        add(diff, ChangeLevel::Breaking, &path, "type direction changed");
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
            diff_fields(diff, &path, old.direction, old_fields, new_fields);
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
}

fn diff_fields(diff: &mut Diff, owner: &str, direction: Direction, old: &[Field], new: &[Field]) {
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
                && field.access != Some(crate::Access::ReadOnly);
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
            let breaking = if input {
                previous.presence == Presence::Optional && field.presence == Presence::Required
            } else {
                output
                    && previous.presence == Presence::Required
                    && field.presence == Presence::Optional
            };
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

fn diff_operations(diff: &mut Diff, old: &Document, new: &Document) {
    let old: BTreeMap<_, _> = old
        .operations
        .iter()
        .map(|operation| (&operation.id, operation))
        .collect();
    let new: BTreeMap<_, _> = new
        .operations
        .iter()
        .map(|operation| (&operation.id, operation))
        .collect();
    for id in old.keys() {
        if !new.contains_key(id) {
            add(
                diff,
                ChangeLevel::Breaking,
                operation_path(id),
                "operation removed",
            );
        }
    }
    for (id, operation) in new {
        let path = operation_path(id);
        let Some(previous) = old.get(id) else {
            add(diff, ChangeLevel::NonBreaking, path, "operation added");
            continue;
        };
        diff_operation(diff, &path, previous, operation);
    }
}

fn operation_path(id: &[Vec<String>]) -> String {
    format!(
        "operations.{}",
        id.iter()
            .map(|part| part.join("_"))
            .collect::<Vec<_>>()
            .join(".")
    )
}

fn diff_operation(diff: &mut Diff, path: &str, old: &Operation, new: &Operation) {
    if old.method != new.method {
        add(diff, ChangeLevel::Breaking, path, "HTTP method changed");
    }
    if old.semantics.idempotent != new.semantics.idempotent {
        add(
            diff,
            ChangeLevel::Breaking,
            format!("{path}.semantics"),
            "operation idempotency changed",
        );
    }
    if route_shape(&old.path) != route_shape(&new.path) {
        add(diff, ChangeLevel::NonBreaking, path, "route URL changed");
    }
    if old.path != new.path {
        let old_params: Vec<_> = old
            .path
            .iter()
            .filter_map(|segment| match segment {
                crate::PathSegment::Param { param, ty } => Some((param, ty)),
                crate::PathSegment::Literal { .. } => None,
            })
            .collect();
        let new_params: Vec<_> = new
            .path
            .iter()
            .filter_map(|segment| match segment {
                crate::PathSegment::Param { param, ty } => Some((param, ty)),
                crate::PathSegment::Literal { .. } => None,
            })
            .collect();
        if old_params.len() != new_params.len() {
            add(
                diff,
                ChangeLevel::Breaking,
                format!("{path}.path"),
                "path parameter count changed",
            );
        }
        for (index, ((old_name, before), (new_name, after))) in
            old_params.iter().zip(&new_params).enumerate()
        {
            if before != after {
                add(
                    diff,
                    ChangeLevel::Breaking,
                    format!("{path}.path[{index}]"),
                    "path parameter type changed",
                );
            }
            if old_name != new_name {
                add(
                    diff,
                    ChangeLevel::Breaking,
                    format!("{path}.path[{index}]"),
                    "path parameter name changed",
                );
            }
        }
    }
    if old.query != new.query {
        add(
            diff,
            ChangeLevel::Breaking,
            format!("{path}.query"),
            "query type changed",
        );
    }
    diff_headers(diff, path, &old.headers, &new.headers);
    diff_body(diff, path, old.body.as_ref(), new.body.as_ref());
    if old.responses != new.responses {
        diff_responses(diff, path, old, new);
    }
    diff_errors(diff, path, &old.errors, &new.errors);
    if old.errors_inherit != new.errors_inherit {
        add(
            diff,
            ChangeLevel::Breaking,
            format!("{path}.errors"),
            "inherited errors changed",
        );
    }
    if old.auth != new.auth {
        add(
            diff,
            ChangeLevel::Breaking,
            format!("{path}.auth"),
            "authentication requirement changed",
        );
    }
    if old.provides != new.provides {
        add(
            diff,
            ChangeLevel::Breaking,
            format!("{path}.provides"),
            "credential provision changed",
        );
    }
    if old.stream != new.stream {
        add(
            diff,
            ChangeLevel::Breaking,
            format!("{path}.stream"),
            "stream definition changed",
        );
    }
}

fn route_shape(path: &[crate::PathSegment]) -> Vec<String> {
    path.iter()
        .map(|segment| match segment {
            crate::PathSegment::Literal { lit } => lit.clone(),
            crate::PathSegment::Param { param, .. } => format!("{{{param}}}"),
        })
        .collect()
}

fn diff_body(diff: &mut Diff, path: &str, old: Option<&Body>, new: Option<&Body>) {
    let (Some(old), Some(new)) = (old, new) else {
        if old.is_some() || new.is_some() {
            add(
                diff,
                ChangeLevel::Breaking,
                format!("{path}.body"),
                "request body changed",
            );
        }
        return;
    };
    if old.ty != new.ty
        || old.content != new.content
        || old.media_type != new.media_type
        || old.ext != new.ext
    {
        add(
            diff,
            ChangeLevel::Breaking,
            format!("{path}.body"),
            "request body changed",
        );
    } else if old.required != new.required {
        add(
            diff,
            if old.required {
                ChangeLevel::NonBreaking
            } else {
                ChangeLevel::Breaking
            },
            format!("{path}.body"),
            if old.required {
                "request body became optional"
            } else {
                "request body became required"
            },
        );
    }
}

fn diff_headers(
    diff: &mut Diff,
    path: &str,
    old: &[crate::HeaderParameter],
    new: &[crate::HeaderParameter],
) {
    let old: BTreeMap<String, _> = old
        .iter()
        .map(|header| (header.wire.to_ascii_lowercase(), header))
        .collect();
    let new: BTreeMap<String, _> = new
        .iter()
        .map(|header| (header.wire.to_ascii_lowercase(), header))
        .collect();
    for (key, header) in &old {
        if !new.contains_key(key) {
            add(
                diff,
                ChangeLevel::Breaking,
                format!("{path}.headers.{}", header.wire),
                "request header removed",
            );
        }
    }
    for (key, header) in &new {
        let header_path = format!("{path}.headers.{}", header.wire);
        match old.get(key) {
            Some(previous) if previous.ty != header.ty => add(
                diff,
                ChangeLevel::Breaking,
                header_path,
                "request header type changed",
            ),
            Some(previous)
                if previous.presence != header.presence
                    && previous.presence == Presence::Optional =>
            {
                add(
                    diff,
                    ChangeLevel::Breaking,
                    header_path,
                    "optional request header became required",
                )
            }
            Some(previous) if previous.presence != header.presence => add(
                diff,
                ChangeLevel::NonBreaking,
                header_path,
                "request header presence changed",
            ),
            Some(previous) if previous.ident != header.ident => add(
                diff,
                ChangeLevel::Breaking,
                header_path,
                "request header identifier changed",
            ),
            None if header.presence == Presence::Required => add(
                diff,
                ChangeLevel::Breaking,
                header_path,
                "required request header added",
            ),
            None => add(
                diff,
                ChangeLevel::NonBreaking,
                header_path,
                "optional request header added",
            ),
            _ => {}
        }
    }
}

fn diff_responses(diff: &mut Diff, path: &str, old: &Operation, new: &Operation) {
    let old: BTreeMap<_, _> = old
        .responses
        .iter()
        .map(|response| (response.status, response))
        .collect();
    let new: BTreeMap<_, _> = new
        .responses
        .iter()
        .map(|response| (response.status, response))
        .collect();
    for status in old.keys() {
        if !new.contains_key(status) {
            add(
                diff,
                ChangeLevel::Breaking,
                format!("{path}.responses.{status}"),
                "response removed",
            );
        }
    }
    for (status, response) in new {
        match old.get(&status) {
            Some(previous) if !previous.equivalent_ignoring_header_case(response) => add(
                diff,
                ChangeLevel::Breaking,
                format!("{path}.responses.{status}"),
                "response changed",
            ),
            None => add(
                diff,
                ChangeLevel::Warning,
                format!("{path}.responses.{status}"),
                "response status added",
            ),
            _ => {}
        }
    }
}

fn diff_errors(diff: &mut Diff, path: &str, old: &[ErrorResponse], new: &[ErrorResponse]) {
    let old: BTreeMap<_, _> = old
        .iter()
        .map(|error| ((&error.status, &error.name), error))
        .collect();
    let new: BTreeMap<_, _> = new
        .iter()
        .map(|error| ((&error.status, &error.name), error))
        .collect();
    for key in old.keys() {
        if !new.contains_key(key) {
            add(
                diff,
                ChangeLevel::Breaking,
                format!("{path}.errors.{}", key.1.join("_")),
                "error removed",
            );
        }
    }
    for (key, error) in new {
        match old.get(&key) {
            Some(previous) if !previous.equivalent_ignoring_header_case(error) => add(
                diff,
                ChangeLevel::Breaking,
                format!("{path}.errors.{}", key.1.join("_")),
                "error definition changed",
            ),
            None => add(
                diff,
                ChangeLevel::Breaking,
                format!("{path}.errors.{}", key.1.join("_")),
                "error status added",
            ),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> serde_json::Value {
        serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap()
    }

    fn level(old: &Document, changed: serde_json::Value, path: &str) -> ChangeLevel {
        let new: Document = serde_json::from_value(changed).unwrap();
        old.diff(&new)
            .changes
            .into_iter()
            .find(|change| change.path == path)
            .unwrap()
            .level
    }

    #[test]
    fn equivalent_orderings_have_no_diff() {
        let mut old: Document = serde_json::from_value(fixture()).unwrap();
        let event = old
            .operations
            .iter_mut()
            .find(|operation| operation.stream.is_some())
            .unwrap();
        event.auth.alternatives = vec![vec!["session".into()], vec!["tenant".into()]];
        let mut second_header = event.responses[0].headers[0].clone();
        second_header.wire = "X-Second".into();
        event.responses[0].headers.push(second_header);
        let flow = |kind| crate::OAuthFlow {
            kind,
            authorization_url: None,
            token_url: Some("https://identity.example.test/token".into()),
            scopes: Default::default(),
            ext: Default::default(),
        };
        old.auth_schemes.insert(
            "oauth".into(),
            crate::AuthScheme::OAuth2 {
                flows: vec![
                    flow(crate::OAuthFlowKind::Password),
                    flow(crate::OAuthFlowKind::ClientCredentials),
                ],
            },
        );

        let mut new = old.clone();
        new.operations.reverse();
        let event = &mut new.operations[0];
        event.auth.alternatives.reverse();
        event.responses[0].headers.reverse();
        let crate::AuthScheme::OAuth2 { flows } = new.auth_schemes.get_mut("oauth").unwrap() else {
            unreachable!()
        };
        flows.reverse();
        old.validate().unwrap();
        new.validate().unwrap();

        assert_eq!(old.diff(&new), Diff::default());
    }

    #[test]
    fn classifies_added_fields_and_output_enum_variants() {
        let old: Document = serde_json::from_value(fixture()).unwrap();
        let mut changed = fixture();
        changed["types"]["LoginRequest"]["fields"]
            .as_array_mut()
            .unwrap()
            .push(serde_json::json!({
                "wire": "remember",
                "ident": ["remember"],
                "ty": {"kind": "bool"},
                "presence": "optional"
            }));
        assert_eq!(
            level(&old, changed, "types.LoginRequest.fields.remember"),
            ChangeLevel::NonBreaking
        );

        let mut changed = fixture();
        changed["types"]["LoginError"]["variants"]
            .as_array_mut()
            .unwrap()
            .push(serde_json::json!({
                "wire": "expired",
                "ident": ["expired"],
                "payload": {"kind": "unit"}
            }));
        let new: Document = serde_json::from_value(changed).unwrap();
        let diff = old.diff(&new);
        assert!(diff.has_breaking_changes());
        assert_eq!(
            diff.changes
                .iter()
                .find(|change| change.path == "types.LoginError.variants.expired")
                .unwrap()
                .level,
            ChangeLevel::Breaking
        );
    }

    #[test]
    fn object_openness_changes_are_breaking() {
        let old: Document = serde_json::from_value(fixture()).unwrap();
        let mut changed = fixture();
        changed["types"]["User"]["openness"] = serde_json::json!("reject");
        assert_eq!(level(&old, changed, "types.User"), ChangeLevel::Breaking);
    }

    #[test]
    fn idempotency_changes_are_breaking() {
        let old: Document = serde_json::from_value(fixture()).unwrap();
        let mut new = old.clone();
        new.operations[0].semantics.idempotent = true;
        let change = old
            .diff(&new)
            .changes
            .into_iter()
            .find(|change| change.path == "operations.auth.login.semantics")
            .unwrap();
        assert_eq!(change.level, ChangeLevel::Breaking);
        assert_eq!(change.detail, "operation idempotency changed");
    }

    #[test]
    fn route_changes_with_stable_operation_identity_are_nonbreaking() {
        let old: Document = serde_json::from_value(fixture()).unwrap();
        let mut changed = fixture();
        changed["operations"][0]["path"][1]["lit"] = "sign-in".into();
        assert_eq!(
            level(&old, changed, "operations.auth.login"),
            ChangeLevel::NonBreaking
        );
    }

    #[test]
    fn path_parameter_type_changes_are_breaking() {
        let old: Document = serde_json::from_value(fixture()).unwrap();
        let mut changed = fixture();
        changed["operations"][1]["path"][1]["ty"] =
            serde_json::json!({"kind": "int", "bits": 32, "signed": true, "wire": "number"});
        let new: Document = serde_json::from_value(changed).unwrap();
        let diff = old.diff(&new);
        assert!(diff.changes.iter().any(|change| {
            change.path == "operations.events.watch.path[0]"
                && change.level == ChangeLevel::Breaking
        }));
        assert!(!diff.changes.iter().any(|change| {
            change.path == "operations.events.watch" && change.detail == "route URL changed"
        }));
    }

    #[test]
    fn new_errors_are_breaking() {
        let old: Document = serde_json::from_value(fixture()).unwrap();
        let mut operation = old.clone();
        let mut operation_error = operation.operations[0].errors[0].clone();
        operation_error.status = crate::ErrorStatus::Exact(418);
        operation_error.name = vec!["teapot".into()];
        operation.operations[0].errors.push(operation_error);
        let diff = old.diff(&operation);
        assert!(diff.has_breaking_changes());
        assert!(diff.changes.iter().any(|change| {
            change.path == "operations.auth.login.errors.teapot"
                && change.level == ChangeLevel::Breaking
        }));

        let mut common = old.clone();
        let mut common_error = common.operations[0].errors[0].clone();
        common_error.status = crate::ErrorStatus::Exact(503);
        common_error.name = vec!["unavailable".into()];
        common.common_errors.push(common_error);
        let diff = old.diff(&common);
        assert!(diff.has_breaking_changes());
        assert!(diff.changes.iter().any(|change| {
            change.path == "common_errors.errors.unavailable"
                && change.level == ChangeLevel::Breaking
        }));
    }

    #[test]
    fn request_body_requiredness_is_classified() {
        let old: Document = serde_json::from_value(fixture()).unwrap();
        let mut optional = old.clone();
        optional.operations[0].body.as_mut().unwrap().required = false;
        let change = old
            .diff(&optional)
            .changes
            .into_iter()
            .find(|change| change.path == "operations.auth.login.body")
            .unwrap();
        assert_eq!(change.level, ChangeLevel::NonBreaking);
        assert_eq!(change.detail, "request body became optional");

        let mut required = optional.clone();
        required.operations[0].body.as_mut().unwrap().required = true;
        assert_eq!(
            optional
                .diff(&required)
                .changes
                .into_iter()
                .find(|change| change.path == "operations.auth.login.body")
                .unwrap()
                .level,
            ChangeLevel::Breaking
        );

        let mut media = old.clone();
        media.operations[0].body.as_mut().unwrap().media_type =
            Some("application/vnd.acme+json".into());
        assert_eq!(
            old.diff(&media)
                .changes
                .into_iter()
                .find(|change| change.path == "operations.auth.login.body")
                .unwrap()
                .level,
            ChangeLevel::Breaking
        );
    }
}
