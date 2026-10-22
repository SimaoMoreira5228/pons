use std::collections::BTreeSet;

use pons_ir::{
    Access, AuthScheme, ContentKind, Document, ErrorResponse, ErrorStatus, FieldDefault,
    InlineType, Method, ObjectOpenness, Operation, PathSegment, Presence, Response, ResponseStatus,
    TypeKind, TypeRef, VariantPayload, query_fields, validate_value,
};
use serde_json::{Map, Value};

use crate::example::Examples;
use crate::mutate;

use super::{
    content_name, default_content_type, header_text, join_path, json_content, json_media,
    percent_encode, render_violations, scalar_text,
};

pub(super) struct Plan<'a> {
    pub(super) operation: &'a Operation,
    pub(super) success: &'a Response,
    pub(super) status: u16,
    pub(super) valid: RequestPlan,
    pub(super) invalid: Option<RequestPlan>,
    pub(super) invalid_note: String,
}

pub(super) struct RequestPlan {
    method: Method,
    path: String,
    query: Vec<(String, String)>,
    headers: Vec<(String, String)>,
    body: Option<(String, String)>,
}

impl RequestPlan {
    pub(super) fn request(&self) -> crate::live::Request<'_> {
        crate::live::Request {
            method: self.method,
            path: &self.path,
            query: &self.query,
            headers: &self.headers,
            body: self
                .body
                .as_ref()
                .map(|(content_type, text)| crate::live::Body { content_type, text }),
        }
    }
}

