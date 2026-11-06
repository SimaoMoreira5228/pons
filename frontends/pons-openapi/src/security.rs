use std::collections::BTreeMap;

use pons_ir::{ApiKeyLocation, AuthRequirement, AuthScheme, Diagnostic, OAuthFlow, OAuthFlowKind};
use serde_json::{Map, Value};

use crate::document::push_dropped;
use crate::error::ImportError;
use crate::refs::{Resolver, Site};

pub(crate) struct Schemes {
    pub(crate) definitions: BTreeMap<String, AuthScheme>,
    names: BTreeMap<String, String>,
}

impl Schemes {
    pub(crate) fn resolve(&self, scheme: &str) -> Option<String> {
        if self.definitions.contains_key(scheme) {
            return Some(scheme.to_owned());
        }
        if let Some(mapped) = self.names.get(scheme)
            && self.definitions.contains_key(mapped)
        {
            return Some(mapped.clone());
        }
        let mut matching = self
            .definitions
            .keys()
            .filter(|key| key.eq_ignore_ascii_case(scheme));
        let first = matching.next()?;
        if matching.next().is_some() {
            return None;
        }
        Some(first.clone())
    }
}

pub(crate) fn schemes<'v>(
    declared: Option<&'v Map<String, Value>>,
    stored: Option<&Value>,
    resolver: &'v Resolver<'_>,
    diagnostics: &mut Vec<Diagnostic>,
) -> Result<Schemes, ImportError> {
    let derived = derived_schemes(declared, resolver, diagnostics)?;
    let Some(stored) = stored_schemes(stored, diagnostics) else {
        return Ok(Schemes {
            definitions: derived,
            names: BTreeMap::new(),
        });
    };
    let names = scheme_names(&derived, &stored);
    Ok(Schemes {
        definitions: stored,
        names,
    })
}

fn stored_schemes(
    stored: Option<&Value>,
    diagnostics: &mut Vec<Diagnostic>,
) -> Option<BTreeMap<String, AuthScheme>> {
    let stored = stored?;
    if stored.is_null() {
        return None;
    }
    match serde_json::from_value(stored.clone()) {
        Ok(schemes) => Some(schemes),
        Err(error) => {
            push_dropped(
                diagnostics,
                None,
                format!(
                    "the x-pons auth_schemes member is malformed ({error}), so the security schemes come from components.securitySchemes"
                ),
            );
            None
        }
    }
}

fn scheme_names(
    derived: &BTreeMap<String, AuthScheme>,
    stored: &BTreeMap<String, AuthScheme>,
) -> BTreeMap<String, String> {
    let mut names = BTreeMap::new();
    for (declared, definition) in derived {
        let candidates = stored
            .iter()
            .filter(|(key, scheme)| key.eq_ignore_ascii_case(declared) || *scheme == definition)
            .map(|(key, _)| key.clone())
            .collect::<Vec<_>>();
        if candidates.len() == 1 {
            names.insert(declared.clone(), candidates[0].clone());
        }
    }
    names
}

