use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::IR_VERSION;
use crate::operation::{
    ApiKeyLocation, AuthScheme, ContentKind, CredentialLocator, ErrorContent, ErrorResponse,
    ErrorStatus, Method, OAuthFlowKind, Operation, PathSegment, ResponseStatus, StreamKind,
};
use crate::types::{
    Access, Diagnostic, DiagnosticSeverity, Direction, EnumRepr, Field, InlineType, IntegerWire,
    ObjectOpenness, OpaqueWire, Presence, TypeDef, TypeKind, TypeRef, VariantPayload,
};
use crate::violation::direction_name;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Document {
    #[schemars(regex(pattern = "^0\\.2$"))]
    pub ir_version: String,
    pub info: Info,
    #[serde(default)]
    pub types: BTreeMap<String, TypeDef>,
    #[serde(default)]
    pub auth_schemes: BTreeMap<String, AuthScheme>,
    #[serde(default)]
    pub common_errors: Vec<ErrorResponse>,
    #[serde(default)]
    pub operations: Vec<Operation>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub ext: BTreeMap<String, Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<Diagnostic>,
}

fn status_forbids_body(status: u16) -> bool {
    matches!(status, 100..=199 | 204 | 205 | 304)
}

fn response_status_conflicts(response: ResponseStatus, error: &ErrorStatus) -> bool {
    match (response, error) {
        (ResponseStatus::Exact(response), ErrorStatus::Exact(error)) => response == *error,
        (ResponseStatus::Exact(response), ErrorStatus::Class(class)) => {
            response / 100 == u16::from(*class)
        }
        (ResponseStatus::Class(response), ErrorStatus::Exact(error)) => {
            u16::from(response) == *error / 100
        }
        (ResponseStatus::Class(response), ErrorStatus::Class(error)) => response == *error,
        (ResponseStatus::Default | ResponseStatus::Unknown, ErrorStatus::Exact(status)) => {
            (200..300).contains(status)
        }
        (ResponseStatus::Default | ResponseStatus::Unknown, ErrorStatus::Class(class)) => {
            *class == 2
        }
        (ResponseStatus::Default | ResponseStatus::Unknown, ErrorStatus::Default) => true,
        _ => false,
    }
}

#[derive(Clone, Copy)]
enum Boundary {
    Request,
    Response,
}

