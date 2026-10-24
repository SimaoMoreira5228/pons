use std::collections::BTreeSet;

use serde_json::Value;

use crate::operation::{
    ApiKeyLocation, AuthScheme, Operation, PathSegment, ResponseHeader, StreamKind,
};
use crate::types::{
    Access, Diagnostic, Direction, Field, InlineType, IntegerWire, ObjectOpenness, TypeKind,
    TypeRef, VariantPayload,
};
use crate::violation::direction_name;

use super::{Document, ValidationError};

#[derive(Clone, Copy)]
pub(super) enum Boundary {
    Request,
    Response,
}

impl Boundary {
    pub(super) fn refuses(self, direction: Direction) -> bool {
        matches!(
            (self, direction),
            (Self::Request, Direction::Output) | (Self::Response, Direction::Input)
        )
    }

    fn crossing(self) -> &'static str {
        match self {
            Self::Request => "input or both",
            Self::Response => "output or both",
        }
    }
}

pub(super) fn field_crosses(field: &Field, boundary: Boundary) -> bool {
    !matches!(
        (boundary, field.access),
        (Boundary::Request, Some(Access::ReadOnly)) | (Boundary::Response, Some(Access::WriteOnly))
    )
}

fn carrier_key(location: ApiKeyLocation, name: &str) -> String {
    match location {
        ApiKeyLocation::Header => name.to_ascii_lowercase(),
        ApiKeyLocation::Query | ApiKeyLocation::Cookie => name.to_owned(),
    }
}

fn check_field_extensions(owner: &str, fields: &[Field], errors: &mut Vec<ValidationError>) {
    for field in fields {
        let owner = format!("{owner}.fields.{}", field.wire);
        crate::ext::check(&owner, &field.ext, errors);
        check_diagnostic_extensions(&owner, &field.diagnostics, errors);
    }
}

fn check_diagnostic_extensions(
    owner: &str,
    diagnostics: &[Diagnostic],
    errors: &mut Vec<ValidationError>,
) {
    for (index, diagnostic) in diagnostics.iter().enumerate() {
        crate::ext::check(&format!("{owner}[{index}]"), &diagnostic.ext, errors);
    }
}

fn check_header_extensions(
    owner: &str,
    headers: &[ResponseHeader],
    errors: &mut Vec<ValidationError>,
) {
    for (index, header) in headers.iter().enumerate() {
        crate::ext::check(&format!("{owner}.headers[{index}]"), &header.ext, errors);
    }
}

fn scan_opaque_references(
    references: &[TypeRef],
    pending: &mut Vec<(String, Option<String>)>,
) -> bool {
    let mut found = false;
    for reference in references {
        reference.visit_refs(&mut |reference| {
            if matches!(reference, TypeRef::Inline(InlineType::Opaque { .. })) {
                found = true;
            }
            if let TypeRef::Named(named) = reference {
                pending.push((named.id.clone(), named.variant.clone()));
            }
        });
    }
    found
}

impl Document {
    pub fn extension_namespaces(&self) -> BTreeSet<String> {
        crate::ext::document_namespaces(self)
    }

    pub fn canonicalize(&mut self) {
        self.types
            .values_mut()
            .for_each(|definition| definition.definition.canonicalize());
        self.auth_schemes
            .values_mut()
            .for_each(AuthScheme::canonicalize);
        self.common_errors.sort_by(|left, right| {
            left.status
                .cmp(&right.status)
                .then_with(|| left.name.cmp(&right.name))
        });
        self.common_errors.iter_mut().for_each(|error| {
            if let Some(media) = &mut error.media {
                media.canonicalize();
            }
            error.headers.sort_by(|left, right| {
                left.wire
                    .to_ascii_lowercase()
                    .cmp(&right.wire.to_ascii_lowercase())
                    .then_with(|| left.wire.cmp(&right.wire))
            });
        });
        self.operations
            .sort_by(|left, right| left.id.cmp(&right.id));
        self.operations.iter_mut().for_each(Operation::canonicalize);
    }