pub(super) fn plan<'a>(
    document: &'a Document,
    operation: &'a Operation,
) -> Result<Plan<'a>, String> {
    if operation.stream.is_some() {
        return Err("streams are not exercised".into());
    }
    if operation.provides.is_some() {
        return Err("the operation provides a credential".into());
    }
    if let Some(reason) = authentication(document, operation) {
        return Err(reason);
    }
    if let Some(body) = &operation.body {
        if !json_content(body.content) {
            return Err(format!(
                "the request body is {} content",
                content_name(body.content)
            ));
        }
        if body
            .media
            .as_ref()
            .is_some_and(|media| !json_media(&media.to_string()))
        {
            return Err("the request body uses a custom media type".into());
        }
    }
    let success = success_response(operation)
        .ok_or_else(|| "no exact success response with JSON or empty content".to_owned())?;
    let status = match success.status {
        ResponseStatus::Exact(status) => status,
        _ => unreachable!("the selected success response has an exact status"),
    };
    if reachable_opaque(document, operation, success) {
        return Err("an opaque type is reachable".into());
    }

    let mut examples = Examples::new(document);

    let mut path_parts = Vec::new();
    for segment in &operation.path {
        match segment {
            PathSegment::Literal { lit } => path_parts.push(lit.clone()),
            PathSegment::Param { param, ty } => {
                let value = examples
                    .value_in(ty, pons_ir::Direction::Input)
                    .map_err(|gap| {
                        format!("cannot generate path parameter {param}: {}", gap.reason())
                    })?;
                validate_value(document, ty, pons_ir::Direction::Input, &value).map_err(
                    |errors| {
                        format!(
                            "generated path parameter {param} is invalid: {}",
                            render_violations(&errors)
                        )
                    },
                )?;
                let text = scalar_text(&value).ok_or_else(|| {
                    format!("path parameter {param} is not a scalar this runner can encode")
                })?;
                path_parts.push(percent_encode(&text));
            }
        }
    }

    let mut query = Vec::new();
    let mut query_object = Map::new();
    if let Some(query_ty) = &operation.query {
        let fields = query_fields(document, query_ty).map_err(|rejection| rejection.message())?;
        for field in fields {
            if !crosses(field, pons_ir::Direction::Input) {
                continue;
            }
            let value = match &field.default {
                FieldDefault::Value(value) => value.clone(),
                FieldDefault::Unspecified => {
                    match examples.value_in(&field.ty, pons_ir::Direction::Input) {
                        Ok(value) => value,
                        Err(_gap) if field.presence == Presence::Optional => continue,
                        Err(gap) => {
                            return Err(format!(
                                "cannot generate query field {}: {}",
                                field.wire,
                                gap.reason()
                            ));
                        }
                    }
                }
            };
            if value.is_null() && field.presence == Presence::Optional {
                continue;
            }
            if let Err(errors) =
                validate_value(document, &field.ty, pons_ir::Direction::Input, &value)
            {
                if field.presence == Presence::Optional {
                    continue;
                }
                return Err(format!(
                    "generated query field {} is invalid: {}",
                    field.wire,
                    render_violations(&errors)
                ));
            }
            let Some(text) = scalar_text(&value) else {
                if field.presence == Presence::Optional {
                    continue;
                }
                return Err(format!(
                    "query field {} is not a scalar this runner can encode",
                    field.wire
                ));
            };
            query.push((field.wire.clone(), text));
            query_object.insert(field.wire.clone(), value);
        }
        validate_value(
            document,
            query_ty,
            pons_ir::Direction::Input,
            &Value::Object(query_object),
        )
        .map_err(|errors| format!("generated query is invalid: {}", render_violations(&errors)))?;
    }

    let mut headers = Vec::new();
    for header in &operation.headers {
        let value = match examples.value_in(&header.ty, pons_ir::Direction::Input) {
            Ok(value) => value,
            Err(gap) if header.presence == Presence::Optional => {
                let _ = gap;
                continue;
            }
            Err(gap) => {
                return Err(format!(
                    "cannot generate header {}: {}",
                    header.wire,
                    gap.reason()
                ));
            }
        };
        if value.is_null() && header.presence == Presence::Optional {
            continue;
        }
        if let Err(errors) = validate_value(document, &header.ty, pons_ir::Direction::Input, &value)
        {
            if header.presence == Presence::Optional {
                continue;
            }
            return Err(format!(
                "generated header {} is invalid: {}",
                header.wire,
                render_violations(&errors)
            ));
        }
        let Some(text) = header_text(&value) else {
            if header.presence == Presence::Optional {
                continue;
            }
            return Err(format!(
                "header {} is not a scalar this runner can encode",
                header.wire
            ));
        };
        headers.push((header.wire.clone(), text));
    }

    let mut body = None;
    let mut body_value = None;
    if let Some(declared) = &operation.body {
        let value = examples
            .value_in(&declared.ty, pons_ir::Direction::Input)
            .map_err(|gap| format!("cannot generate a request body: {}", gap.reason()))?;
        validate_value(document, &declared.ty, pons_ir::Direction::Input, &value).map_err(
            |errors| {
                format!(
                    "generated request body is invalid: {}",
                    render_violations(&errors)
                )
            },
        )?;
        let text = serde_json::to_string(&value)
            .map_err(|error| format!("cannot write the request body: {error}"))?;
        let content_type = declared
            .media
            .as_ref()
            .map(ToString::to_string)
            .unwrap_or_else(|| default_content_type(declared.content).to_owned());
        body = Some((content_type, text));
        body_value = Some(value);
    }

    let valid = RequestPlan {
        method: operation.method,
        path: join_path(&path_parts),
        query,
        headers,
        body,
    };

    let mut invalid = None;
    let mut invalid_note = if declared_errors(document, operation).is_empty() {
        "the operation declares no error status".to_owned()
    } else {
        "no required field to mutate".to_owned()
    };
    if !declared_errors(document, operation).is_empty() {
        if let (Some(body), Some(value)) = (&operation.body, &body_value)
            && let Some(mutation) =
                mutate::body(document, &body.ty, pons_ir::Direction::Input, value)
        {
            let mut object = value.as_object().cloned().unwrap_or_default();
            object.insert(mutation.wire, mutation.value);
            let text = serde_json::to_string(&Value::Object(object))
                .map_err(|error| format!("cannot write the invalid request body: {error}"))?;
            let mut plan = RequestPlan {
                method: valid.method,
                path: valid.path.clone(),
                query: valid.query.clone(),
                headers: valid.headers.clone(),
                body: None,
            };
            plan.body = Some((
                valid
                    .body
                    .as_ref()
                    .map(|(content_type, _)| content_type.clone())
                    .unwrap_or_else(|| "application/json".into()),
                text,
            ));
            invalid_note = mutation.detail;
            invalid = Some(plan);
        }
        if invalid.is_none() {
            invalid = path_mutation(document, operation, &path_parts, &valid);
            if invalid.is_some() {
                invalid_note = "mutated a path parameter".into();
            }
        }
        if invalid.is_none() {
            invalid = query_mutation(document, operation, &valid);
            if invalid.is_some() {
                invalid_note = "mutated a query field".into();
            }
        }
        if invalid.is_none() {
            invalid = header_mutation(document, operation, &valid);
            if invalid.is_some() {
                invalid_note = "mutated a request header".into();
            }
        }
    }

    Ok(Plan {
        operation,
        success,
        status,
        valid,
        invalid,
        invalid_note,
    })
}

