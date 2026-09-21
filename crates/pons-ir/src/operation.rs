use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::borrow::Cow;

use schemars::{JsonSchema, Schema, SchemaGenerator};

use crate::types::{Diagnostic, Doc, Name, Presence, TypeRef};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Operation {
    pub id: Vec<Name>,
    pub method: Method,
    pub path: Vec<PathSegment>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub query: Option<TypeRef>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub headers: Vec<HeaderParameter>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<Body>,
    #[serde(default)]
    pub responses: Vec<Response>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub errors: Vec<ErrorResponse>,
    #[serde(default = "inherit_errors")]
    pub errors_inherit: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stream: Option<StreamDescription>,
    pub auth: AuthRequirement,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provides: Option<CredentialProvision>,
    pub semantics: OperationSemantics,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub doc: Option<Doc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<Origin>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub ext: BTreeMap<String, Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<Diagnostic>,
}

fn inherit_errors() -> bool {
    true
}

impl Operation {
    pub(crate) fn visit_refs(&self, visit: &mut impl FnMut(&TypeRef)) {
        self.path.iter().for_each(|segment| {
            if let PathSegment::Param { ty, .. } = segment {
                ty.visit_refs(visit);
            }
        });
        if let Some(query) = &self.query {
            query.visit_refs(visit);
        }
        self.headers
            .iter()
            .for_each(|header| header.ty.visit_refs(visit));
        if let Some(body) = &self.body {
            body.ty.visit_refs(visit);
        }
        self.responses.iter().for_each(|response| {
            if let Some(body) = &response.body {
                body.visit_refs(visit);
            }
            response
                .headers
                .iter()
                .for_each(|header| header.ty.visit_refs(visit));
        });
        self.errors.iter().for_each(|error| {
            if let Some(body) = &error.body {
                body.visit_refs(visit);
            }
        });
        if let Some(stream) = &self.stream {
            stream.kind.visit_refs(visit);
        }
    }