    pub fn has_wide_integer(&self) -> bool {
        let mut found = false;
        let mut visit = |reference: &TypeRef| {
            if matches!(
                reference,
                TypeRef::Inline(InlineType::Int {
                    bits,
                    wire: IntegerWire::Number,
                    ..
                }) if *bits > 53
            ) {
                found = true;
            }
        };
        self.types
            .values()
            .for_each(|definition| definition.definition.visit_refs(&mut visit));
        self.common_errors.iter().for_each(|error| {
            if let Some(body) = &error.body {
                body.visit_refs(&mut visit);
            }
            error
                .headers
                .iter()
                .for_each(|header| header.ty.visit_refs(&mut visit));
        });
        self.operations
            .iter()
            .for_each(|operation| operation.visit_refs(&mut visit));
        found
    }

    pub fn has_opaque(&self) -> bool {
        let mut found = false;
        let mut visit = |reference: &TypeRef| {
            if matches!(reference, TypeRef::Inline(InlineType::Opaque { .. })) {
                found = true;
            }
        };
        self.types
            .values()
            .for_each(|definition| definition.definition.visit_refs(&mut visit));
        self.common_errors.iter().for_each(|error| {
            if let Some(body) = &error.body {
                body.visit_refs(&mut visit);
            }
            error
                .headers
                .iter()
                .for_each(|header| header.ty.visit_refs(&mut visit));
        });
        self.operations
            .iter()
            .for_each(|operation| operation.visit_refs(&mut visit));
        found
    }

    pub fn has_reachable_opaque(&self) -> bool {
        let mut roots = Vec::new();
        {
            let mut visit = |reference: &TypeRef| roots.push(reference.clone());

            if self
                .operations
                .iter()
                .any(|operation| operation.errors_inherit)
            {
                self.common_errors.iter().for_each(|error| {
                    if let Some(body) = &error.body {
                        body.visit_refs(&mut visit);
                    }
                    error
                        .headers
                        .iter()
                        .for_each(|header| header.ty.visit_refs(&mut visit));
                });
            }
            self.operations
                .iter()
                .for_each(|operation| operation.visit_refs(&mut visit));
        }

        let mut pending = Vec::new();
        if scan_opaque_references(&roots, &mut pending) {
            return true;
        }
        let mut seen = BTreeSet::new();
        while let Some((id, selected)) = pending.pop() {
            if !seen.insert((id.clone(), selected.clone())) {
                continue;
            }
            let Some(definition) = self.types.get(&id) else {
                continue;
            };
            let mut references = Vec::new();
            match selected {
                Some(variant) => {
                    if let TypeKind::Enum { variants, .. } = &definition.definition
                        && let Some(variant) =
                            variants.iter().find(|candidate| candidate.wire == variant)
                    {
                        variant
                            .payload
                            .visit_refs(&mut |reference| references.push(reference.clone()));
                    }
                }
                None => definition
                    .definition
                    .visit_refs(&mut |reference| references.push(reference.clone())),
            }
            if scan_opaque_references(&references, &mut pending) {
                return true;
            }
        }
        false
    }

    pub fn to_canonical_json(&self) -> Result<String, serde_json::Error> {
        let mut document = self.clone();
        document.canonicalize();
        serde_json::to_string_pretty(&document)
    }