fn success_response(operation: &Operation) -> Option<&Response> {
    operation
        .responses
        .iter()
        .filter(|response| {
            matches!(
                response.status,
                ResponseStatus::Exact(status) if (200..300).contains(&status)
            )
        })
        .filter(|response| {
            matches!(
                response.content,
                ContentKind::Json | ContentKind::ProblemJson | ContentKind::Empty
            )
        })
        .min_by_key(|response| match response.status {
            ResponseStatus::Exact(status) => status,
            _ => u16::MAX,
        })
}

fn authentication(document: &Document, operation: &Operation) -> Option<String> {
    if operation.auth.alternatives.is_empty() || operation.auth.optional {
        return None;
    }
    let schemes = operation
        .auth
        .alternatives
        .iter()
        .flatten()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    let named = schemes.iter().copied().collect::<Vec<_>>().join(", ");
    let custom = schemes.iter().any(|scheme| {
        matches!(
            document.auth_schemes.get(*scheme),
            Some(AuthScheme::Custom { .. })
        )
    });
    if custom {
        Some(format!("custom authentication is required ({named})"))
    } else {
        Some(format!("authentication is required ({named})"))
    }
}

pub(crate) fn crosses(field: &pons_ir::Field, direction: pons_ir::Direction) -> bool {
    !matches!(
        (direction, field.access),
        (pons_ir::Direction::Input, Some(Access::ReadOnly))
            | (pons_ir::Direction::Output, Some(Access::WriteOnly))
    )
}

fn path_mutation(
    document: &Document,
    operation: &Operation,
    parts: &[String],
    valid: &RequestPlan,
) -> Option<RequestPlan> {
    for (index, segment) in operation.path.iter().enumerate() {
        let PathSegment::Param { ty, .. } = segment else {
            continue;
        };
        let Some(text) = mutate::invalid_text(document, ty) else {
            continue;
        };
        let mut parts = parts.to_vec();
        parts[index] = percent_encode(&text);
        return Some(RequestPlan {
            method: valid.method,
            path: join_path(&parts),
            query: valid.query.clone(),
            headers: valid.headers.clone(),
            body: valid.body.clone(),
        });
    }
    None
}

fn query_mutation(
    document: &Document,
    operation: &Operation,
    valid: &RequestPlan,
) -> Option<RequestPlan> {
    let query_ty = operation.query.as_ref()?;
    let fields = query_fields(document, query_ty).ok()?;
    for field in fields {
        if field.presence != Presence::Required {
            continue;
        }
        let Some(text) = mutate::invalid_text(document, &field.ty) else {
            continue;
        };
        let mut plan = RequestPlan {
            method: valid.method,
            path: valid.path.clone(),
            query: valid.query.clone(),
            headers: valid.headers.clone(),
            body: valid.body.clone(),
        };
        let pair = plan
            .query
            .iter_mut()
            .find(|(wire, _)| *wire == field.wire)?;
        pair.1 = text;
        return Some(plan);
    }
    None
}

fn header_mutation(
    document: &Document,
    operation: &Operation,
    valid: &RequestPlan,
) -> Option<RequestPlan> {
    for header in &operation.headers {
        if header.presence != Presence::Required {
            continue;
        }
        let Some(text) = mutate::invalid_text(document, &header.ty) else {
            continue;
        };
        let mut plan = RequestPlan {
            method: valid.method,
            path: valid.path.clone(),
            query: valid.query.clone(),
            headers: valid.headers.clone(),
            body: valid.body.clone(),
        };
        let pair = plan
            .headers
            .iter_mut()
            .find(|(wire, _)| *wire == header.wire)?;
        pair.1 = text;
        return Some(plan);
    }
    None
}

