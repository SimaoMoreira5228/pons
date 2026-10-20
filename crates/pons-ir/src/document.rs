use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::IR_VERSION;
use crate::operation::{
    ApiKeyLocation, AuthScheme, ContentKind, CredentialLocator, ErrorContent, ErrorResponse,
    ErrorStatus, Method, OAuthFlowKind, Operation, PathSegment, ResponseHeader, ResponseStatus,
    StreamKind,
};
use crate::types::{
    Access, Diagnostic, DiagnosticSeverity, Direction, Field, InlineType, IntegerWire,
    ObjectOpenness, Presence, TypeDef, TypeKind, TypeRef, VariantPayload,
};
use crate::violation::direction_name;

mod types;

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

fn response_answers_success(response: ResponseStatus) -> bool {
    match response {
        ResponseStatus::Exact(status) => (200..300).contains(&status),
        ResponseStatus::Class(class) => class == 2,
        ResponseStatus::Default | ResponseStatus::Unknown => true,
    }
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
            response_answers_success(ResponseStatus::Exact(*status))
        }
        (ResponseStatus::Default | ResponseStatus::Unknown, ErrorStatus::Class(class)) => {
            response_answers_success(ResponseStatus::Class(*class))
        }
        (ResponseStatus::Default | ResponseStatus::Unknown, ErrorStatus::Default) => true,
        _ => false,
    }
}

fn owns_request_header(wire: &str, has_body: bool) -> bool {
    match wire.to_ascii_lowercase().as_str() {
        "content-type" => has_body,
        "accept" | "authorization" | "cookie" => true,
        _ => false,
    }
}

fn absolute_https_url(value: &str) -> bool {
    value.strip_prefix("https://").is_some_and(|rest| {
        let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
        let authority = &rest[..end];
        let host = authority.rsplit('@').next().unwrap_or_default();
        !host.is_empty() && !rest.chars().any(char::is_whitespace)
    })
}

#[derive(Clone, Copy)]
enum Encoding {
    Json,
    Form,
    Multipart,
    Text,
    Bytes,
    Any,
}

impl Encoding {
    fn name(self) -> &'static str {
        match self {
            Self::Json => "json",
            Self::Form => "form",
            Self::Multipart => "multipart",
            Self::Text => "text",
            Self::Bytes => "bytes",
            Self::Any => "declared",
        }
    }

    fn names(self, media_type: &str) -> bool {
        let base = media_type
            .split(';')
            .next()
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase();
        let json = base == "application/json" || base.ends_with("+json");
        match self {
            Self::Json => json,
            Self::Form => base == "application/x-www-form-urlencoded",
            Self::Multipart => base.starts_with("multipart/"),
            Self::Text => base.starts_with("text/"),
            Self::Bytes => {
                !(json
                    || base == "application/x-www-form-urlencoded"
                    || base.starts_with("multipart/"))
            }
            Self::Any => true,
        }
    }
}

fn encoding(content: ContentKind) -> Encoding {
    match content {
        ContentKind::Json | ContentKind::ProblemJson => Encoding::Json,
        ContentKind::Form => Encoding::Form,
        ContentKind::Multipart => Encoding::Multipart,
        ContentKind::Text => Encoding::Text,
        ContentKind::Bytes => Encoding::Bytes,
        ContentKind::Stream | ContentKind::Empty => Encoding::Any,
    }
}