    pub(super) fn validate_credential_carriers(&self, errors: &mut Vec<ValidationError>) {
        for (index, operation) in self.operations.iter().enumerate() {
            for group in &operation.auth.alternatives {
                let mut carriers: Vec<((ApiKeyLocation, String), &str)> = Vec::new();
                for id in group {
                    let Some((location, name)) = self
                        .auth_schemes
                        .get(id)
                        .and_then(AuthScheme::credential_carrier)
                    else {
                        continue;
                    };
                    let key = (location, carrier_key(location, name));
                    if let Some((_, first)) = carriers.iter().find(|(other, _)| other == &key) {
                        errors.push(ValidationError::InvalidValue {
                            path: format!("operations[{index}].auth"),
                            message: format!(
                                "the {id} and {first} schemes of one auth alternative both write the {location:?} carrier {name:?}, so the credential the client attaches last overwrites the other"
                            ),
                        });
                    } else {
                        carriers.push((key, id));
                    }
                    match location {
                        ApiKeyLocation::Header => {
                            for header in operation
                                .headers
                                .iter()
                                .filter(|header| header.wire.eq_ignore_ascii_case(name))
                            {
                                errors.push(ValidationError::InvalidValue {
                                    path: format!("operations[{index}].headers"),
                                    message: format!(
                                        "the request header {} is also the {id} apiKey carrier name, so the credential overwrites it",
                                        header.wire
                                    ),
                                });
                            }
                        }
                        ApiKeyLocation::Query => {
                            if let Some(query) = &operation.query
                                && let Ok(fields) = crate::query::query_fields(self, query)
                            {
                                for field in fields.into_iter().filter(|field| field.wire == name) {
                                    errors.push(ValidationError::InvalidValue {
                                        path: format!("operations[{index}].query"),
                                        message: format!(
                                            "the query field {:?} is also the {id} apiKey carrier name, so the credential overwrites it",
                                            field.wire
                                        ),
                                    });
                                }
                            }
                        }
                        ApiKeyLocation::Cookie => {}
                    }
                }
            }
        }
    }

    pub(super) fn validate_extensions(&self, errors: &mut Vec<ValidationError>) {
        crate::ext::check("document", &self.ext, errors);
        crate::ext::check("info", &self.info.ext, errors);
        check_diagnostic_extensions("document diagnostics", &self.diagnostics, errors);
        for (id, definition) in &self.types {
            let owner = format!("type {id}");
            crate::ext::check(&owner, &definition.ext, errors);
            check_diagnostic_extensions(
                &format!("{owner} diagnostics"),
                &definition.diagnostics,
                errors,
            );
            match &definition.definition {
                TypeKind::Struct { fields, .. } => check_field_extensions(&owner, fields, errors),
                TypeKind::Enum { variants, .. } => {
                    for variant in variants {
                        let owner = format!("{owner}.variants.{}", variant.wire);
                        crate::ext::check(&owner, &variant.ext, errors);
                        check_diagnostic_extensions(
                            &format!("{owner} diagnostics"),
                            &variant.diagnostics,
                            errors,
                        );
                        if let VariantPayload::Struct { fields } = &variant.payload {
                            check_field_extensions(&owner, fields, errors);
                        }
                    }
                }
                TypeKind::Newtype { .. } => {}
            }
        }
        for (id, scheme) in &self.auth_schemes {
            match scheme {
                AuthScheme::Custom { ext, .. } => {
                    crate::ext::check(&format!("auth scheme {id}"), ext, errors)
                }
                AuthScheme::OAuth2 { flows } => {
                    for (index, flow) in flows.iter().enumerate() {
                        crate::ext::check(
                            &format!("auth scheme {id}.flows[{index}]"),
                            &flow.ext,
                            errors,
                        );
                    }
                }
                _ => {}
            }
        }
        for (index, error) in self.common_errors.iter().enumerate() {
            let owner = format!("common_errors[{index}]");
            crate::ext::check(&owner, &error.ext, errors);
            check_header_extensions(&owner, &error.headers, errors);
        }
        for (index, operation) in self.operations.iter().enumerate() {
            let owner = format!("operations[{index}]");
            crate::ext::check(&owner, &operation.ext, errors);
            check_diagnostic_extensions(
                &format!("{owner} diagnostics"),
                &operation.diagnostics,
                errors,
            );
            crate::ext::check(&format!("{owner}.auth"), &operation.auth.ext, errors);
            if let Some(provides) = &operation.provides {
                crate::ext::check(&format!("{owner}.provides"), &provides.ext, errors);
            }
            if let Some(body) = &operation.body {
                crate::ext::check(&format!("{owner}.body"), &body.ext, errors);
            }
            for (header_index, header) in operation.headers.iter().enumerate() {
                crate::ext::check(
                    &format!("{owner}.headers[{header_index}]"),
                    &header.ext,
                    errors,
                );
            }
            for (response_index, response) in operation.responses.iter().enumerate() {
                let owner = format!("{owner}.responses[{response_index}]");
                crate::ext::check(&owner, &response.ext, errors);
                check_header_extensions(&owner, &response.headers, errors);
            }
            for (error_index, error) in operation.errors.iter().enumerate() {
                let owner = format!("{owner}.errors[{error_index}]");
                crate::ext::check(&owner, &error.ext, errors);
                check_header_extensions(&owner, &error.headers, errors);
            }
        }
    }

