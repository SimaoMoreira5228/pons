use pons_ir::{ApiKeyLocation, AuthScheme, OAuthFlow, OAuthFlowKind, Operation};
use serde_json::{Map, Value, json};

use crate::{Builder, EmitError};

impl Builder<'_> {
    pub(super) fn security_schemes(&self) -> Result<Map<String, Value>, EmitError> {
        let mut schemes = Map::new();
        for (id, scheme) in &self.document.auth_schemes {
            let value = match scheme {
                AuthScheme::Bearer { format } => {
                    let mut value = json!({"type":"http","scheme":"bearer"});
                    if let Some(format) = format {
                        value["bearerFormat"] = json!(format);
                    }
                    value
                }
                AuthScheme::Basic => json!({"type":"http","scheme":"basic"}),
                AuthScheme::ApiKey { location, name } => json!({
                    "type":"apiKey",
                    "in":api_key_location(*location),
                    "name":name
                }),
                AuthScheme::CookieSession { cookie } => {
                    json!({"type":"apiKey","in":"cookie","name":cookie})
                }
                AuthScheme::OAuth2 { flows } => self.oauth_flows(id, flows)?,
                AuthScheme::Custom { .. } if self.is_required(id) => {
                    return Err(EmitError(format!(
                        "custom auth scheme {id} cannot be represented in OpenAPI"
                    )));
                }
                AuthScheme::Custom { .. } => continue,
            };
            schemes.insert(self.security_names[id].clone(), value);
        }
        Ok(schemes)
    }

    pub(super) fn operation_security(
        &self,
        operation: &Operation,
    ) -> Result<Vec<Value>, EmitError> {
        let mut requirements = operation
            .auth
            .alternatives
            .iter()
            .map(|group| {
                let mut requirement = Map::new();
                for scheme in group {
                    if !self.document.auth_schemes.contains_key(scheme) {
                        return Err(EmitError(format!("unknown auth scheme {scheme}")));
                    }

                    let scopes = match self.document.auth_schemes[scheme] {
                        AuthScheme::OAuth2 { .. } => operation
                            .auth
                            .scopes
                            .get(scheme)
                            .cloned()
                            .unwrap_or_default(),
                        _ => Vec::new(),
                    };
                    requirement.insert(self.security_names[scheme].clone(), json!(scopes));
                }
                Ok(Value::Object(requirement))
            })
            .collect::<Result<Vec<_>, EmitError>>()?;
        if operation.auth.optional
            && !requirements
                .iter()
                .any(|value| value.as_object().is_some_and(Map::is_empty))
        {
            requirements.push(json!({}));
        }
        Ok(requirements)
    }

    fn oauth_flows(&self, id: &str, flows: &[OAuthFlow]) -> Result<Value, EmitError> {
        if flows.is_empty() {
            return Err(EmitError(format!("OAuth2 scheme {id} has no flows")));
        }
        let mut result = Map::new();
        for flow in flows {
            let name = flow_name(flow.kind);
            if result.contains_key(name) {
                return Err(EmitError(format!(
                    "OAuth2 scheme {id} repeats the {name} flow"
                )));
            }
            let mut value = Map::new();
            match flow.kind {
                OAuthFlowKind::AuthorizationCode => {
                    let authorization_url = flow.authorization_url.as_deref().ok_or_else(|| {
                        EmitError(format!(
                            "OAuth2 authorizationCode flow in {id} needs an authorization URL"
                        ))
                    })?;
                    let token_url = flow.token_url.as_deref().ok_or_else(|| {
                        EmitError(format!(
                            "OAuth2 authorizationCode flow in {id} needs a token URL"
                        ))
                    })?;
                    validate_https_url(id, "authorization", authorization_url)?;
                    validate_https_url(id, "token", token_url)?;
                    value.insert("authorizationUrl".into(), json!(authorization_url));
                    value.insert("tokenUrl".into(), json!(token_url));
                }
                OAuthFlowKind::Implicit => {
                    if flow.token_url.is_some() {
                        return Err(EmitError(format!(
                            "OAuth2 implicit flow in {id} cannot have a token URL"
                        )));
                    }
                    let authorization_url = flow.authorization_url.as_deref().ok_or_else(|| {
                        EmitError(format!(
                            "OAuth2 implicit flow in {id} needs an authorization URL"
                        ))
                    })?;
                    validate_https_url(id, "authorization", authorization_url)?;
                    value.insert("authorizationUrl".into(), json!(authorization_url));
                }
                OAuthFlowKind::Password | OAuthFlowKind::ClientCredentials => {
                    if flow.authorization_url.is_some() {
                        return Err(EmitError(format!(
                            "OAuth2 {} flow in {id} cannot have an authorization URL",
                            name
                        )));
                    }
                    let token_url = flow.token_url.as_deref().ok_or_else(|| {
                        EmitError(format!("OAuth2 {name} flow in {id} needs a token URL"))
                    })?;
                    validate_https_url(id, "token", token_url)?;
                    value.insert("tokenUrl".into(), json!(token_url));
                }
            }
            value.insert("scopes".into(), json!(flow.scopes));
            result.insert(name.into(), Value::Object(value));
        }
        Ok(json!({"type":"oauth2","flows":result}))
    }

    fn is_required(&self, id: &str) -> bool {
        self.document.operations.iter().any(|operation| {
            operation
                .auth
                .alternatives
                .iter()
                .any(|group| group.iter().any(|scheme| scheme == id))
        })
    }
}