fn derived_schemes<'v>(
    declared: Option<&'v Map<String, Value>>,
    resolver: &'v Resolver<'_>,
    diagnostics: &mut Vec<Diagnostic>,
) -> Result<BTreeMap<String, AuthScheme>, ImportError> {
    let mut schemes = BTreeMap::new();
    let Some(declared) = declared else {
        return Ok(schemes);
    };
    for (name, value) in declared {
        let Some(value) = resolver.resolve(value, Site::Document, diagnostics) else {
            continue;
        };
        let Some(object) = value.as_object() else {
            push_dropped(
                diagnostics,
                None,
                format!("the security scheme {name:?} is not an object, so it is dropped"),
            );
            continue;
        };
        let scheme = match object.get("type").and_then(Value::as_str) {
            Some("http") => match object.get("scheme").and_then(Value::as_str) {
                Some(scheme) if scheme.eq_ignore_ascii_case("bearer") => AuthScheme::Bearer {
                    format: object
                        .get("bearerFormat")
                        .and_then(Value::as_str)
                        .map(str::to_owned),
                },
                Some(scheme) if scheme.eq_ignore_ascii_case("basic") => AuthScheme::Basic,
                Some(scheme) => {
                    push_dropped(
                        diagnostics,
                        None,
                        format!(
                            "the security scheme {name:?} uses HTTP {scheme} authentication, which has no Pons adapter, so it becomes a custom scheme"
                        ),
                    );
                    custom(object, format!("HTTP {scheme} authentication"))
                }
                None => {
                    push_dropped(
                        diagnostics,
                        None,
                        format!(
                            "the security scheme {name:?} is HTTP with no scheme, so it becomes a custom scheme"
                        ),
                    );
                    custom(object, "HTTP authentication".into())
                }
            },
            Some("apiKey") => match (
                object.get("in").and_then(Value::as_str),
                object.get("name").and_then(Value::as_str),
            ) {
                (Some("header"), Some(carrier)) => AuthScheme::ApiKey {
                    location: ApiKeyLocation::Header,
                    name: carrier.to_owned(),
                },
                (Some("query"), Some(carrier)) => AuthScheme::ApiKey {
                    location: ApiKeyLocation::Query,
                    name: carrier.to_owned(),
                },
                (Some("cookie"), Some(carrier)) => AuthScheme::ApiKey {
                    location: ApiKeyLocation::Cookie,
                    name: carrier.to_owned(),
                },
                _ => {
                    push_dropped(
                        diagnostics,
                        None,
                        format!(
                            "the apiKey security scheme {name:?} has no header, query, or cookie carrier, so it becomes a custom scheme"
                        ),
                    );
                    custom(object, "API key authentication".into())
                }
            },
            Some("oauth2") => match oauth_flows(object.get("flows"), diagnostics, name) {
                Some(flows) => AuthScheme::OAuth2 { flows },
                None => custom(object, "OAuth 2.0 authentication".into()),
            },
            Some("openIdConnect") => {
                push_dropped(
                    diagnostics,
                    None,
                    format!(
                        "the security scheme {name:?} is openIdConnect, which Pons cannot express, so it becomes a custom scheme"
                    ),
                );
                custom(object, "OpenID Connect authentication".into())
            }
            Some(kind) => {
                push_dropped(
                    diagnostics,
                    None,
                    format!(
                        "the security scheme {name:?} has type {kind:?}, which Pons cannot express, so it becomes a custom scheme"
                    ),
                );
                custom(object, format!("{kind} authentication"))
            }
            None => {
                push_dropped(
                    diagnostics,
                    None,
                    format!(
                        "the security scheme {name:?} declares no type, so it becomes a custom scheme"
                    ),
                );
                custom(object, "custom authentication".into())
            }
        };
        schemes.insert(name.clone(), scheme);
    }
    Ok(schemes)
}

fn custom(object: &Map<String, Value>, description: String) -> AuthScheme {
    AuthScheme::Custom {
        description: object
            .get("description")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .unwrap_or(description),
        adapter: None,
        ext: Default::default(),
    }
}