    pub(super) fn validate_directions(&self, errors: &mut Vec<ValidationError>) {
        for (index, error) in self.common_errors.iter().enumerate() {
            let prefix = format!("common_errors[{index}]");
            if let Some(body) = &error.body {
                self.check_direction(
                    body,
                    "error body",
                    format!("{prefix}.body"),
                    Boundary::Response,
                    errors,
                );
            }
            for (header_index, header) in error.headers.iter().enumerate() {
                self.check_direction(
                    &header.ty,
                    "error header",
                    format!("{prefix}.headers[{header_index}].ty"),
                    Boundary::Response,
                    errors,
                );
            }
        }
        for (index, operation) in self.operations.iter().enumerate() {
            let prefix = format!("operations[{index}]");
            for (segment_index, segment) in operation.path.iter().enumerate() {
                if let PathSegment::Param { ty, .. } = segment {
                    self.check_direction(
                        ty,
                        "path parameter",
                        format!("{prefix}.path[{segment_index}].ty"),
                        Boundary::Request,
                        errors,
                    );
                }
            }
            if let Some(query) = &operation.query {
                self.check_direction(
                    query,
                    "query",
                    format!("{prefix}.query"),
                    Boundary::Request,
                    errors,
                );
            }
            for (header_index, header) in operation.headers.iter().enumerate() {
                self.check_direction(
                    &header.ty,
                    "request header",
                    format!("{prefix}.headers[{header_index}].ty"),
                    Boundary::Request,
                    errors,
                );
            }
            if let Some(body) = &operation.body {
                self.check_direction(
                    &body.ty,
                    "request body",
                    format!("{prefix}.body.ty"),
                    Boundary::Request,
                    errors,
                );
            }
            for (response_index, response) in operation.responses.iter().enumerate() {
                let response_prefix = format!("{prefix}.responses[{response_index}]");
                if let Some(body) = &response.body {
                    self.check_direction(
                        body,
                        "response body",
                        format!("{response_prefix}.body"),
                        Boundary::Response,
                        errors,
                    );
                }
                for (header_index, header) in response.headers.iter().enumerate() {
                    self.check_direction(
                        &header.ty,
                        "response header",
                        format!("{response_prefix}.headers[{header_index}].ty"),
                        Boundary::Response,
                        errors,
                    );
                }
            }
            for (error_index, error) in operation.errors.iter().enumerate() {
                let error_prefix = format!("{prefix}.errors[{error_index}]");
                if let Some(body) = &error.body {
                    self.check_direction(
                        body,
                        "error body",
                        format!("{error_prefix}.body"),
                        Boundary::Response,
                        errors,
                    );
                }
                for (header_index, header) in error.headers.iter().enumerate() {
                    self.check_direction(
                        &header.ty,
                        "error header",
                        format!("{error_prefix}.headers[{header_index}].ty"),
                        Boundary::Response,
                        errors,
                    );
                }
            }
            if let Some(stream) = &operation.stream {
                match &stream.kind {
                    StreamKind::Sse { events } => {
                        for (event_index, event) in events.iter().enumerate() {
                            self.check_direction(
                                &event.data,
                                "stream event",
                                format!("{prefix}.stream.events[{event_index}].data"),
                                Boundary::Response,
                                errors,
                            );
                        }
                    }
                    StreamKind::Ndjson { item } => self.check_direction(
                        item,
                        "stream item",
                        format!("{prefix}.stream.item"),
                        Boundary::Response,
                        errors,
                    ),
                    StreamKind::Websocket { client, server } => {
                        self.check_direction(
                            client,
                            "websocket client message",
                            format!("{prefix}.stream.client"),
                            Boundary::Request,
                            errors,
                        );
                        self.check_direction(
                            server,
                            "websocket server message",
                            format!("{prefix}.stream.server"),
                            Boundary::Response,
                            errors,
                        );
                    }
                    StreamKind::Bytes { .. } => {}
                }
            }
        }
    }