fn error_encoding(content: ErrorContent) -> Encoding {
    match content {
        ErrorContent::Json | ErrorContent::ProblemJson => Encoding::Json,
        ErrorContent::Text => Encoding::Text,
        ErrorContent::Empty => Encoding::Any,
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
                    for (kind, url) in [
                        ("authorization", flow.authorization_url.as_deref()),
                        ("token", flow.token_url.as_deref()),
                    ] {
                        if let Some(url) = url
                            && !absolute_https_url(url)
                        {
                            errors.push(ValidationError::InvalidValue {
                                path: owner.clone(),
                                message: format!(
                                    "OAuth2 {kind} URL for {id} must be an absolute HTTPS URL"
                                ),
                            });
                        }
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
            if let Some(shadowing) =
                self.common_errors
                    .iter()
                    .enumerate()
                    .find(|(other, candidate)| {
                        *other != index && candidate.status.shadows(&error.status)
                    })
            {
                errors.push(ValidationError::InvalidValue {
                    path: "common_errors".into(),
                    message: format!(
                        "error dispatch slot {:?} is shadowed: the common error {:?} already answers for it",
                        error.status, shadowing.1.status
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
                    if let Err(refusal) = crate::query::path_fields(self, param, ty) {
                        errors.push(ValidationError::InvalidValue {
                            path: owner.clone(),
                            message: refusal.in_carrier(&crate::query::PATH),
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
                if owns_request_header(&header.wire, operation.body.is_some()) {
                    errors.push(ValidationError::InvalidValue {
                        path: owner.clone(),
                        message: format!(
                            "request header {} is owned by the request body or the security model, so every client and server built from the document writes it too",
                            header.wire
                        ),
                    });
                }
            }
            if let Some(body) = &operation.body {
                if body.content == ContentKind::Stream {
                    errors.push(ValidationError::InvalidValue {
                        path: owner.clone(),
                        message: "a request body cannot be stream content, because no built-in target reads one"
                            .into(),
                    });
                }
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
                    self.validate_media_type(
                        &owner,
                        "request body",
                        encoding(body.content),
                        media_type,
                        &mut errors,
                    );
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
                if body.content == ContentKind::Form
                    && let Err(refusal) = crate::query::form_body_fields(self, &body.ty)
                {
                    errors.push(ValidationError::InvalidValue {
                        path: owner.clone(),
                        message: refusal.in_carrier(&crate::query::FORM),
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
            if let Some(pagination) = &operation.semantics.pagination {
                if let Err(message) = pagination.validate() {
                    errors.push(ValidationError::InvalidValue {
                        path: owner.clone(),
                        message,
                    });
                }
                for field in std::iter::once(pagination.request.field())
                    .chain(pagination.page_size.iter().map(|size| size.field.as_str()))
                {
                    if !self.request_field_declared(operation, field) {
                        errors.push(ValidationError::InvalidValue {
                            path: owner.clone(),
                            message: format!(
                                "pagination field {field:?} is not declared on the operation query or body"
                            ),
                        });
                    }
                }
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
                if let Some(AuthScheme::OAuth2 { flows }) = self.auth_schemes.get(scheme) {
                    for scope in scopes {
                        if !flows.iter().any(|flow| flow.scopes.contains_key(scope)) {
                            errors.push(ValidationError::InvalidValue {
                                path: owner.clone(),
                                message: format!(
                                    "the OAuth2 scheme {scheme} does not declare the {scope} scope"
                                ),
                            });
                        }
                    }
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
                        encoding(response.content),
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
                if response.content == ContentKind::Stream
                    || (operation
                        .stream
                        .as_ref()
                        .is_some_and(|stream| matches!(stream.kind, StreamKind::Websocket { .. }))
                        && response.status == ResponseStatus::Exact(101)
                        && response.content == ContentKind::Empty)
                {
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
                        .find(|common| common.status.shadows(&error.status))
                {
                    errors.push(ValidationError::InvalidValue {
                        path: owner.clone(),
                        message: format!(
                            "error dispatch slot {:?} is inherited: the common error {:?} already answers for it",
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
        self.validate_credential_carriers(&mut errors);
        self.validate_extensions(&mut errors);

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }

    fn request_field_declared(&self, operation: &Operation, wire: &str) -> bool {
        if operation.query.as_ref().is_some_and(|query| {
            crate::query::query_fields(self, query)
                .is_ok_and(|fields| fields.iter().any(|field| field.wire == wire))
        }) {
            return true;
        }
        operation.body.as_ref().is_some_and(|body| {
            self.struct_field_named(&body.ty, wire, &mut BTreeSet::new())
        })
    }

    fn validate_credential_carriers(&self, errors: &mut Vec<ValidationError>) {
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

    fn validate_extensions(&self, errors: &mut Vec<ValidationError>) {
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
        written: Encoding,
        value: &str,
        errors: &mut Vec<ValidationError>,
    ) {
        if !crate::is_valid_media_type(value) {
            errors.push(ValidationError::InvalidValue {
                path: owner.into(),
                message: format!("{label} has invalid media type {value:?}"),
            });
        } else if !written.names(value) {
            errors.push(ValidationError::InvalidValue {
                path: owner.into(),
                message: format!(
                    "{label} has media type {value:?}, which does not name the {} body the client writes",
                    written.name()
                ),
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
        if error.name.is_empty() || error.name.iter().any(String::is_empty) {
            errors.push(ValidationError::InvalidValue {
                path: owner.into(),
                message: "error name must contain non-empty name segments".into(),
            });
        }
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
            self.validate_media_type(
                owner,
                "error body",
                error_encoding(error.content),
                media_type,
                errors,
            );
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecodeError {
    Json(String),
    UnsupportedVersion {
        found: String,
        supported: &'static str,
    },
    Member {
        path: String,
        message: String,
    },
}

impl std::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Json(message) => write!(f, "invalid JSON: {message}"),
            Self::UnsupportedVersion { found, supported } => write!(
                f,
                "unsupported IR version {found}, this build reads {supported}"
            ),
            Self::Member { path, message } if path.is_empty() => write!(f, "{message}"),
            Self::Member { path, message } => write!(f, "{path}: {message}"),
        }
    }
}

impl std::error::Error for DecodeError {}

pub fn decode(bytes: &[u8]) -> Result<Document, DecodeError> {
    let value: Value =
        serde_json::from_slice(bytes).map_err(|error| DecodeError::Json(error.to_string()))?;
    let Some(found) = value.get("ir_version").and_then(Value::as_str) else {
        return Err(DecodeError::Member {
            path: "ir_version".into(),
            message: "the document does not carry a string ir_version".into(),
        });
    };
    if found != IR_VERSION {
        return Err(DecodeError::UnsupportedVersion {
            found: found.to_owned(),
            supported: IR_VERSION,
        });
    }
    serde_json::from_slice::<Document>(bytes).map_err(|error| {
        let text = error.to_string();
        let message = text.split(" at line ").next().unwrap_or(&text);
        DecodeError::Member {
            path: member_path(bytes, error.line(), error.column()),
            message: message.to_owned(),
        }
    })
}

fn member_path(bytes: &[u8], line: usize, column: usize) -> String {
    enum Part {
        Key(String),
        Index(usize),
    }

    struct Frame {
        array: bool,
        index: usize,
        key: Option<String>,
    }

    let mut frames: Vec<Frame> = Vec::new();
    let mut at = 0;
    let target = byte_offset(bytes, line, column);
    while at < target {
        match bytes[at] {
            b'{' | b'[' => frames.push(Frame {
                array: bytes[at] == b'[',
                index: 0,
                key: None,
            }),
            b'}' | b']' => {
                frames.pop();
            }
            b',' => {
                if let Some(frame) = frames.last_mut() {
                    frame.index += 1;
                    frame.key = None;
                }
            }
            b':' | b' ' | b'\t' | b'\n' | b'\r' => {}
            b'"' => {
                let (text, end) = string_at(bytes, at);
                let is_key = bytes[end..]
                    .iter()
                    .find(|byte| !byte.is_ascii_whitespace())
                    .is_some_and(|byte| *byte == b':');
                if is_key && let Some(frame) = frames.last_mut() {
                    frame.key = Some(text);
                }
                at = end;
                continue;
            }
            _ => {}
        }
        at += 1;
    }

    let mut parts: Vec<Part> = Vec::new();
    for frame in frames {
        if frame.array {
            parts.push(Part::Index(frame.index));
        } else if let Some(key) = frame.key {
            parts.push(Part::Key(key));
        }
    }

    if parts.len() > 2 && matches!(&parts[0], Part::Key(key) if key == "types") {
        parts.truncate(2);
    }

    let mut path = String::new();
    for part in parts {
        match part {
            Part::Index(index) => path.push_str(&format!("[{index}]")),
            Part::Key(key) => {
                if !path.is_empty() {
                    path.push('.');
                }
                path.push_str(&key);
            }
        }
    }
    path
}

fn byte_offset(bytes: &[u8], line: usize, column: usize) -> usize {
    let mut offset = 0;
    for _ in 1..line.max(1) {
        match bytes[offset..].iter().position(|byte| *byte == b'\n') {
            Some(at) => offset += at + 1,
            None => return bytes.len(),
        }
    }
    offset + column.saturating_sub(1).min(bytes.len() - offset)
}

fn string_at(bytes: &[u8], start: usize) -> (String, usize) {
    let mut at = start + 1;
    while let Some(byte) = bytes.get(at) {
        match byte {
            b'\\' => at += 2,
            b'"' => {
                let raw = String::from_utf8_lossy(&bytes[start..=at]).into_owned();
                let text = if raw.contains('\\') {
                    serde_json::from_str(&raw).unwrap_or_default()
                } else {
                    raw[1..raw.len() - 1].to_owned()
                };
                return (text, at + 1);
            }
            _ => at += 1,
        }
    }
    (String::new(), at)
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
