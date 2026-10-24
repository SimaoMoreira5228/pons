use std::collections::BTreeMap;

use pons_ir::{ApiKeyLocation, AuthScheme, CredentialLocator, Document, Operation};
use serde_json::Value;

use crate::live::Reply;

pub(crate) struct Auth {
    bearer: Option<String>,
    api_keys: Vec<(String, String)>,
    headers: Vec<(String, String)>,
    cookies: Vec<(String, String)>,
    provided: BTreeMap<String, String>,
}

impl Auth {
    pub(crate) fn new(
        bearer: Option<String>,
        api_keys: Vec<(String, String)>,
        headers: Vec<(String, String)>,
        cookies: Vec<(String, String)>,
    ) -> Self {
        Self {
            bearer,
            api_keys,
            headers,
            cookies,
            provided: BTreeMap::new(),
        }
    }

    pub(crate) fn attach(
        &self,
        document: &Document,
        operation: &Operation,
    ) -> Result<Attachment, String> {
        let mut attachment = Attachment::default();
        if operation.auth.alternatives.is_empty() {
            self.raw(&mut attachment);
            return Ok(attachment);
        }
        for alternative in &operation.auth.alternatives {
            let mut placements = Vec::new();
            let mut satisfied = true;
            for scheme in alternative {
                match self.scheme(scheme, document) {
                    Some(found) => placements.extend(found),
                    None => {
                        satisfied = false;
                        break;
                    }
                }
            }
            if satisfied {
                for placement in placements {
                    match placement {
                        Placement::Header(name, value) => {
                            set_header(&mut attachment.headers, name, value);
                        }
                        Placement::Query(name, value) => attachment.query.push((name, value)),
                        Placement::Cookie(name, value) => attachment.cookies.push((name, value)),
                    }
                }
                self.finish(&mut attachment);
                return Ok(attachment);
            }
        }
        if operation.auth.optional {
            self.raw(&mut attachment);
            return Ok(attachment);
        }
        Err(unsatisfied(document, operation))
    }

    pub(crate) fn capture(
        &mut self,
        operation: &Operation,
        reply: &Reply,
    ) -> Result<(), Vec<String>> {
        let Some(provides) = &operation.provides else {
            return Ok(());
        };
        match locate(&provides.value, reply) {
            Some(value) => {
                self.provided.insert(provides.scheme.clone(), value);
                Ok(())
            }
            None => Err(vec![format!(
                "the response does not provide {} at {}",
                provides.scheme,
                describe(&provides.value)
            )]),
        }
    }

    fn scheme(&self, scheme: &str, document: &Document) -> Option<Vec<Placement>> {
        let definition = document.auth_schemes.get(scheme)?;
        if let Some(value) = self.provided.get(scheme) {
            return carrier(definition, value);
        }
        match definition {
            AuthScheme::Bearer { .. } => self
                .bearer
                .as_ref()
                .map(|token| vec![bearer(token.clone())]),
            AuthScheme::ApiKey { location, name } => self
                .api_key(scheme, name)
                .map(|value| api_key(*location, name, value)),
            AuthScheme::CookieSession { cookie } => self
                .cookie(cookie)
                .map(|value| vec![Placement::Cookie(cookie.clone(), value)]),
            AuthScheme::Basic | AuthScheme::OAuth2 { .. } | AuthScheme::Custom { .. } => None,
        }
    }

    fn api_key(&self, scheme: &str, wire: &str) -> Option<String> {
        self.api_keys
            .iter()
            .find(|(name, _)| name == scheme || name == wire)
            .map(|(_, value)| value.clone())
    }

    fn cookie(&self, name: &str) -> Option<String> {
        self.cookies
            .iter()
            .find(|(cookie, _)| cookie == name)
            .map(|(_, value)| value.clone())
    }

    fn raw(&self, attachment: &mut Attachment) {
        attachment.cookies.extend(self.cookies.iter().cloned());
        self.finish(attachment);
    }