    fn check_direction(
        &self,
        reference: &TypeRef,
        role: &str,
        path: String,
        boundary: Boundary,
        errors: &mut Vec<ValidationError>,
    ) {
        if let Some(mismatch) = self.direction_mismatch(reference, boundary) {
            errors.push(ValidationError::InvalidValue {
                path,
                message: format!(
                    "the {role} needs a type of the {} direction, but {mismatch}",
                    boundary.crossing()
                ),
            });
        }
    }

    fn direction_mismatch(&self, reference: &TypeRef, boundary: Boundary) -> Option<String> {
        self.direction_reached(reference, boundary, &mut BTreeSet::new())
    }

    fn direction_reached(
        &self,
        reference: &TypeRef,
        boundary: Boundary,
        visited: &mut BTreeSet<(String, Option<String>)>,
    ) -> Option<String> {
        let TypeRef::Named(named) = reference else {
            return match reference {
                TypeRef::Inline(InlineType::Nullable { inner }) => {
                    self.direction_reached(inner, boundary, visited)
                }
                TypeRef::Inline(InlineType::Array { item, .. }) => {
                    self.direction_reached(item, boundary, visited)
                }
                TypeRef::Inline(InlineType::Map { key, value }) => self
                    .direction_reached(key, boundary, visited)
                    .or_else(|| self.direction_reached(value, boundary, visited)),
                TypeRef::Inline(InlineType::Tuple { items }) => items
                    .iter()
                    .find_map(|item| self.direction_reached(item, boundary, visited)),
                _ => None,
            };
        };
        let definition = self.types.get(&named.id)?;
        if boundary.refuses(definition.direction) {
            return Some(format!(
                "{} is declared {}",
                named.id,
                direction_name(definition.direction)
            ));
        }
        if !visited.insert((named.id.clone(), named.variant.clone())) {
            return None;
        }
        match &definition.definition {
            TypeKind::Newtype { inner } => self.direction_reached(inner, boundary, visited),
            TypeKind::Struct {
                fields,
                flattened,
                openness,
            } => fields
                .iter()
                .filter(|field| field_crosses(field, boundary))
                .find_map(|field| self.direction_reached(&field.ty, boundary, visited))
                .or_else(|| {
                    flattened
                        .iter()
                        .find_map(|member| self.direction_reached(member, boundary, visited))
                })
                .or_else(|| match openness {
                    ObjectOpenness::PreserveTyped { value } => {
                        self.direction_reached(value, boundary, visited)
                    }
                    _ => None,
                }),
            TypeKind::Enum { variants, .. } => {
                let selected: Vec<_> = match &named.variant {
                    Some(wire) => variants
                        .iter()
                        .filter(|variant| variant.wire == *wire)
                        .collect(),
                    None => variants.iter().collect(),
                };
                selected.iter().find_map(|variant| match &variant.payload {
                    VariantPayload::Unit => None,
                    VariantPayload::Newtype { ty } => self.direction_reached(ty, boundary, visited),
                    VariantPayload::Tuple { items } => items
                        .iter()
                        .find_map(|item| self.direction_reached(item, boundary, visited)),
                    VariantPayload::Struct { fields } => fields
                        .iter()
                        .filter(|field| field_crosses(field, boundary))
                        .find_map(|field| self.direction_reached(&field.ty, boundary, visited)),
                })
            }
        }
    }