    pub(crate) fn canonicalize(&mut self) {
        self.headers.sort_by(|left, right| {
            left.wire
                .to_ascii_lowercase()
                .cmp(&right.wire.to_ascii_lowercase())
                .then_with(|| left.wire.cmp(&right.wire))
        });
        self.responses.sort_by_key(|response| response.status);
        self.responses.iter_mut().for_each(|response| {
            response.headers.sort_by(|left, right| {
                left.wire
                    .to_ascii_lowercase()
                    .cmp(&right.wire.to_ascii_lowercase())
                    .then_with(|| left.wire.cmp(&right.wire))
            });
        });
        self.errors.sort_by(|left, right| {
            left.status
                .cmp(&right.status)
                .then_with(|| left.name.cmp(&right.name))
        });
        self.auth
            .alternatives
            .iter_mut()
            .for_each(|items| items.sort());
        self.auth.alternatives.sort();
        if let Some(stream) = &mut self.stream {
            stream.kind.canonicalize();
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "UPPERCASE")]
pub enum Method {
    Get,
    Post,
    Put,
    Patch,
    Delete,
    Head,
    Options,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(untagged)]
pub enum PathSegment {
    Literal { lit: String },
    Param { param: String, ty: TypeRef },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct HeaderParameter {
    pub wire: String,
    pub ident: Name,
    pub ty: TypeRef,
    pub presence: Presence,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub ext: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Body {
    pub ty: TypeRef,
    pub content: ContentKind,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub ext: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ContentKind {
    Json,
    Form,
    Multipart,
    Text,
    Bytes,
    Stream,
    ProblemJson,
    Empty,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Response {
    pub status: u16,
    pub content: ContentKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<TypeRef>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub headers: Vec<ResponseHeader>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub ext: BTreeMap<String, Value>,
}

impl Response {
    pub(crate) fn equivalent_ignoring_header_case(&self, other: &Self) -> bool {
        self.status == other.status
            && self.content == other.content
            && self.body == other.body
            && self.ext == other.ext
            && self.headers.len() == other.headers.len()
            && self
                .headers
                .iter()
                .zip(&other.headers)
                .all(|(left, right)| {
                    left.wire.eq_ignore_ascii_case(&right.wire)
                        && left.ty == right.ty
                        && left.presence == right.presence
                        && left.ext == right.ext
                })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ResponseHeader {
    pub wire: String,
    pub ty: TypeRef,
    pub presence: Presence,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub ext: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ErrorResponse {
    pub status: ErrorStatus,
    pub name: Name,
    pub content: ErrorContent,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<TypeRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retryable: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub doc: Option<Doc>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub ext: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum ErrorStatus {
    Exact(u16),
    Class(u8),
    Default,
}

impl JsonSchema for ErrorStatus {
    fn schema_name() -> Cow<'static, str> {
        "ErrorStatus".into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        schemars::json_schema!({
            "oneOf": [
                {"type": "integer", "minimum": 100, "maximum": 599},
                {"type": "string", "enum": ["1xx", "2xx", "3xx", "4xx", "5xx", "default"]}
            ]
        })
    }
}

impl Serialize for ErrorStatus {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match self {
            Self::Exact(status) => serializer.serialize_u16(*status),
            Self::Class(class) => serializer.serialize_str(&format!("{class}xx")),
            Self::Default => serializer.serialize_str("default"),
        }
    }
}

impl<'de> Deserialize<'de> for ErrorStatus {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct StatusVisitor;

        impl serde::de::Visitor<'_> for StatusVisitor {
            type Value = ErrorStatus;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("an HTTP status, status class, or default")
            }

            fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                u16::try_from(value)
                    .map(ErrorStatus::Exact)
                    .map_err(|_| E::custom("HTTP status is out of range"))
            }

            fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                match value {
                    "default" => Ok(ErrorStatus::Default),
                    "1xx" => Ok(ErrorStatus::Class(1)),
                    "2xx" => Ok(ErrorStatus::Class(2)),
                    "3xx" => Ok(ErrorStatus::Class(3)),
                    "4xx" => Ok(ErrorStatus::Class(4)),
                    "5xx" => Ok(ErrorStatus::Class(5)),
                    _ => Err(E::custom(format!("invalid HTTP status class {value}"))),
                }
            }
        }

        deserializer.deserialize_any(StatusVisitor)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ErrorContent {
    Json,
    ProblemJson,
    Text,
    Empty,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct AuthRequirement {
    #[serde(default)]
    pub alternatives: Vec<Vec<String>>,
    #[serde(default)]
    pub optional: bool,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub ext: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AuthScheme {
    Bearer {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        format: Option<String>,
    },
    Basic,
    ApiKey {
        #[serde(rename = "in")]
        location: ApiKeyLocation,
        name: String,
    },
    CookieSession {
        cookie: String,
    },
    OAuth2 {
        flows: Vec<OAuthFlow>,
    },
    Custom {
        description: String,
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        ext: BTreeMap<String, Value>,
    },
}

impl AuthScheme {
    pub(crate) fn canonicalize(&mut self) {
        if let Self::OAuth2 { flows } = self {
            flows.sort_by_key(|flow| flow.kind);
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ApiKeyLocation {
    Header,
    Query,
    Cookie,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct OAuthFlow {
    pub kind: OAuthFlowKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub authorization_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_url: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub scopes: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub ext: BTreeMap<String, Value>,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum OAuthFlowKind {
    AuthorizationCode,
    ClientCredentials,
    Password,
    Implicit,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct CredentialProvision {
    pub scheme: String,
    pub value: CredentialLocator,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<CredentialLocator>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub ext: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "from", rename_all = "snake_case")]
pub enum CredentialLocator {
    Body { pointer: String },
    Header { name: String },
    SetCookie { name: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct StreamDescription {
    #[serde(flatten)]
    pub kind: StreamKind,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum StreamKind {
    Sse { events: Vec<StreamMessage> },
    Ndjson { item: TypeRef },
    Bytes { content_type: String },
    Websocket { client: TypeRef, server: TypeRef },
}

impl StreamKind {
    pub(crate) fn visit_refs(&self, visit: &mut impl FnMut(&TypeRef)) {
        match self {
            Self::Sse { events } => events.iter().for_each(|event| event.data.visit_refs(visit)),
            Self::Ndjson { item } => item.visit_refs(visit),
            Self::Bytes { .. } => {}
            Self::Websocket { client, server } => {
                client.visit_refs(visit);
                server.visit_refs(visit);
            }
        }
    }

    pub(crate) fn canonicalize(&mut self) {
        if let Self::Sse { events } = self {
            events.sort_by(|left, right| left.event.cmp(&right.event));
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct StreamMessage {
    pub event: String,
    pub data: TypeRef,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct OperationSemantics {
    pub safe: bool,
    pub idempotent: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pagination: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Origin {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub column: Option<u32>,
}
