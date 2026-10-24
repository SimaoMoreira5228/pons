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
                    let Some(definition) = self.document.auth_schemes.get(scheme) else {
                        return Err(EmitError(format!("unknown auth scheme {scheme}")));
                    };
                    if matches!(definition, AuthScheme::Custom { .. }) {
                        if group.len() > 1 {
                            return Err(EmitError(format!(
                                "the auth alternative {} cannot be represented in OpenAPI, because one requirement states every scheme in it at once and no Security Scheme Object describes the custom auth scheme {scheme}",
                                group.join(" or ")
                            )));
                        }
                        if !operation.auth.optional {
                            return Err(EmitError(format!(
                                "custom auth scheme {scheme} cannot be represented in OpenAPI"
                            )));
                        }
                        continue;
                    }
                    let scopes = match definition {
                        AuthScheme::OAuth2 { .. } => {
                            operation.auth.scopes.get(scheme).cloned().unwrap_or_default()
                        }
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
                    value.insert("tokenUrl".into(), json!(token_url));
                }
            }
            value.insert("scopes".into(), json!(flow.scopes));
            result.insert(name.into(), Value::Object(value));
        }
        Ok(json!({"type":"oauth2","flows":result}))
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
    use std::collections::BTreeMap;

    use pons_ir::{AuthScheme, OAuthFlow, OAuthFlowKind};
    use serde_json::{Value, json};

    fn fixture() -> pons_ir::Document {
        serde_json::from_str(include_str!("../../../spec/fixtures/login.json")).unwrap()
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
                    scopes: BTreeMap::from([("events:read".into(), "Read events".into())]),
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

    #[test]
    fn an_optional_custom_scheme_is_the_anonymous_alternative_and_a_required_one_stops() {
        let mut document = fixture();
        document.auth_schemes.insert(
            "custom".into(),
            AuthScheme::Custom {
                description: "custom auth".into(),
                adapter: None,
                ext: Default::default(),
            },
        );
        let events = &mut document.operations[1];
        events.auth.alternatives = vec![vec!["custom".into()], vec!["tenant".into()]];
        events.auth.optional = true;
        let output: Value = serde_json::from_str(&crate::emit(&document).unwrap()).unwrap();

        assert_eq!(
            output["paths"]["/events/{cursor}"]["get"]["security"],
            json!([{}, {"Tenant": []}]),
            "the optional custom scheme is the anonymous alternative the client may take"
        );
        assert!(
            output["components"]["securitySchemes"]
                .get("Custom")
                .is_none(),
            "no Security Scheme Object describes a custom scheme"
        );

        document.operations[1].auth.optional = false;
        assert_eq!(
            crate::emit(&document).unwrap_err().0,
            "custom auth scheme custom cannot be represented in OpenAPI"
        );
    }

    #[test]
    fn refuses_an_alternative_that_mixes_a_custom_scheme_with_another() {
        let mut document = fixture();
        document.auth_schemes.insert(
            "custom".into(),
            AuthScheme::Custom {
                description: "custom auth".into(),
                adapter: None,
                ext: Default::default(),
            },
        );
        let events = &mut document.operations[1];
        events.auth.alternatives = vec![vec!["custom".into(), "tenant".into()]];
        events.auth.optional = true;
        let error = crate::emit(&document).unwrap_err().0;
        assert!(error.contains("custom or tenant"), "{error}");
        assert!(
            error.contains("cannot be represented in OpenAPI"),
            "{error}"
        );

        document.operations[1].auth.alternatives = vec![vec!["custom".into()]];
        let output: Value = serde_json::from_str(&crate::emit(&document).unwrap()).unwrap();
        assert_eq!(
            output["paths"]["/events/{cursor}"]["get"]["security"],
            json!([{}]),
            "one custom scheme in its own alternative is the anonymous alternative alone"
        );
    }

    #[test]
    fn rejects_an_oauth2_scope_that_no_flow_declares() {
        let mut document = fixture();
        document.auth_schemes.insert(
            "oauth".into(),
            AuthScheme::OAuth2 {
                flows: vec![OAuthFlow {
                    kind: OAuthFlowKind::ClientCredentials,
                    authorization_url: None,
                    token_url: Some("https://auth.example.test/token".into()),
                    scopes: BTreeMap::from([("events:read".into(), "Read events".into())]),
                    ext: Default::default(),
                }],
            },
        );
        let events = &mut document.operations[1];
        events.auth.alternatives = vec![vec!["oauth".into()]];
        events
            .auth
            .scopes
            .insert("oauth".into(), vec!["events:write".into()]);
        assert_eq!(
            crate::emit(&document).unwrap_err().0,
            "operation [[\"events\"], [\"watch\"]]: the OAuth2 scheme oauth does not declare the events:write scope"
        );
    }
}