    pub(crate) fn is_nullable_type(&self, ty: &TypeRef, visited: &mut BTreeSet<String>) -> bool {
        match ty {
            TypeRef::Inline(InlineType::Nullable { .. }) => true,
            TypeRef::Named(named) => {
                if !visited.insert(named.id.clone()) {
                    return false;
                }
                let result = self.types.get(&named.id).is_some_and(|definition| {
                    matches!(
                        &definition.definition,
                        TypeKind::Newtype { inner } if self.is_nullable_type(inner, visited)
                    )
                });
                visited.remove(&named.id);
                result
            }
            TypeRef::Inline(_) => false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Exact {
    Integer((bool, u128)),
    Fraction,
    Beyond,
}

pub(crate) fn exact_integer(value: &Value) -> Exact {
    let Some(number) = value.as_number() else {
        return Exact::Fraction;
    };
    if number.is_f64() {
        return match number.as_f64() {
            Some(float) if float.fract() != 0.0 => Exact::Fraction,
            Some(float) if float.abs() < 2_f64.powi(128) => {
                let magnitude = float.abs() as u128;
                Exact::Integer((float.is_sign_negative() && magnitude != 0, magnitude))
            }
            _ => Exact::Beyond,
        };
    }
    let text = number.to_string();
    let (negative, unsigned) = match text.strip_prefix('-') {
        Some(unsigned) => (true, unsigned),
        None => (false, text.as_str()),
    };
    let (mantissa, exponent) = match unsigned.split_once(['e', 'E']) {
        Some((mantissa, exponent)) => (
            mantissa,
            exponent
                .parse::<i64>()
                .unwrap_or(if exponent.starts_with('-') {
                    i64::MIN
                } else {
                    i64::MAX
                }),
        ),
        None => (unsigned, 0),
    };
    let (whole, fraction) = match mantissa.split_once('.') {
        Some((whole, fraction)) => (whole, fraction),
        None => (mantissa, ""),
    };
    let digits = format!("{whole}{fraction}");
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return Exact::Fraction;
    }
    let significant = digits.trim_start_matches('0');
    if significant.is_empty() {
        return Exact::Integer((false, 0));
    }
    let scale = i64::try_from(fraction.len())
        .unwrap_or(i64::MAX)
        .saturating_sub(exponent);
    let magnitude = if scale <= 0 {
        let zeros = usize::try_from(scale.unsigned_abs()).unwrap_or(usize::MAX);
        if significant.len().saturating_add(zeros) > 39 {
            return Exact::Beyond;
        }
        let mut integer = String::with_capacity(significant.len() + zeros);
        integer.push_str(significant);
        integer.extend(std::iter::repeat_n('0', zeros));
        match integer.parse() {
            Ok(magnitude) => magnitude,
            Err(_) => return Exact::Beyond,
        }
    } else {
        let scale = usize::try_from(scale).unwrap_or(usize::MAX);
        if scale >= significant.len()
            || significant[significant.len() - scale..]
                .bytes()
                .any(|byte| byte != b'0')
        {
            return Exact::Fraction;
        }
        match significant[..significant.len() - scale].parse() {
            Ok(magnitude) => magnitude,
            Err(_) => return Exact::Beyond,
        }
    };
    Exact::Integer((negative && magnitude != 0, magnitude))
}