impl Boundary {
    fn refuses(self, direction: Direction) -> bool {
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

fn field_crosses(field: &Field, boundary: Boundary) -> bool {
    !matches!(
        (boundary, field.access),
        (Boundary::Request, Some(Access::ReadOnly)) | (Boundary::Response, Some(Access::WriteOnly))
    )
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

    pub fn validate(&self) -> Result<(), Vec<ValidationError>> {
        let mut errors = Vec::new();

        if self.ir_version != IR_VERSION {
            errors.push(ValidationError::UnsupportedVersion {
                found: self.ir_version.clone(),
            });
        }
        self.validate_diagnostics("document", &self.diagnostics, &mut errors);
        for (id, definition) in &self.types {
            self.validate_diagnostics(&format!("type {id}"), &definition.diagnostics, &mut errors);
            self.validate_type(id, definition, &mut errors);
        }
        for (id, scheme) in &self.auth_schemes {
            if let AuthScheme::OAuth2 { flows } = scheme {
                let owner = format!("auth scheme {id}");
                if flows.is_empty() {
                    errors.push(ValidationError::InvalidValue {
                        path: owner.clone(),
                        message: "OAuth2 scheme needs at least one flow".into(),
                    });
                }
                let mut kinds = BTreeSet::new();
                for flow in flows {
                    if !kinds.insert(flow.kind) {
                        errors.push(ValidationError::InvalidValue {
                            path: owner.clone(),
                            message: format!("duplicate OAuth2 flow {:?}", flow.kind),
                        });
                    }
                    let valid_urls = match flow.kind {
                        OAuthFlowKind::AuthorizationCode => {
                            flow.authorization_url.is_some() && flow.token_url.is_some()
                        }
                        OAuthFlowKind::Implicit => {
                            flow.authorization_url.is_some() && flow.token_url.is_none()
                        }
                        OAuthFlowKind::Password | OAuthFlowKind::ClientCredentials => {
                            flow.authorization_url.is_none() && flow.token_url.is_some()
                        }
                    };
                    if !valid_urls {
                        errors.push(ValidationError::InvalidValue {
                            path: owner.clone(),
                            message: format!("OAuth2 {:?} flow has invalid URL fields", flow.kind),
                        });
                    }
                }
            }
            if let AuthScheme::ApiKey {
                location: ApiKeyLocation::Header,
                name,
            } = scheme
            {
                self.validate_header_name(
                    &format!("auth scheme {id}"),
                    "API key header",
                    name,
                    &mut BTreeSet::new(),
                    &mut errors,
                );
            }
            if matches!(
                scheme,
                AuthScheme::ApiKey {
                    location: ApiKeyLocation::Query | ApiKeyLocation::Cookie,
                    name
                } if name.is_empty()
            ) || matches!(scheme, AuthScheme::CookieSession { cookie } if cookie.is_empty())
            {
                errors.push(ValidationError::InvalidValue {
                    path: format!("auth scheme {id}"),
                    message: "query and cookie carrier names cannot be empty".into(),
                });
            }
        }

        let mut common_error_ids = BTreeSet::new();
        let mut common_error_slots = BTreeSet::new();
        for (index, error) in self.common_errors.iter().enumerate() {
            if !common_error_slots.insert(error.status.clone()) {
                errors.push(ValidationError::InvalidValue {
                    path: "common_errors".into(),
                    message: format!("duplicate error dispatch slot {:?}", error.status),
                });
            }
            if let Some(covering) =
                self.common_errors
                    .iter()
                    .enumerate()
                    .find(|(other, candidate)| {
                        *other != index && candidate.status.covers(&error.status)
                    })
            {
                errors.push(ValidationError::InvalidValue {
                    path: "common_errors".into(),
                    message: format!(
                        "error dispatch slot {:?} is already claimed: the common error {:?} covers it",
                        error.status, covering.1.status
                    ),
                });
            }
            if !common_error_ids.insert((error.status.clone(), error.name.clone())) {
                errors.push(ValidationError::InvalidValue {
                    path: "common_errors".into(),
                    message: format!("duplicate error {:?} {:?}", error.status, error.name),
                });
            }
            self.validate_error("common_errors", error, false, &mut errors);
            self.visit_error_refs("common_errors", error, &mut errors);
        }

        let mut operation_ids = BTreeSet::new();
        for operation in &self.operations {
            let id = operation.id.clone();
            let owner = format!("operation {id:?}");
            self.validate_diagnostics(&owner, &operation.diagnostics, &mut errors);
            if id.is_empty()
                || id
                    .iter()
                    .any(|name| name.is_empty() || name.iter().any(String::is_empty))
            {
                errors.push(ValidationError::InvalidValue {
                    path: owner.clone(),
                    message: "operation id must contain non-empty name segments".into(),
                });
            }
            if !operation_ids.insert(id.clone()) {
                errors.push(ValidationError::DuplicateOperation { id });
            }

            let mut path_parameters = BTreeSet::new();
            for segment in &operation.path {
                if let crate::operation::PathSegment::Param { param, ty } = segment {
                    if !path_parameters.insert(param) {
                        errors.push(ValidationError::InvalidValue {
                            path: owner.clone(),
                            message: format!("duplicate path parameter {param}"),
                        });
                    }
                    if self.is_nullable_type(ty, &mut BTreeSet::new()) {
                        errors.push(ValidationError::InvalidValue {
                            path: owner.clone(),
                            message: format!("required path parameter {param} cannot be nullable"),
                        });
                    }
                }
            }
            let mut header_names = BTreeSet::new();
            for header in &operation.headers {
                self.validate_header_name(
                    &owner,
                    "request header",
                    &header.wire,
                    &mut header_names,
                    &mut errors,
                );
                if header.presence == Presence::Required
                    && self.is_nullable_type(&header.ty, &mut BTreeSet::new())
                {
                    errors.push(ValidationError::InvalidValue {
                        path: owner.clone(),
                        message: format!(
                            "required request header {} cannot be nullable",
                            header.wire
                        ),
                    });
                }
            }
            if let Some(body) = &operation.body {
                if body.content == ContentKind::Empty {
                    errors.push(ValidationError::InvalidValue {
                        path: owner.clone(),
                        message: format!(
                            "request body content {:?} cannot carry a body type",
                            body.content
                        ),
                    });
                }
                if body.content == ContentKind::Empty && body.media_type.is_some() {
                    errors.push(ValidationError::InvalidValue {
                        path: owner.clone(),
                        message: "empty request bodies cannot declare a media type".into(),
                    });
                }
                if let Some(media_type) = &body.media_type {
                    self.validate_media_type(&owner, "request body", media_type, &mut errors);
                }
                if body.content == ContentKind::Bytes
                    && !self.is_byte_type(&body.ty, &mut BTreeSet::new())
                {
                    errors.push(ValidationError::InvalidValue {
                        path: owner.clone(),
                        message: "raw byte bodies must use a byte type, byte newtype, nullable bytes, or a bytes opaque"
                            .into(),
                    });
                }
                if body.content == ContentKind::Form && !self.is_flat_form_body(&body.ty) {
                    errors.push(ValidationError::InvalidValue {
                        path: owner.clone(),
                        message: "form bodies must be a flat record of scalars and arrays of scalars, because a urlencoded form cannot carry nested objects"
                            .into(),
                    });
                }
            }
            if let Some(query) = &operation.query
                && let Err(rejection) = crate::query::query_fields(self, query)
            {
                errors.push(ValidationError::InvalidValue {
                    path: owner.clone(),
                    message: rejection.message(),
                });
            }
            self.visit_operation_refs(&owner, operation, &mut errors);
            if let Some(stream) = &operation.stream
                && let Err(message) = stream.kind.validate()
            {
                errors.push(ValidationError::InvalidValue {
                    path: owner.clone(),
                    message,
                });
            }

            for alternative in &operation.auth.alternatives {
                if alternative.is_empty() {
                    errors.push(ValidationError::InvalidValue {
                        path: owner.clone(),
                        message: "auth alternative cannot be empty".into(),
                    });
                    continue;
                }
                for scheme in alternative {
                    if !self.auth_schemes.contains_key(scheme) {
                        errors.push(ValidationError::UnknownAuthScheme {
                            owner: owner.clone(),
                            scheme: scheme.clone(),
                        });
                    }
                }
            }
            for (scheme, scopes) in &operation.auth.scopes {
                if !self.auth_schemes.contains_key(scheme) {
                    errors.push(ValidationError::UnknownAuthScheme {
                        owner: owner.clone(),
                        scheme: scheme.clone(),
                    });
                }
                if !operation
                    .auth
                    .alternatives
                    .iter()
                    .any(|alternative| alternative.iter().any(|candidate| candidate == scheme))
                {
                    errors.push(ValidationError::InvalidValue {
                        path: owner.clone(),
                        message: format!("auth scheme {scheme} has scopes but no auth requirement"),
                    });
                }
                if scopes.is_empty() {
                    errors.push(ValidationError::InvalidValue {
                        path: owner.clone(),
                        message: format!("auth scheme {scheme} has an empty scope list"),
                    });
                }
                let mut unique = BTreeSet::new();
                for scope in scopes {
                    if scope.is_empty() || scope.chars().any(char::is_whitespace) {
                        errors.push(ValidationError::InvalidValue {
                            path: owner.clone(),
                            message: format!("auth scheme {scheme} has an invalid scope"),
                        });
                    }
                    if !unique.insert(scope) {
                        errors.push(ValidationError::InvalidValue {
                            path: owner.clone(),
                            message: format!("auth scheme {scheme} repeats scope {scope}"),
                        });
                    }
                }
            }
            if let Some(provides) = &operation.provides
                && !self.auth_schemes.contains_key(&provides.scheme)
            {
                errors.push(ValidationError::UnknownAuthScheme {
                    owner: owner.clone(),
                    scheme: provides.scheme.clone(),
                });
            }
            if let Some(provides) = &operation.provides {
                for locator in std::iter::once(&provides.value).chain(provides.expires_at.iter()) {
                    if let CredentialLocator::Header { name } = locator {
                        self.validate_header_name(
                            &owner,
                            "credential header",
                            name,
                            &mut BTreeSet::new(),
                            &mut errors,
                        );
                    }
                    if matches!(locator, CredentialLocator::SetCookie { name } if name.is_empty()) {
                        errors.push(ValidationError::InvalidValue {
                            path: owner.clone(),
                            message: "credential cookie name cannot be empty".into(),
                        });
                    }
                    if let Err(message) = locator.validate() {
                        errors.push(ValidationError::InvalidValue {
                            path: owner.clone(),
                            message,
                        });
                    }
                }
            }

            let mut response_statuses = BTreeSet::new();
            let mut stream_responses = 0;
            for response in &operation.responses {
                match response.status {
                    ResponseStatus::Exact(status) if !(100..=599).contains(&status) => {
                        errors.push(ValidationError::InvalidValue {
                            path: owner.clone(),
                            message: format!("invalid response status {status}"),
                        });
                    }
                    ResponseStatus::Class(class) if !(1..=5).contains(&class) => {
                        errors.push(ValidationError::InvalidValue {
                            path: owner.clone(),
                            message: format!("invalid response status class {class}xx"),
                        });
                    }
                    _ => {}
                }
                if !response_statuses.insert(response.status) {
                    errors.push(ValidationError::InvalidValue {
                        path: owner.clone(),
                        message: format!("duplicate response status {}", response.status),
                    });
                }
                if response.content == ContentKind::Empty && response.media_type.is_some() {
                    errors.push(ValidationError::InvalidValue {
                        path: owner.clone(),
                        message: format!(
                            "empty response {} cannot declare a media type",
                            response.status
                        ),
                    });
                }
                if let Some(media_type) = &response.media_type {
                    self.validate_media_type(
                        &owner,
                        &format!("response {} body", response.status),
                        media_type,
                        &mut errors,
                    );
                }
                if response.content == ContentKind::Stream
                    && let Some(media_type) = &response.media_type
                    && let Some(content_type) = operation
                        .stream
                        .as_ref()
                        .and_then(|stream| stream.kind.media_type())
                    && !crate::media::media_type_matches(media_type, content_type)
                {
                    errors.push(ValidationError::InvalidValue {
                        path: owner.clone(),
                        message: format!(
                            "response media type {media_type:?} does not match stream kind media type {content_type:?}"
                        ),
                    });
                }
                let content_matches_body = match response.content {
                    ContentKind::Empty | ContentKind::Stream => response.body.is_none(),
                    _ => response.body.is_some(),
                };
                if !content_matches_body {
                    errors.push(ValidationError::InvalidValue {
                        path: owner.clone(),
                        message: format!(
                            "response {} content {:?} does not match its body",
                            response.status, response.content
                        ),
                    });
                }
                if (operation.method == Method::Head
                    || matches!(response.status, ResponseStatus::Class(1))
                    || matches!(response.status, ResponseStatus::Exact(status) if status_forbids_body(status)))
                    && (response.content != ContentKind::Empty || response.body.is_some())
                {
                    errors.push(ValidationError::InvalidValue {
                        path: owner.clone(),
                        message: format!("response {} cannot carry a body", response.status),
                    });
                }
                if response.content == ContentKind::Stream {
                    stream_responses += 1;
                }
                if response.content == ContentKind::Bytes
                    && let Some(body) = &response.body
                    && !self.is_byte_type(body, &mut BTreeSet::new())
                {
                    errors.push(ValidationError::InvalidValue {
                        path: owner.clone(),
                        message: format!(
                            "raw byte response bodies must use a byte type, byte newtype, nullable bytes, or a bytes opaque (response {})",
                            response.status
                        ),
                    });
                }
                let mut response_header_names = BTreeSet::new();
                for header in &response.headers {
                    self.validate_header_name(
                        &owner,
                        &format!("response {} header", response.status),
                        &header.wire,
                        &mut response_header_names,
                        &mut errors,
                    );
                }
            }
            if response_statuses.contains(&ResponseStatus::Default)
                && response_statuses.contains(&ResponseStatus::Unknown)
            {
                errors.push(ValidationError::InvalidValue {
                    path: owner.clone(),
                    message: format!("duplicate response status {}", ResponseStatus::Default),
                });
            }
            let stream_descriptions = usize::from(operation.stream.is_some());
            if stream_responses != stream_descriptions {
                errors.push(ValidationError::InvalidValue {
                    path: owner.clone(),
                    message: format!(
                        "operation has {stream_descriptions} stream descriptions and {stream_responses} stream responses"
                    ),
                });
            }
            let mut operation_error_ids = BTreeSet::new();
            let mut operation_error_slots = BTreeSet::new();
            for error in &operation.errors {
                let identity = (error.status.clone(), error.name.clone());
                if !operation_error_slots.insert(error.status.clone()) {
                    errors.push(ValidationError::InvalidValue {
                        path: owner.clone(),
                        message: format!("duplicate error dispatch slot {:?}", error.status),
                    });
                } else if operation.errors_inherit
                    && let Some(common) = self
                        .common_errors
                        .iter()
                        .find(|common| common.status.covers(&error.status))
                {
                    errors.push(ValidationError::InvalidValue {
                        path: owner.clone(),
                        message: format!(
                            "error dispatch slot {:?} is inherited: the common error {:?} already covers it",
                            error.status, common.status
                        ),
                    });
                }
                if !operation_error_ids.insert(identity.clone()) {
                    errors.push(ValidationError::InvalidValue {
                        path: owner.clone(),
                        message: format!("duplicate error {:?} {:?}", error.status, error.name),
                    });
                } else if operation.errors_inherit && common_error_ids.contains(&identity) {
                    errors.push(ValidationError::InvalidValue {
                        path: owner.clone(),
                        message: format!("error {:?} {:?} is inherited", error.status, error.name),
                    });
                }
                if operation
                    .responses
                    .iter()
                    .any(|response| response_status_conflicts(response.status, &error.status))
                {
                    errors.push(ValidationError::InvalidValue {
                        path: owner.clone(),
                        message: format!(
                            "error status {:?} collides with a success response",
                            error.status
                        ),
                    });
                }
                self.validate_error(&owner, error, operation.method == Method::Head, &mut errors);
            }
            if operation.errors_inherit {
                for error in &self.common_errors {
                    if operation
                        .responses
                        .iter()
                        .any(|response| response_status_conflicts(response.status, &error.status))
                    {
                        errors.push(ValidationError::InvalidValue {
                            path: owner.clone(),
                            message: format!(
                                "error status {:?} collides with a success response",
                                error.status
                            ),
                        });
                    }
                    if operation.method == Method::Head && error.body.is_some() {
                        errors.push(ValidationError::InvalidValue {
                            path: owner.clone(),
                            message: format!("error {:?} cannot carry a body", error.status),
                        });
                    }
                }
            }
        }
        self.validate_directions(&mut errors);

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }

    fn validate_directions(&self, errors: &mut Vec<ValidationError>) {
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
                    StreamKind::Websocket { .. } | StreamKind::Bytes { .. } => {}
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
        visited: &mut BTreeSet<String>,
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
        if !visited.insert(named.id.clone()) {
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

    fn validate_type(&self, id: &str, definition: &TypeDef, errors: &mut Vec<ValidationError>) {
        let owner = format!("type {id}");
        if id.is_empty()
            || definition.name.is_empty()
            || definition.name.iter().any(String::is_empty)
        {
            errors.push(ValidationError::InvalidValue {
                path: owner.clone(),
                message: "named types need a non-empty id and name".into(),
            });
        }
        definition.definition.visit_refs(&mut |reference| {
            self.validate_ref(&owner, reference, errors);
            self.validate_inline(&owner, reference, errors);
        });
        if let TypeKind::Struct { flattened, .. } = &definition.definition {
            for (index, ty) in flattened.iter().enumerate() {
                if self.is_nullable_type(ty, &mut BTreeSet::new()) {
                    errors.push(ValidationError::InvalidValue {
                        path: format!("{owner}.flattened[{index}]"),
                        message: "nullable flattened types are not representable".into(),
                    });
                }
            }
        }

        match &definition.definition {
            TypeKind::Struct { fields, .. } => self.validate_fields(&owner, fields, errors),
            TypeKind::Enum { repr, variants } => {
                if let EnumRepr::Adjacent { tag, content } = repr
                    && tag == content
                {
                    errors.push(ValidationError::InvalidValue {
                        path: owner.clone(),
                        message: "adjacent enum tag and content must differ".into(),
                    });
                }
                if variants.is_empty() {
                    errors.push(ValidationError::InvalidValue {
                        path: owner.clone(),
                        message: "enum needs at least one variant".into(),
                    });
                }
                let mut names = BTreeSet::new();
                for variant in variants {
                    self.validate_diagnostics(
                        &format!("{owner}.variants.{}", variant.wire),
                        &variant.diagnostics,
                        errors,
                    );
                    if !names.insert(&variant.wire) {
                        errors.push(ValidationError::InvalidValue {
                            path: owner.clone(),
                            message: format!("duplicate enum variant {}", variant.wire),
                        });
                    }
                    if matches!(repr, EnumRepr::String)
                        && !matches!(variant.payload, VariantPayload::Unit)
                    {
                        errors.push(ValidationError::InvalidValue {
                            path: owner.clone(),
                            message: "string enum variants must have unit payloads".into(),
                        });
                    }
                    let reserved = match repr {
                        EnumRepr::Internal { tag } => vec![tag.as_str()],
                        EnumRepr::Adjacent { tag, content } => vec![tag.as_str(), content.as_str()],
                        _ => Vec::new(),
                    };
                    if let VariantPayload::Struct { fields } = &variant.payload
                        && fields
                            .iter()
                            .any(|field| reserved.contains(&field.wire.as_str()))
                    {
                        errors.push(ValidationError::InvalidValue {
                            path: owner.clone(),
                            message: format!("variant {} uses a reserved enum field", variant.wire),
                        });
                    }
                    if let VariantPayload::Struct { fields } = &variant.payload {
                        self.validate_fields(&owner, fields, errors);
                    }
                }
            }
            TypeKind::Newtype { .. } => {}
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

    fn is_byte_type(&self, ty: &TypeRef, visited: &mut BTreeSet<String>) -> bool {
        match ty {
            TypeRef::Inline(InlineType::Bytes { .. }) => true,
            TypeRef::Inline(InlineType::Nullable { inner }) => self.is_byte_type(inner, visited),
            TypeRef::Inline(InlineType::Opaque {
                wire: OpaqueWire::Bytes,
                ..
            }) => true,
            TypeRef::Named(named) => {
                if !visited.insert(named.id.clone()) {
                    return false;
                }
                let result = self.types.get(&named.id).is_some_and(|definition| {
                    matches!(
                        &definition.definition,
                        TypeKind::Newtype { inner } if self.is_byte_type(inner, visited)
                    )
                });
                visited.remove(&named.id);
                result
            }
            TypeRef::Inline(_) => false,
        }
    }

    fn is_flat_form_body(&self, ty: &TypeRef) -> bool {
        self.is_form_value(ty, true, &mut BTreeSet::new())
    }

    fn is_form_value(&self, ty: &TypeRef, top: bool, visiting: &mut BTreeSet<String>) -> bool {
        match ty {
            TypeRef::Named(named) => {
                if !visiting.insert(named.id.clone()) {
                    return false;
                }
                let result = match self
                    .types
                    .get(&named.id)
                    .map(|definition| &definition.definition)
                {
                    None => true,
                    Some(TypeKind::Newtype { inner }) => self.is_form_value(inner, top, visiting),
                    Some(TypeKind::Struct { fields, .. }) if top => fields.iter().all(|field| {
                        crate::query::encodes_nullable_as_absence(self, field)
                            && self.is_form_value(&field.ty, false, visiting)
                    }),
                    Some(TypeKind::Enum { .. }) => {
                        crate::query::is_scalar_field(self, ty, &mut Vec::new())
                    }
                    Some(TypeKind::Struct { .. }) => false,
                };
                visiting.remove(&named.id);
                result
            }
            TypeRef::Inline(InlineType::Nullable { inner }) => {
                self.is_form_value(inner, top, visiting)
            }
            TypeRef::Inline(InlineType::Array { item, .. }) if top => {
                self.is_form_value(item, false, visiting)
            }
            TypeRef::Inline(InlineType::Map { value, .. }) if top => {
                self.is_form_value(value, false, visiting)
            }
            TypeRef::Inline(_) if top => true,
            TypeRef::Inline(InlineType::Array { item, .. }) => {
                crate::query::is_scalar_field(self, item, &mut Vec::new())
            }
            TypeRef::Inline(_) => crate::query::is_scalar_field(self, ty, &mut Vec::new()),
        }
    }

    fn validate_fields(&self, owner: &str, fields: &[Field], errors: &mut Vec<ValidationError>) {
        let mut names = BTreeSet::new();
        for field in fields {
            self.validate_diagnostics(
                &format!("{owner}.fields.{}", field.wire),
                &field.diagnostics,
                errors,
            );
            if !names.insert(&field.wire) {
                errors.push(ValidationError::InvalidValue {
                    path: owner.into(),
                    message: format!("duplicate field {}", field.wire),
                });
            }
        }
    }

    fn validate_header_name(
        &self,
        owner: &str,
        context: &str,
        wire: &str,
        names: &mut BTreeSet<String>,
        errors: &mut Vec<ValidationError>,
    ) {
        if wire.is_empty() {
            errors.push(ValidationError::InvalidValue {
                path: owner.into(),
                message: format!("{context} name cannot be empty"),
            });
        } else if !wire
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&byte))
        {
            errors.push(ValidationError::InvalidValue {
                path: owner.into(),
                message: format!("{context} name {wire:?} is not a valid HTTP field name"),
            });
        } else if !names.insert(wire.to_ascii_lowercase()) {
            errors.push(ValidationError::InvalidValue {
                path: owner.into(),
                message: format!("duplicate {context} {wire}"),
            });
        }
    }

    fn validate_inline(&self, owner: &str, ty: &TypeRef, errors: &mut Vec<ValidationError>) {
        let TypeRef::Inline(ty) = ty else { return };
        match ty {
            InlineType::Int {
                bits,
                signed,
                minimum,
                maximum,
                multiple_of,
                ..
            } => {
                if ![8, 16, 32, 64, 128].contains(bits) {
                    errors.push(ValidationError::InvalidValue {
                        path: owner.into(),
                        message: format!("integer width must be 8, 16, 32, 64, or 128, got {bits}"),
                    });
                }
                for (name, value) in [
                    ("minimum", minimum),
                    ("maximum", maximum),
                    ("multiple_of", multiple_of),
                ] {
                    if let Some(value) = value {
                        if !value.is_number() {
                            errors.push(ValidationError::InvalidValue {
                                path: owner.into(),
                                message: format!("integer {name} must be a number"),
                            });
                        } else if !integral_number(value) {
                            errors.push(ValidationError::InvalidValue {
                                path: owner.into(),
                                message: format!(
                                    "integer {name} must be an integral value, got {value}"
                                ),
                            });
                        }
                    }
                }
                for (name, value) in [("minimum", minimum), ("maximum", maximum)] {
                    if value
                        .as_ref()
                        .is_some_and(|value| integer_below_native(value, *bits, *signed))
                    {
                        errors.push(ValidationError::InvalidValue {
                            path: owner.into(),
                            message: format!("integer {name} is below its native range"),
                        });
                    }
                    if value
                        .as_ref()
                        .is_some_and(|value| integer_above_native(value, *bits, *signed))
                    {
                        errors.push(ValidationError::InvalidValue {
                            path: owner.into(),
                            message: format!("integer {name} is above its native range"),
                        });
                    }
                }
                if let (Some(minimum), Some(maximum)) = (minimum, maximum)
                    && integer_minimum_exceeds_maximum(minimum, maximum)
                {
                    errors.push(ValidationError::InvalidValue {
                        path: owner.into(),
                        message: "integer minimum exceeds maximum".into(),
                    });
                }
                if multiple_of
                    .as_ref()
                    .is_some_and(|value| !positive_number(value))
                {
                    errors.push(ValidationError::InvalidValue {
                        path: owner.into(),
                        message: "integer multiple_of must be greater than zero".into(),
                    });
                }
            }
            InlineType::Float { bits } if ![32, 64].contains(bits) => {
                errors.push(ValidationError::InvalidValue {
                    path: owner.into(),
                    message: format!("float width must be 32 or 64, got {bits}"),
                });
            }
            InlineType::String {
                min_length: Some(minimum),
                max_length: Some(maximum),
                ..
            } if minimum > maximum => errors.push(ValidationError::InvalidValue {
                path: owner.into(),
                message: "string min_length exceeds max_length".into(),
            }),
            InlineType::Opaque { codec, .. } if codec.trim().is_empty() => {
                errors.push(ValidationError::InvalidValue {
                    path: owner.into(),
                    message: "opaque codec cannot be empty".into(),
                });
            }
            InlineType::Map { key, .. }
                if !matches!(key.as_ref(), TypeRef::Inline(InlineType::String { .. })) =>
            {
                errors.push(ValidationError::InvalidValue {
                    path: owner.into(),
                    message: "map keys must use an inline string type".into(),
                });
            }
            InlineType::Nullable { .. }
            | InlineType::Array { .. }
            | InlineType::Map { .. }
            | InlineType::Tuple { .. } => {}
            _ => {}
        }
    }

    fn visit_operation_refs(
        &self,
        owner: &str,
        operation: &Operation,
        errors: &mut Vec<ValidationError>,
    ) {
        operation.visit_refs(&mut |reference| {
            self.validate_ref(owner, reference, errors);
            self.validate_inline(owner, reference, errors);
        });
    }

    fn validate_diagnostics(
        &self,
        owner: &str,
        diagnostics: &[Diagnostic],
        errors: &mut Vec<ValidationError>,
    ) {
        for diagnostic in diagnostics {
            if diagnostic.severity == DiagnosticSeverity::Error {
                errors.push(ValidationError::InvalidValue {
                    path: format!("{owner} diagnostics"),
                    message: format!(
                        "error diagnostic {}: {}",
                        diagnostic.code, diagnostic.message
                    ),
                });
            }
        }
    }

    fn validate_media_type(
        &self,
        owner: &str,
        label: &str,
        value: &str,
        errors: &mut Vec<ValidationError>,
    ) {
        if !crate::is_valid_media_type(value) {
            errors.push(ValidationError::InvalidValue {
                path: owner.into(),
                message: format!("{label} has invalid media type {value:?}"),
            });
        }
    }

    fn validate_error(
        &self,
        owner: &str,
        error: &ErrorResponse,
        body_forbidden: bool,
        errors: &mut Vec<ValidationError>,
    ) {
        if let ErrorStatus::Exact(status) = error.status
            && !(100..=599).contains(&status)
        {
            errors.push(ValidationError::InvalidValue {
                path: owner.into(),
                message: format!("invalid error status {status}"),
            });
        }
        if let ErrorStatus::Class(class) = error.status
            && !(1..=5).contains(&class)
        {
            errors.push(ValidationError::InvalidValue {
                path: owner.into(),
                message: format!("invalid HTTP status class {class}xx"),
            });
        }
        if error.body.is_some()
            && (body_forbidden
                || matches!(error.status, ErrorStatus::Class(1..=3))
                || matches!(error.status, ErrorStatus::Exact(status) if status_forbids_body(status)))
        {
            errors.push(ValidationError::InvalidValue {
                path: owner.into(),
                message: format!("error {:?} cannot carry a body", error.status),
            });
        }
        if (error.content == ErrorContent::Empty) == error.body.is_some() {
            errors.push(ValidationError::InvalidValue {
                path: owner.into(),
                message: format!("error content {:?} does not match its body", error.content),
            });
        }
        if error.content == ErrorContent::Empty && error.media_type.is_some() {
            errors.push(ValidationError::InvalidValue {
                path: owner.into(),
                message: "empty errors cannot declare a media type".into(),
            });
        }
        if let Some(media_type) = &error.media_type {
            self.validate_media_type(owner, "error body", media_type, errors);
        }
        let mut header_names = BTreeSet::new();
        for header in &error.headers {
            self.validate_header_name(
                owner,
                &format!("error {:?} header", error.status),
                &header.wire,
                &mut header_names,
                errors,
            );
        }
    }

    fn visit_error_refs(
        &self,
        owner: &str,
        error: &ErrorResponse,
        errors: &mut Vec<ValidationError>,
    ) {
        if let Some(body) = &error.body {
            body.visit_refs(&mut |reference| {
                self.validate_ref(owner, reference, errors);
                self.validate_inline(owner, reference, errors);
            });
        }
        for header in &error.headers {
            header.ty.visit_refs(&mut |reference| {
                self.validate_ref(owner, reference, errors);
                self.validate_inline(owner, reference, errors);
            });
        }
    }

    fn validate_ref(&self, owner: &str, reference: &TypeRef, errors: &mut Vec<ValidationError>) {
        let TypeRef::Named(named) = reference else {
            return;
        };
        let Some(definition) = self.types.get(&named.id) else {
            errors.push(ValidationError::UnknownType {
                owner: owner.into(),
                reference: named.id.clone(),
            });
            return;
        };
        if let Some(variant) = &named.variant {
            let exists = matches!(
                &definition.definition,
                TypeKind::Enum { variants, .. } if variants.iter().any(|candidate| candidate.wire == *variant)
            );
            if !exists {
                errors.push(ValidationError::InvalidValue {
                    path: owner.into(),
                    message: format!("type {} has no enum variant {variant}", named.id),
                });
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Info {
    pub title: String,
    pub version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub servers: Vec<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub ext: BTreeMap<String, Value>,
}

pub(crate) fn integer_magnitude(value: &Value) -> Option<(bool, u128)> {
    let text = value.as_number()?.to_string();
    let (negative, unsigned) = match text.strip_prefix('-') {
        Some(unsigned) => (true, unsigned),
        None => (false, text.as_str()),
    };
    let mantissa = match unsigned.split_once(['e', 'E']) {
        Some((mantissa, exponent)) => {
            if exponent.parse::<i32>().ok()? != 0 {
                return None;
            }
            mantissa
        }
        None => unsigned,
    };
    let (whole, fraction) = match mantissa.split_once('.') {
        Some((whole, fraction)) => (whole, fraction),
        None => (mantissa, ""),
    };
    if whole.is_empty() && fraction.is_empty() {
        return None;
    }
    let digits = format!("{whole}{fraction}");
    if !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let first_significant = digits.bytes().position(|byte| byte != b'0');
    let Some(first_significant) = first_significant else {
        return Some((false, 0));
    };
    let significant = &digits[first_significant..];
    let scale = i64::try_from(fraction.len()).ok()?;
    let magnitude = if scale <= 0 {
        let zeros = usize::try_from(-scale).ok()?;
        if significant.len().checked_add(zeros)? > 39 {
            return None;
        }
        let mut integer = String::with_capacity(significant.len() + zeros);
        integer.push_str(significant);
        integer.extend(std::iter::repeat_n('0', zeros));
        integer.parse().ok()?
    } else {
        let scale = usize::try_from(scale).ok()?;
        if scale >= significant.len()
            || significant[significant.len() - scale..]
                .bytes()
                .any(|byte| byte != b'0')
        {
            return None;
        }
        significant[..significant.len() - scale].parse().ok()?
    };
    Some((negative && magnitude != 0, magnitude))
}

fn positive_number(value: &Value) -> bool {
    if let Some((negative, magnitude)) = integer_magnitude(value) {
        return !negative && magnitude != 0;
    }
    value.as_f64().is_some_and(|number| number > 0.0)
}

fn integral_number(value: &Value) -> bool {
    value.as_f64().is_some_and(|number| number.fract() == 0.0)
}

fn integer_minimum_exceeds_maximum(minimum: &Value, maximum: &Value) -> bool {
    if let (Some((minimum_negative, minimum)), Some((maximum_negative, maximum))) =
        (integer_magnitude(minimum), integer_magnitude(maximum))
    {
        return match (minimum_negative, maximum_negative) {
            (false, true) => true,
            (true, false) => false,
            (false, false) => minimum > maximum,
            (true, true) => minimum < maximum,
        };
    }
    minimum
        .as_f64()
        .zip(maximum.as_f64())
        .is_some_and(|(minimum, maximum)| minimum > maximum)
}

fn integer_native_max(bits: u16) -> u128 {
    if bits == 128 {
        u128::MAX
    } else {
        (1_u128 << u32::from(bits)) - 1
    }
}

fn integer_below_native(value: &Value, bits: u16, signed: bool) -> bool {
    if !matches!(bits, 8 | 16 | 32 | 64 | 128) {
        return false;
    }
    if let Some((negative, magnitude)) = integer_magnitude(value) {
        return if signed {
            negative && magnitude > (1_u128 << (u32::from(bits) - 1))
        } else {
            negative || magnitude > integer_native_max(bits)
        };
    }
    let Some(number) = value.as_f64() else {
        return value
            .as_number()
            .is_some_and(|number| number.to_string().starts_with('-'));
    };
    if signed {
        number < -(2_f64.powi(i32::from(bits) - 1))
    } else {
        number < 0.0
    }
}

fn integer_above_native(value: &Value, bits: u16, signed: bool) -> bool {
    if !matches!(bits, 8 | 16 | 32 | 64 | 128) {
        return false;
    }
    if let Some((negative, magnitude)) = integer_magnitude(value) {
        return if signed {
            !negative && magnitude > ((1_u128 << (u32::from(bits) - 1)) - 1)
        } else {
            !negative && magnitude > integer_native_max(bits)
        };
    }
    let Some(number) = value.as_f64() else {
        return value
            .as_number()
            .is_some_and(|number| !number.to_string().starts_with('-'));
    };
    if signed {
        number >= 2_f64.powi(i32::from(bits) - 1)
    } else {
        number >= 2_f64.powi(i32::from(bits))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValidationError {
    UnsupportedVersion { found: String },
    DuplicateOperation { id: Vec<Vec<String>> },
    UnknownType { owner: String, reference: String },
    UnknownAuthScheme { owner: String, scheme: String },
    InvalidValue { path: String, message: String },
}

impl std::fmt::Display for ValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnsupportedVersion { found } => write!(f, "unsupported IR version {found}"),
            Self::DuplicateOperation { id } => write!(f, "duplicate operation id {id:?}"),
            Self::UnknownType { owner, reference } => {
                write!(f, "{owner} refers to unknown type {reference}")
            }
            Self::UnknownAuthScheme { owner, scheme } => {
                write!(f, "{owner} refers to unknown auth scheme {scheme}")
            }
            Self::InvalidValue { path, message } => write!(f, "{path}: {message}"),
        }
    }
}

impl std::error::Error for ValidationError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ByteEncoding, Direction, ObjectOpenness, OpaqueWire};

    #[test]
    fn rich_document_round_trips_and_validates() {
        let document: Document =
            serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
        document.validate().unwrap();
        let encoded = serde_json::to_value(&document).unwrap();
        assert_eq!(encoded["ext"]["source.note"]["kept"], true);
        assert_eq!(encoded["types"]["User"]["ext"]["source.type"], "kept");
        assert_eq!(encoded["auth_schemes"]["tenant"]["in"], "header");
        let decoded: Document = serde_json::from_value(encoded).unwrap();
        assert_eq!(document, decoded);
        assert_eq!(
            document.operations[0].errors[0].status,
            ErrorStatus::Class(4)
        );
    }

    #[test]
    fn opaque_values_require_a_codec_and_can_be_scanned() {
        let mut document: Document =
            serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
        let user = document.types.get_mut("User").unwrap();
        let TypeKind::Struct { fields, .. } = &mut user.definition else {
            panic!()
        };
        fields[0].ty = TypeRef::Inline(InlineType::Opaque {
            codec: String::new(),
            wire: OpaqueWire::Unknown,
        });
        let errors = document.validate().unwrap_err();
        assert!(
            errors
                .iter()
                .any(|error| error.to_string().contains("opaque codec cannot be empty"))
        );
        assert!(document.has_opaque());
        let TypeKind::Struct { fields, .. } =
            &mut document.types.get_mut("User").unwrap().definition
        else {
            panic!()
        };
        fields[0].ty = TypeRef::Inline(InlineType::Opaque {
            codec: "example".into(),
            wire: OpaqueWire::Json,
        });
        document.validate().unwrap();
    }

    #[test]
    fn common_errors_only_root_opaque_when_an_operation_inherits_them() {
        let mut document: Document =
            serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
        let mut common = document.operations[0].errors[0].clone();
        common.name = vec!["common".into(), "error".into()];
        common.body = Some(TypeRef::Inline(InlineType::Opaque {
            codec: "example".into(),
            wire: OpaqueWire::Json,
        }));
        document.common_errors.push(common);
        document.operations[0].errors.clear();
        document
            .operations
            .iter_mut()
            .for_each(|operation| operation.errors_inherit = false);
        document.validate().unwrap();
        assert!(document.has_opaque());
        assert!(!document.has_reachable_opaque());

        document.operations[0].errors_inherit = true;
        assert!(document.has_reachable_opaque());
    }

    #[test]
    fn rejects_nullable_flattened_types() {
        let mut document: Document =
            serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
        let TypeKind::Struct { flattened, .. } =
            &mut document.types.get_mut("User").unwrap().definition
        else {
            panic!()
        };
        flattened.push(TypeRef::Inline(InlineType::Nullable {
            inner: Box::new(TypeRef::Inline(InlineType::Bool)),
        }));
        assert!(
            document
                .validate()
                .unwrap_err()
                .iter()
                .any(|error| error.to_string().contains("nullable flattened"))
        );
    }

    #[test]
    fn rejects_required_nullable_headers_and_path_parameters() {
        fn fixture() -> Document {
            serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap()
        }
        fn nullable() -> TypeRef {
            TypeRef::Inline(InlineType::Nullable {
                inner: Box::new(TypeRef::Inline(InlineType::Bool)),
            })
        }
        fn rejects(document: &Document, message: &str) {
            let errors = document
                .validate()
                .unwrap_err()
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>();
            assert!(
                errors.iter().any(|error| error.contains(message)),
                "{message:?} missing from {errors:?}"
            );
        }

        let mut document = fixture();
        let watch = &mut document.operations[1];
        watch.headers[0].presence = Presence::Required;
        watch.headers[0].ty = nullable();
        rejects(
            &document,
            "required request header X-Request-ID cannot be nullable",
        );

        let mut document = fixture();
        document.types.insert(
            "MaybeHeader".into(),
            TypeDef {
                name: vec!["maybe".into(), "header".into()],
                direction: Direction::Input,
                definition: TypeKind::Newtype { inner: nullable() },
                origin: None,
                doc: None,
                diagnostics: Vec::new(),
                ext: BTreeMap::new(),
            },
        );
        let watch = &mut document.operations[1];
        watch.headers[0].presence = Presence::Required;
        watch.headers[0].ty = TypeRef::Named(crate::NamedRef {
            id: "MaybeHeader".into(),
            variant: None,
        });
        rejects(
            &document,
            "required request header X-Request-ID cannot be nullable",
        );

        let mut document = fixture();
        document.operations[1].path[1] = crate::operation::PathSegment::Param {
            param: "cursor".into(),
            ty: nullable(),
        };
        rejects(
            &document,
            "required path parameter cursor cannot be nullable",
        );

        let mut document = fixture();
        document.operations[1].headers[0].ty = nullable();
        document.validate().unwrap();
    }

    #[test]
    fn auth_scopes_validate_scheme_and_uniqueness() {
        let mut document: Document =
            serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
        document.operations[1]
            .auth
            .scopes
            .insert("missing".into(), vec!["read".into(), "read".into()]);
        let errors = document.validate().unwrap_err();
        assert!(
            errors
                .iter()
                .any(|error| error.to_string().contains("missing"))
        );
        assert!(
            errors
                .iter()
                .any(|error| error.to_string().contains("repeats scope read"))
        );
    }

    #[test]
    fn media_types_validate_and_empty_payloads_reject_them() {
        let mut document: Document =
            serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
        document.operations[0]
            .body
            .as_mut()
            .unwrap()
            .media_type
            .clone_from(&Some("application/vnd.acme+json; charset=utf-8".into()));
        document.validate().unwrap();
        document.operations[0].body.as_mut().unwrap().media_type = Some("not-a-media-type".into());
        document.operations[0].responses[0].content = ContentKind::Empty;
        document.operations[0].responses[0].body = None;
        document.operations[0].responses[0].media_type = Some("application/json".into());
        let errors = document.validate().unwrap_err();
        assert!(
            errors
                .iter()
                .any(|error| error.to_string().contains("invalid media type"))
        );
        assert!(
            errors
                .iter()
                .any(|error| error.to_string().contains("empty response"))
        );
    }

    #[test]
    fn byte_stream_media_types_are_consistent_and_validated() {
        let mut document: Document =
            serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
        let operation_index = document
            .operations
            .iter()
            .position(|operation| operation.stream.is_some())
            .unwrap();
        document.operations[operation_index]
            .stream
            .as_mut()
            .unwrap()
            .kind = crate::StreamKind::Bytes {
            content_type: "Application/PDF ; Version=1".into(),
        };
        document.validate().unwrap();
        document.operations[operation_index].responses[0].media_type =
            Some("application/pdf; version=2".into());
        assert!(document.validate().unwrap_err().iter().any(|error| {
            error
                .to_string()
                .contains("does not match stream kind media type")
        }));
        document.operations[operation_index].responses[0].media_type =
            Some("application/pdf; version=1; extra=value".into());
        document.validate().unwrap();
        document.operations[operation_index]
            .stream
            .as_mut()
            .unwrap()
            .kind = crate::StreamKind::Bytes {
            content_type: "not-a-media-type".into(),
        };
        assert!(document.validate().unwrap_err().iter().any(|error| {
            error
                .to_string()
                .contains("invalid byte stream content type")
        }));
    }

    #[test]
    fn sse_and_ndjson_media_types_are_checked_against_their_kind() {
        let mut document: Document =
            serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
        let operation_index = document
            .operations
            .iter()
            .position(|operation| operation.stream.is_some())
            .unwrap();
        for (kind, expected) in [
            (
                crate::StreamKind::Sse { events: vec![] },
                "text/event-stream",
            ),
            (
                crate::StreamKind::Ndjson {
                    item: TypeRef::Inline(InlineType::Bool),
                },
                "application/x-ndjson",
            ),
        ] {
            document.operations[operation_index]
                .stream
                .as_mut()
                .unwrap()
                .kind = kind;

            for accepted in [
                expected.to_owned(),
                format!("{expected}; charset=utf-8"),
                format!("{expected} ; CharSet=UTF-8"),
            ] {
                document.operations[operation_index].responses[0].media_type = Some(accepted);
                document.validate().unwrap();
            }
            for rejected in ["application/json", "application/octet-stream", "text/plain"] {
                document.operations[operation_index].responses[0].media_type =
                    Some(rejected.into());
                assert!(
                    document.validate().unwrap_err().iter().any(|error| error
                        .to_string()
                        .contains("does not match stream kind media type")),
                    "{rejected:?} must not serve an {expected} stream"
                );
            }
        }
    }

    #[test]
    fn object_openness_is_explicit_and_typed_references_are_validated() {
        let mut document: Document =
            serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
        let user = document.types.get_mut("User").unwrap();
        let TypeKind::Struct {
            fields, flattened, ..
        } = &user.definition
        else {
            unreachable!()
        };
        user.definition = TypeKind::Struct {
            fields: fields.clone(),
            flattened: flattened.clone(),
            openness: ObjectOpenness::PreserveTyped {
                value: TypeRef::Named(crate::NamedRef {
                    id: "Missing".into(),
                    variant: None,
                }),
            },
        };
        let errors = document.validate().unwrap_err();
        assert!(
            errors
                .iter()
                .any(|error| error.to_string().contains("Missing"))
        );
        let encoded = serde_json::to_value(&document).unwrap();
        assert_eq!(
            encoded["types"]["User"]["openness"]["preserve_typed"]["value"]["ref"],
            "Missing"
        );
    }

    #[test]
    fn response_status_variants_round_trip_and_validate() {
        let mut document: Document =
            serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
        document.operations[0].responses[0].status = ResponseStatus::Class(2);
        document.validate().unwrap();
        let encoded = serde_json::to_value(&document).unwrap();
        assert_eq!(encoded["operations"][0]["responses"][0]["status"], "2xx");
        document.operations[0].responses[0].status = ResponseStatus::Unknown;
        document.validate().unwrap();
        let encoded = serde_json::to_value(&document).unwrap();
        assert_eq!(
            encoded["operations"][0]["responses"][0]["status"],
            "unknown"
        );
        document.operations[0].responses[0].status = ResponseStatus::Default;
        document.validate().unwrap();
        let encoded = serde_json::to_value(&document).unwrap();
        assert_eq!(
            encoded["operations"][0]["responses"][0]["status"],
            "default"
        );
        document.operations[0].responses[0].status = ResponseStatus::Class(6);
        assert!(document.validate().is_err());
    }

    #[test]
    fn error_diagnostics_block_document_validation() {
        let mut document: Document =
            serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
        document.operations[0].diagnostics.push(Diagnostic {
            severity: DiagnosticSeverity::Error,
            code: "PONS-E999".into(),
            message: "unsupported boundary".into(),
            origin: None,
            ext: BTreeMap::new(),
        });
        assert!(
            document
                .validate()
                .unwrap_err()
                .iter()
                .any(|error| error.to_string().contains("PONS-E999"))
        );
    }

    #[test]
    fn validation_finds_nested_unknown_types_and_bad_contract_links() {
        let json = r#"{
            "ir_version": "0.2",
            "info": {"title": "Example", "version": "1"},
            "types": {"Items": {"kind": "struct", "openness": "reject", "name": ["items"], "direction": "both", "fields": [
                {"wire": "items", "ident": ["items"], "ty": {"kind": "array", "item": {"ref": "Missing"}}, "presence": "required"}
            ]}},
            "operations": [{
                "id": [["items"], ["get"]], "method": "GET", "path": [],
                "responses": [{"status": 200, "content": "empty", "headers": [
                    {"wire": "X-Missing", "ty": {"ref": "MissingHeader"}, "presence": "required"}
                ]}],
                "auth": {"alternatives": [["missing_scheme"]], "optional": false},
                "semantics": {"safe": true, "idempotent": true}
            }, {
                "id": [["items"], ["get"]], "method": "GET", "path": [],
                "auth": {"alternatives": [], "optional": false},
                "semantics": {"safe": true, "idempotent": true}
            }]
        }"#;

        let document: Document = serde_json::from_str(json).unwrap();
        let errors = document.validate().unwrap_err();
        assert!(errors.iter().any(|error| matches!(error, ValidationError::UnknownType { reference, .. } if reference == "Missing")));
        assert!(errors.iter().any(|error| matches!(error, ValidationError::UnknownType { reference, .. } if reference == "MissingHeader")));
        assert!(errors.iter().any(|error| matches!(error, ValidationError::UnknownAuthScheme { scheme, .. } if scheme == "missing_scheme")));
        assert!(
            errors
                .iter()
                .any(|error| matches!(error, ValidationError::DuplicateOperation { .. }))
        );
    }

    #[test]
    fn validation_rejects_inconsistent_payload_header_and_numeric_contracts() {
        let json = r#"{
            "ir_version": "0.2",
            "info": {"title": "Example", "version": "1"},
            "types": {
                "Bad": {
                    "kind": "struct",
                    "openness": "reject",
                    "name": ["bad"],
                    "direction": "both",
                    "fields": [{
                        "wire": "value",
                        "ident": ["value"],
                        "ty": {
                            "kind": "int",
                            "bits": 32,
                            "signed": true,
                            "wire": "number",
                            "minimum": "zero",
                            "maximum": null,
                            "multiple_of": 0
                        },
                        "presence": "required"
                    }, {
                        "wire": "bounded",
                        "ident": ["bounded"],
                        "ty": {
                            "kind": "int",
                            "bits": 8,
                            "signed": false,
                            "wire": "number",
                            "minimum": -1,
                            "maximum": 300
                        },
                        "presence": "required"
                    }]
                },
                "Empty": {
                    "kind": "enum",
                    "name": ["empty"],
                    "direction": "both",
                    "repr": {"kind": "string"},
                    "variants": []
                }
            },
            "operations": [{
                "id": [["test"]],
                "method": "POST",
                "path": [],
                "headers": [
                    {"wire": "X-Trace", "ident": ["trace"], "ty": {"kind": "string"}, "presence": "optional"},
                    {"wire": "x-trace", "ident": ["trace"], "ty": {"kind": "string"}, "presence": "optional"},
                    {"wire": "bad header", "ident": ["bad"], "ty": {"kind": "string"}, "presence": "optional"}
                ],
                "body": {"ty": {"kind": "string"}, "content": "empty", "required": true},
                "responses": [
                    {"status": 200, "content": "json"},
                    {"status": 201, "content": "empty", "body": {"kind": "string"}},
                    {"status": 202, "content": "stream"}
                ],
                "errors": [{
                    "status": 400,
                    "name": ["bad"],
                    "content": "empty",
                    "body": {"kind": "string"}
                }],
                "auth": {"alternatives": [], "optional": false},
                "semantics": {"safe": false, "idempotent": false}
            }]
        }"#;

        let document: Document = serde_json::from_str(json).unwrap();
        let errors = document.validate().unwrap_err();
        let messages = errors
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(messages.contains("integer minimum must be a number"));
        assert!(messages.contains("integer multiple_of must be greater than zero"));
        assert!(messages.contains("integer minimum is below its native range"));
        assert!(messages.contains("integer maximum is above its native range"));
        assert!(messages.contains("enum needs at least one variant"));
        assert!(messages.contains("request body content Empty"));
        assert!(messages.contains("duplicate request header x-trace"));
        assert!(messages.contains("not a valid HTTP field name"));
        assert!(messages.contains("response 200 content Json"));
        assert!(messages.contains("response 201 content Empty"));
        assert!(messages.contains("0 stream descriptions and 1 stream responses"));
        assert!(messages.contains("error content Empty"));
    }

    #[test]
    fn byte_bodies_must_resolve_to_a_byte_value() {
        const REQUEST: &str =
            "raw byte bodies must use a byte type, byte newtype, nullable bytes, or a bytes opaque";
        const RESPONSE: &str = "raw byte response bodies must use a byte type, byte newtype, nullable bytes, or a bytes opaque";

        fn bytes() -> TypeRef {
            TypeRef::Inline(InlineType::Bytes {
                encoding: ByteEncoding::Base64,
            })
        }
        fn nullable(inner: TypeRef) -> TypeRef {
            TypeRef::Inline(InlineType::Nullable {
                inner: Box::new(inner),
            })
        }
        fn opaque(wire: OpaqueWire) -> TypeRef {
            TypeRef::Inline(InlineType::Opaque {
                codec: "Raw".into(),
                wire,
            })
        }
        fn string() -> TypeRef {
            TypeRef::Inline(InlineType::String {
                format: None,
                min_length: None,
                max_length: None,
                pattern: None,
            })
        }
        fn named(id: &str) -> TypeRef {
            TypeRef::Named(crate::NamedRef {
                id: id.into(),
                variant: None,
            })
        }
        fn newtype(document: &mut Document, id: &str, inner: TypeRef) -> TypeRef {
            document.types.insert(
                id.into(),
                TypeDef {
                    name: vec![id.to_ascii_lowercase()],
                    direction: Direction::Both,
                    definition: TypeKind::Newtype { inner },
                    origin: None,
                    doc: None,
                    diagnostics: Vec::new(),
                    ext: BTreeMap::new(),
                },
            );
            named(id)
        }
        fn rejects(document: &Document, message: &str) {
            let errors = document
                .validate()
                .unwrap_err()
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>();
            assert!(
                errors.iter().any(|error| error.contains(message)),
                "{message:?} missing from {errors:?}"
            );
        }

        let mut document: Document =
            serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
        let login = &mut document.operations[0];
        login.body.as_mut().unwrap().content = ContentKind::Bytes;
        login.body.as_mut().unwrap().ty = bytes();
        login.responses[0].content = ContentKind::Bytes;
        login.responses[0].body = Some(bytes());
        document.validate().unwrap();

        let mut document = document.clone();
        document.operations[0].body.as_mut().unwrap().ty = string();
        rejects(&document, REQUEST);
        document.operations[0].body.as_mut().unwrap().ty = named("User");
        rejects(&document, REQUEST);
        document.operations[0].body.as_mut().unwrap().ty = newtype(&mut document, "Text", string());
        rejects(&document, REQUEST);

        let mut document = document.clone();
        document.operations[0].responses[0].body = Some(named("User"));
        rejects(&document, RESPONSE);

        let raw = newtype(&mut document, "RawBytes", bytes());
        let nested = newtype(&mut document, "NestedBytes", raw);
        document.operations[0].body.as_mut().unwrap().ty = nested.clone();
        document.operations[0].responses[0].body = Some(nested);
        document.validate().unwrap();

        for accepted in [
            nullable(bytes()),
            newtype(&mut document, "MaybeBytes", nullable(bytes())),
            opaque(OpaqueWire::Bytes),
            nullable(opaque(OpaqueWire::Bytes)),
            newtype(&mut document, "OpaqueBytes", opaque(OpaqueWire::Bytes)),
        ] {
            let mut document = document.clone();
            document.operations[0].body.as_mut().unwrap().ty = accepted.clone();
            document.operations[0].responses[0].body = Some(accepted);
            document.validate().unwrap();
        }

        for rejected in [
            nullable(string()),
            nullable(named("User")),
            opaque(OpaqueWire::Text),
            opaque(OpaqueWire::Json),
            opaque(OpaqueWire::Unknown),
            newtype(&mut document, "OpaqueText", opaque(OpaqueWire::Text)),
        ] {
            let mut document = document.clone();
            document.operations[0].body.as_mut().unwrap().ty = rejected.clone();
            rejects(&document, REQUEST);
            document.operations[0].responses[0].body = Some(rejected);
            rejects(&document, RESPONSE);
        }

        document.operations[0].body.as_mut().unwrap().ty =
            newtype(&mut document, "Loop", named("Loop"));
        rejects(&document, REQUEST);
    }

    fn string() -> TypeRef {
        TypeRef::Inline(InlineType::String {
            format: None,
            min_length: None,
            max_length: None,
            pattern: None,
        })
    }

    fn array(item: TypeRef) -> TypeRef {
        TypeRef::Inline(InlineType::Array {
            item: Box::new(item),
            unique: None,
        })
    }

    fn nullable(inner: TypeRef) -> TypeRef {
        TypeRef::Inline(InlineType::Nullable {
            inner: Box::new(inner),
        })
    }

    fn map(value: TypeRef) -> TypeRef {
        TypeRef::Inline(InlineType::Map {
            key: Box::new(string()),
            value: Box::new(value),
        })
    }

    fn named(id: &str) -> TypeRef {
        TypeRef::Named(crate::NamedRef {
            id: id.into(),
            variant: None,
        })
    }

    fn named_type(id: &'static str, definition: TypeKind) -> (&'static str, TypeDef) {
        (
            id,
            TypeDef {
                name: vec![id.into()],
                direction: Direction::Both,
                definition,
                origin: None,
                doc: None,
                diagnostics: Vec::new(),
                ext: BTreeMap::new(),
            },
        )
    }

    fn profile() -> (&'static str, TypeDef) {
        named_type(
            "Profile",
            TypeKind::Struct {
                fields: Vec::new(),
                flattened: Vec::new(),
                openness: ObjectOpenness::Reject,
            },
        )
    }

    fn form_document(fields: Vec<(&str, TypeRef)>, extra: Vec<(&str, TypeDef)>) -> Document {
        form_document_with(Presence::Required, fields, extra)
    }

    fn form_document_with(
        presence: Presence,
        fields: Vec<(&str, TypeRef)>,
        extra: Vec<(&str, TypeDef)>,
    ) -> Document {
        let mut document: Document =
            serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
        for (id, definition) in extra {
            document.types.insert(id.into(), definition);
        }
        document.types.insert(
            "FormBody".into(),
            TypeDef {
                name: vec!["form".into(), "body".into()],
                direction: Direction::Input,
                definition: TypeKind::Struct {
                    fields: fields
                        .into_iter()
                        .map(|(wire, ty)| Field {
                            wire: wire.into(),
                            ident: vec![wire.into()],
                            ty,
                            presence,
                            default: crate::FieldDefault::Unspecified,
                            access: None,
                            doc: None,
                            diagnostics: Vec::new(),
                            ext: BTreeMap::new(),
                        })
                        .collect(),
                    flattened: Vec::new(),
                    openness: ObjectOpenness::Reject,
                },
                origin: None,
                doc: None,
                diagnostics: Vec::new(),
                ext: BTreeMap::new(),
            },
        );
        let body = document.operations[0].body.as_mut().unwrap();
        body.content = ContentKind::Form;
        body.ty = named("FormBody");
        document
    }

    fn rejects_nested_form_values(document: &Document) {
        let errors = document
            .validate()
            .unwrap_err()
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>();
        assert!(
            errors
                .iter()
                .any(|error| error.contains("form bodies must be a flat record")),
            "a nested form value must be rejected: {errors:?}"
        );
    }

    #[test]
    fn form_bodies_reuse_the_query_scalar_rule_and_reject_nested_objects() {
        for (fields, extra, body) in [
            (vec![("email", string())], vec![], None),
            (
                vec![("email", string())],
                vec![named_type(
                    "Wrapped",
                    TypeKind::Newtype {
                        inner: named("FormBody"),
                    },
                )],
                Some(named("Wrapped")),
            ),
            (
                vec![],
                vec![],
                Some(TypeRef::Inline(InlineType::Opaque {
                    codec: "axum.form".into(),
                    wire: OpaqueWire::Unknown,
                })),
            ),
        ] {
            let mut document = form_document(fields, extra);
            if let Some(body) = body {
                document.operations[0].body.as_mut().unwrap().ty = body;
            }
            document.validate().unwrap();
        }

        for (fields, extra) in [
            (vec![("profile", named("Profile"))], vec![profile()]),
            (vec![("profile", map(string()))], vec![]),
            (vec![("email", nullable(string()))], vec![]),
            (
                vec![("profile", named("WrappedProfile"))],
                vec![
                    profile(),
                    named_type(
                        "WrappedProfile",
                        TypeKind::Newtype {
                            inner: named("Profile"),
                        },
                    ),
                ],
            ),
        ] {
            rejects_nested_form_values(&form_document(fields, extra));
        }

        form_document_with(
            Presence::Optional,
            vec![("email", nullable(string()))],
            vec![],
        )
        .validate()
        .unwrap();
    }

    #[test]
    fn form_bodies_add_one_array_level_to_the_query_scalar_rule() {
        form_document(vec![("tags", array(string()))], vec![])
            .validate()
            .unwrap();
        form_document_with(
            Presence::Optional,
            vec![("tags", nullable(array(string())))],
            vec![],
        )
        .validate()
        .unwrap();

        for (fields, extra) in [
            (vec![("matrix", array(array(string())))], vec![]),
            (vec![("profiles", array(named("Profile")))], vec![profile()]),
            (vec![("tags", nullable(array(string())))], vec![]),
        ] {
            rejects_nested_form_values(&form_document(fields, extra));
        }
    }

    #[test]
    fn status_defaults_and_classes_keep_their_wire_form() {
        let status: ErrorStatus = serde_json::from_str("\"default\"").unwrap();
        assert_eq!(serde_json::to_string(&status).unwrap(), "\"default\"");
        let status: ErrorStatus = serde_json::from_str("\"5xx\"").unwrap();
        assert_eq!(serde_json::to_string(&status).unwrap(), "\"5xx\"");
    }

    #[test]
    fn checked_in_schema_matches_the_rust_model() {
        let generated = serde_json::to_value(schemars::schema_for!(Document)).unwrap();
        let checked_in: Value =
            serde_json::from_str(include_str!("../schema/ir.schema.json")).unwrap();
        assert_eq!(generated, checked_in);
    }

    #[test]
    fn canonical_json_ignores_order_of_unordered_contract_members() {
        let document: Document =
            serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
        let mut shuffled = document.clone();
        shuffled.operations.reverse();
        if let TypeKind::Struct { fields, .. } =
            &mut shuffled.types.get_mut("User").unwrap().definition
        {
            fields.reverse();
        }

        assert_ne!(
            serde_json::to_string(&document).unwrap(),
            serde_json::to_string(&shuffled).unwrap()
        );
        assert_eq!(
            document.to_canonical_json().unwrap(),
            shuffled.to_canonical_json().unwrap()
        );
        assert_eq!(shuffled.operations[0].id[0], ["events"]);
    }
}