fn oauth_flows(
    flows: Option<&Value>,
    diagnostics: &mut Vec<Diagnostic>,
    scheme: &str,
) -> Option<Vec<OAuthFlow>> {
    let Some(flows) = flows.and_then(Value::as_object) else {
        push_dropped(
            diagnostics,
            None,
            format!("the oauth2 security scheme {scheme:?} declares no flows"),
        );
        return None;
    };
    let mut result = Vec::new();
    for (name, flow) in flows {
        let Some(flow) = flow.as_object() else {
            push_dropped(
                diagnostics,
                None,
                format!("the {name} flow of the oauth2 scheme {scheme:?} is not an object"),
            );
            continue;
        };
        let kind = match name.as_str() {
            "authorizationCode" => OAuthFlowKind::AuthorizationCode,
            "implicit" => OAuthFlowKind::Implicit,
            "password" => OAuthFlowKind::Password,
            "clientCredentials" => OAuthFlowKind::ClientCredentials,
            other => {
                push_dropped(
                    diagnostics,
                    None,
                    format!(
                        "the oauth2 scheme {scheme:?} declares the unknown {other:?} flow, which is dropped"
                    ),
                );
                continue;
            }
        };
        result.push(OAuthFlow {
            kind,
            authorization_url: flow
                .get("authorizationUrl")
                .and_then(Value::as_str)
                .map(str::to_owned),
            token_url: flow
                .get("tokenUrl")
                .and_then(Value::as_str)
                .map(str::to_owned),
            scopes: scopes_of(flow.get("scopes")),
            ext: Default::default(),
        });
    }
    if result.is_empty() {
        push_dropped(
            diagnostics,
            None,
            format!("the oauth2 security scheme {scheme:?} declares no usable flow"),
        );
        None
    } else {
        Some(result)
    }
}

fn scopes_of(value: Option<&Value>) -> BTreeMap<String, String> {
    value
        .and_then(Value::as_object)
        .map(|scopes| {
            scopes
                .iter()
                .map(|(name, description)| {
                    (
                        name.clone(),
                        description.as_str().unwrap_or_default().to_owned(),
                    )
                })
                .collect()
        })
        .unwrap_or_default()
}

pub(crate) fn requirement(
    declared: Option<&Value>,
    schemes: &Schemes,
    operation: &str,
    diagnostics: &mut Vec<Diagnostic>,
) -> Result<AuthRequirement, ImportError> {
    let Some(declared) = declared else {
        return Ok(AuthRequirement::default());
    };
    let Some(requirements) = declared.as_array() else {
        return Err(ImportError::Document(format!(
            "operation {operation}: the security value is not an array"
        )));
    };
    let mut requirement = AuthRequirement::default();
    for entry in requirements {
        let Some(entry) = entry.as_object() else {
            return Err(ImportError::Document(format!(
                "operation {operation}: a security requirement is not an object"
            )));
        };
        if entry.is_empty() {
            requirement.optional = true;
            continue;
        }
        let mut alternative = Vec::new();
        for (scheme, scopes) in entry {
            let Some(resolved) = schemes.resolve(scheme) else {
                return Err(ImportError::Document(format!(
                    "operation {operation}: the security requirement names the unknown scheme {scheme}"
                )));
            };
            alternative.push(resolved.clone());
            let Some(scopes) = scopes.as_array() else {
                return Err(ImportError::Document(format!(
                    "operation {operation}: the security requirement for {scheme} is not an array of scopes"
                )));
            };
            let mut kept = Vec::new();
            for scope in scopes {
                let Some(scope) = scope.as_str() else {
                    return Err(ImportError::Document(format!(
                        "operation {operation}: the security requirement for {scheme} has a non-string scope"
                    )));
                };
                if !declared_scopes(schemes.definitions.get(&resolved), scope) {
                    push_dropped(
                        diagnostics,
                        Some(operation),
                        format!(
                            "the {scheme:?} security requirement names the scope {scope:?}, which the scheme does not declare, so the scope is dropped"
                        ),
                    );
                    continue;
                }
                kept.push(scope.to_owned());
            }
            if !kept.is_empty() {
                requirement.scopes.entry(resolved).or_default().extend(kept);
            }
        }
        requirement.alternatives.push(alternative);
    }
    Ok(requirement)
}

fn declared_scopes(scheme: Option<&AuthScheme>, scope: &str) -> bool {
    match scheme {
        Some(AuthScheme::OAuth2 { flows }) => {
            flows.iter().any(|flow| flow.scopes.contains_key(scope))
        }
        _ => false,
    }
}