    fn finish(&self, attachment: &mut Attachment) {
        if !attachment.cookies.is_empty() {
            let value = attachment
                .cookies
                .iter()
                .map(|(name, value)| format!("{name}={value}"))
                .collect::<Vec<_>>()
                .join("; ");
            set_header(&mut attachment.headers, "cookie".into(), value);
        }
        for (name, value) in &self.headers {
            set_header(&mut attachment.headers, name.clone(), value.clone());
        }
    }
}

#[derive(Default)]
pub(crate) struct Attachment {
    pub(crate) headers: Vec<(String, String)>,
    pub(crate) query: Vec<(String, String)>,
    cookies: Vec<(String, String)>,
}

enum Placement {
    Header(String, String),
    Query(String, String),
    Cookie(String, String),
}

fn carrier(scheme: &AuthScheme, value: &str) -> Option<Vec<Placement>> {
    match scheme {
        AuthScheme::Bearer { .. } => Some(vec![bearer(value.to_owned())]),
        AuthScheme::ApiKey { location, name } => Some(api_key(*location, name, value.to_owned())),
        AuthScheme::CookieSession { cookie } => {
            Some(vec![Placement::Cookie(cookie.clone(), value.to_owned())])
        }
        AuthScheme::Basic | AuthScheme::OAuth2 { .. } | AuthScheme::Custom { .. } => None,
    }
}

fn bearer(token: String) -> Placement {
    Placement::Header("authorization".into(), format!("Bearer {token}"))
}

fn api_key(location: ApiKeyLocation, name: &str, value: String) -> Vec<Placement> {
    vec![match location {
        ApiKeyLocation::Header => Placement::Header(name.to_owned(), value),
        ApiKeyLocation::Query => Placement::Query(name.to_owned(), value),
        ApiKeyLocation::Cookie => Placement::Cookie(name.to_owned(), value),
    }]
}

fn set_header(headers: &mut Vec<(String, String)>, name: String, value: String) {
    match headers
        .iter_mut()
        .find(|(existing, _)| existing.eq_ignore_ascii_case(&name))
    {
        Some((_, existing)) => *existing = value,
        None => headers.push((name, value)),
    }
}

fn unsatisfied(document: &Document, operation: &Operation) -> String {
    let schemes = operation
        .auth
        .alternatives
        .iter()
        .flatten()
        .map(String::as_str)
        .collect::<Vec<_>>();
    let named = schemes.join(", ");
    let custom = schemes.iter().any(|scheme| {
        matches!(
            document.auth_schemes.get(*scheme),
            Some(AuthScheme::Custom { .. })
        )
    });
    if custom {
        format!("custom authentication is required ({named})")
    } else {
        format!(
            "authentication is required ({named}); supply the matching --bearer, --api-key, or --cookie"
        )
    }
}

fn locate(locator: &CredentialLocator, reply: &Reply) -> Option<String> {
    match locator {
        CredentialLocator::Body { pointer } => {
            let value = serde_json::from_str::<serde_json::Value>(&reply.body).ok()?;
            value.pointer(pointer).and_then(scalar)
        }
        CredentialLocator::Header { name } => reply
            .headers
            .get(&name.to_ascii_lowercase())
            .and_then(|values| values.first())
            .cloned(),
        CredentialLocator::SetCookie { name } => reply
            .headers
            .get("set-cookie")
            .into_iter()
            .flatten()
            .find_map(|value| cookie_value(value, name)),
    }
}

fn cookie_value(header: &str, name: &str) -> Option<String> {
    let pair = header.split(';').next()?.trim();
    let (cookie, value) = pair.split_once('=')?;
    (cookie.trim() == name).then(|| value.trim().to_owned())
}

fn scalar(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => Some(text.clone()),
        Value::Bool(flag) => Some(flag.to_string()),
        Value::Number(number) => Some(number.to_string()),
        _ => None,
    }
}

fn describe(locator: &CredentialLocator) -> String {
    match locator {
        CredentialLocator::Body { pointer } => format!("body:{pointer}"),
        CredentialLocator::Header { name } => format!("header:{name}"),
        CredentialLocator::SetCookie { name } => format!("set_cookie:{name}"),
    }
}
