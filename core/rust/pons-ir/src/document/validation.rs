use std::collections::BTreeSet;

use crate::IR_VERSION;
use crate::media::MediaDescriptor;
use crate::operation::{
    ApiKeyLocation, AuthScheme, ContentKind, CredentialLocator, ErrorContent, ErrorResponse,
    ErrorStatus, Method, OAuthFlowKind, Operation, ResponseStatus, StreamKind,
};
use crate::types::{Diagnostic, DiagnosticSeverity, Presence};

use super::{Document, ValidationError};

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

    fn names(self, media: &MediaDescriptor) -> bool {
        let base = media.base();
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

impl Document {
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
                if body.content == ContentKind::Empty && body.media.is_some() {
                    errors.push(ValidationError::InvalidValue {
                        path: owner.clone(),
                        message: "empty request bodies cannot declare a media type".into(),
                    });
                }
                if let Some(media) = &body.media {
                    self.validate_media(
                        &owner,
                        "request body",
                        encoding(body.content),
                        media,
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
                if response.content == ContentKind::Empty && response.media.is_some() {
                    errors.push(ValidationError::InvalidValue {
                        path: owner.clone(),
                        message: format!(
                            "empty response {} cannot declare a media type",
                            response.status
                        ),
                    });
                }
                if let Some(media) = &response.media {
                    self.validate_media(
                        &owner,
                        &format!("response {} body", response.status),
                        encoding(response.content),
                        media,
                        &mut errors,
                    );
                }
                if response.content == ContentKind::Stream
                    && let Some(media) = &response.media
                    && let Some(content_type) = operation
                        .stream
                        .as_ref()
                        .and_then(|stream| stream.kind.media_type())
                    && !crate::media::media_type_matches(&media.to_string(), content_type)
                {
                    errors.push(ValidationError::InvalidValue {
                        path: owner.clone(),
                        message: format!(
                            "response media type {media} does not match stream kind media type {content_type:?}"
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
        operation
            .body
            .as_ref()
            .is_some_and(|body| self.struct_field_named(&body.ty, wire, &mut BTreeSet::new()))
    }

    pub(super) fn validate_diagnostics(
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

    fn validate_media(
        &self,
        owner: &str,
        label: &str,
        written: Encoding,
        media: &MediaDescriptor,
        errors: &mut Vec<ValidationError>,
    ) {
        if let Err(message) = media.validate() {
            errors.push(ValidationError::InvalidValue {
                path: owner.into(),
                message: format!("{label} has {message}"),
            });
        } else if !written.names(media) {
            errors.push(ValidationError::InvalidValue {
                path: owner.into(),
                message: format!(
                    "{label} has media type {media}, which does not name the {} body the client writes",
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
        if error.content == ErrorContent::Empty && error.media.is_some() {
            errors.push(ValidationError::InvalidValue {
                path: owner.into(),
                message: "empty errors cannot declare a media type".into(),
            });
        }
        if let Some(media) = &error.media {
            self.validate_media(
                owner,
                "error body",
                error_encoding(error.content),
                media,
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
