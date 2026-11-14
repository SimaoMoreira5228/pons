use std::collections::BTreeSet;

use crate::operation::{ApiKeyLocation, AuthScheme, CredentialLocator, OAuthFlowKind, Operation};

use super::{Document, ValidationError};

fn absolute_https_url(value: &str) -> bool {
    value.strip_prefix("https://").is_some_and(|rest| {
        let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
        let authority = &rest[..end];
        let host = authority.rsplit('@').next().unwrap_or_default();
        !host.is_empty() && !rest.chars().any(char::is_whitespace)
    })
}

impl Document {
    pub(super) fn validate_auth_schemes(&self, errors: &mut Vec<ValidationError>) {
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
                    errors,
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
    }

    pub(super) fn validate_operation_auth(
        &self,
        owner: &str,
        operation: &Operation,
        errors: &mut Vec<ValidationError>,
    ) {
        for alternative in &operation.auth.alternatives {
            if alternative.is_empty() {
                errors.push(ValidationError::InvalidValue {
                    path: owner.to_owned(),
                    message: "auth alternative cannot be empty".into(),
                });
                continue;
            }
            for scheme in alternative {
                if !self.auth_schemes.contains_key(scheme) {
                    errors.push(ValidationError::UnknownAuthScheme {
                        owner: owner.to_owned(),
                        scheme: scheme.clone(),
                    });
                }
            }
        }
        for (scheme, scopes) in &operation.auth.scopes {
            if !self.auth_schemes.contains_key(scheme) {
                errors.push(ValidationError::UnknownAuthScheme {
                    owner: owner.to_owned(),
                    scheme: scheme.clone(),
                });
            }
            if let Some(AuthScheme::OAuth2 { flows }) = self.auth_schemes.get(scheme) {
                for scope in scopes {
                    if !flows.iter().any(|flow| flow.scopes.contains_key(scope)) {
                        errors.push(ValidationError::InvalidValue {
                            path: owner.to_owned(),
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
                    path: owner.to_owned(),
                    message: format!("auth scheme {scheme} has scopes but no auth requirement"),
                });
            }
            if scopes.is_empty() {
                errors.push(ValidationError::InvalidValue {
                    path: owner.to_owned(),
                    message: format!("auth scheme {scheme} has an empty scope list"),
                });
            }
            let mut unique = BTreeSet::new();
            for scope in scopes {
                if scope.is_empty() || scope.chars().any(char::is_whitespace) {
                    errors.push(ValidationError::InvalidValue {
                        path: owner.to_owned(),
                        message: format!("auth scheme {scheme} has an invalid scope"),
                    });
                }
                if !unique.insert(scope) {
                    errors.push(ValidationError::InvalidValue {
                        path: owner.to_owned(),
                        message: format!("auth scheme {scheme} repeats scope {scope}"),
                    });
                }
            }
        }
        if let Some(provides) = &operation.provides
            && !self.auth_schemes.contains_key(&provides.scheme)
        {
            errors.push(ValidationError::UnknownAuthScheme {
                owner: owner.to_owned(),
                scheme: provides.scheme.clone(),
            });
        }
        if let Some(provides) = &operation.provides {
            for locator in std::iter::once(&provides.value).chain(provides.expires_at.iter()) {
                if let CredentialLocator::Header { name } = locator {
                    self.validate_header_name(
                        owner,
                        "credential header",
                        name,
                        &mut BTreeSet::new(),
                        errors,
                    );
                }
                if matches!(locator, CredentialLocator::SetCookie { name } if name.is_empty()) {
                    errors.push(ValidationError::InvalidValue {
                        path: owner.to_owned(),
                        message: "credential cookie name cannot be empty".into(),
                    });
                }
                if let Err(message) = locator.validate() {
                    errors.push(ValidationError::InvalidValue {
                        path: owner.to_owned(),
                        message,
                    });
                }
            }
        }
    }
}