fn reachable_opaque(document: &Document, operation: &Operation, success: &Response) -> bool {
    let mut search = OpaqueSearch {
        document,
        seen: BTreeSet::new(),
        found: false,
    };
    for segment in &operation.path {
        if let PathSegment::Param { ty, .. } = segment {
            search.visit(ty);
        }
    }
    if let Some(query) = &operation.query {
        search.visit(query);
    }
    for header in &operation.headers {
        search.visit(&header.ty);
    }
    if let Some(body) = &operation.body {
        search.visit(&body.ty);
    }
    if let Some(body) = &success.body {
        search.visit(body);
    }
    for header in &success.headers {
        search.visit(&header.ty);
    }
    for error in declared_errors(document, operation) {
        if let Some(body) = &error.body {
            search.visit(body);
        }
        for header in &error.headers {
            search.visit(&header.ty);
        }
    }
    search.found
}

struct OpaqueSearch<'a> {
    document: &'a Document,
    seen: BTreeSet<(String, Option<String>)>,
    found: bool,
}

impl OpaqueSearch<'_> {
    fn visit(&mut self, ty: &TypeRef) {
        if self.found {
            return;
        }
        match ty {
            TypeRef::Inline(InlineType::Opaque { .. }) => self.found = true,
            TypeRef::Inline(InlineType::Nullable { inner }) => self.visit(inner),
            TypeRef::Inline(InlineType::Array { item, .. }) => self.visit(item),
            TypeRef::Inline(InlineType::Map { key, value }) => {
                self.visit(key);
                self.visit(value);
            }
            TypeRef::Inline(InlineType::Tuple { items }) => {
                for item in items {
                    self.visit(item);
                }
            }
            TypeRef::Inline(_) => {}
            TypeRef::Named(named) => {
                if !self.seen.insert((named.id.clone(), named.variant.clone())) {
                    return;
                }
                let Some(definition) = self.document.types.get(&named.id) else {
                    return;
                };
                match &definition.definition {
                    TypeKind::Newtype { inner } => self.visit(inner),
                    TypeKind::Struct {
                        fields,
                        flattened,
                        openness,
                    } => {
                        for field in fields {
                            self.visit(&field.ty);
                        }
                        for flat in flattened {
                            self.visit(flat);
                        }
                        if let ObjectOpenness::PreserveTyped { value } = openness {
                            self.visit(value);
                        }
                    }
                    TypeKind::Enum { variants, .. } => {
                        for variant in variants {
                            if named
                                .variant
                                .as_deref()
                                .is_some_and(|selected| selected != variant.wire)
                            {
                                continue;
                            }
                            self.payload(&variant.payload);
                        }
                    }
                }
            }
        }
    }

    fn payload(&mut self, payload: &VariantPayload) {
        match payload {
            VariantPayload::Unit => {}
            VariantPayload::Newtype { ty } => self.visit(ty),
            VariantPayload::Tuple { items } => {
                for item in items {
                    self.visit(item);
                }
            }
            VariantPayload::Struct { fields } => {
                for field in fields {
                    self.visit(&field.ty);
                }
            }
        }
    }
}

pub(super) fn declared_errors<'a>(
    document: &'a Document,
    operation: &'a Operation,
) -> Vec<&'a ErrorResponse> {
    let mut errors = operation.errors.iter().collect::<Vec<_>>();
    if operation.errors_inherit {
        errors.extend(document.common_errors.iter());
    }
    errors
}

pub(super) fn matching_error<'a>(
    errors: &[&'a ErrorResponse],
    status: u16,
) -> Option<&'a ErrorResponse> {
    errors
        .iter()
        .copied()
        .find(|error| matches!(error.status, ErrorStatus::Exact(declared) if declared == status))
        .or_else(|| {
            errors.iter().copied().find(|error| {
                matches!(error.status, ErrorStatus::Class(class) if status / 100 == u16::from(class))
            })
        })
        .or_else(|| {
            errors
                .iter()
                .copied()
                .find(|error| error.status == ErrorStatus::Default)
        })
}