fn validate_https_url(id: &str, kind: &str, value: &str) -> Result<(), EmitError> {
    let valid = value.strip_prefix("https://").is_some_and(|rest| {
        let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
        let authority = &rest[..end];
        let host = authority.rsplit('@').next().unwrap_or_default();
        !host.is_empty() && !rest.chars().any(char::is_whitespace)
    });
    if valid {
        Ok(())
    } else {
        Err(EmitError(format!(
            "OAuth2 {kind} URL for {id} must be an absolute HTTPS URL"
        )))
    }
}

fn api_key_location(location: ApiKeyLocation) -> &'static str {
    match location {
        ApiKeyLocation::Header => "header",
        ApiKeyLocation::Query => "query",
        ApiKeyLocation::Cookie => "cookie",
    }
}

fn flow_name(kind: OAuthFlowKind) -> &'static str {
    match kind {
        OAuthFlowKind::AuthorizationCode => "authorizationCode",
        OAuthFlowKind::ClientCredentials => "clientCredentials",
        OAuthFlowKind::Password => "password",
        OAuthFlowKind::Implicit => "implicit",
    }
}

#[cfg(test)]
mod tests {
    use pons_ir::{AuthScheme, OAuthFlow, OAuthFlowKind};
    use serde_json::{Value, json};

    fn fixture() -> pons_ir::Document {
        serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap()
    }

    #[test]
    fn emits_scopes_only_for_oauth2_schemes() {
        let mut document = fixture();
        document.auth_schemes.insert(
            "oauth".into(),
            AuthScheme::OAuth2 {
                flows: vec![OAuthFlow {
                    kind: OAuthFlowKind::ClientCredentials,
                    authorization_url: None,
                    token_url: Some("https://auth.example.test/token".into()),
                    scopes: Default::default(),
                    ext: Default::default(),
                }],
            },
        );
        let events = &mut document.operations[1];
        events.auth.alternatives = vec![
            vec!["session".into(), "tenant".into()],
            vec!["oauth".into()],
        ];
        for scheme in ["session", "tenant", "oauth"] {
            events
                .auth
                .scopes
                .insert(scheme.into(), vec!["events:read".into()]);
        }
        let output: Value = serde_json::from_str(&crate::emit(&document).unwrap()).unwrap();

        assert_eq!(
            output["components"]["securitySchemes"]["Session"]["type"],
            "http"
        );
        assert_eq!(
            output["components"]["securitySchemes"]["Oauth"]["type"],
            "oauth2"
        );
        let security = output["paths"]["/events/{cursor}"]["get"]["security"]
            .as_array()
            .unwrap();
        let scopes = |scheme: &str| {
            security
                .iter()
                .find(|requirement| requirement.get(scheme).is_some())
                .unwrap_or_else(|| panic!("{scheme} is not a required scheme"))[scheme]
                .clone()
        };
        assert_eq!(scopes("Session"), json!([]));
        assert_eq!(scopes("Tenant"), json!([]));
        assert_eq!(scopes("Oauth"), json!(["events:read"]));
    }
}
