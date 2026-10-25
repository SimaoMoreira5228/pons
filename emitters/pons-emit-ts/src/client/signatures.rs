use std::collections::BTreeMap;

use pons_ir::{
    ApiKeyLocation, AuthScheme, ContentKind, Doc, Document, ErrorResponse, ErrorStatus, Operation,
    Presence, Response, ResponseHeader, ResponseStatus, StreamKind,
};

use crate::naming::{pascal, unique_names};
use crate::render::{jsdoc, string, union_type};
use crate::{EmitError, TsOptions, types};

#[derive(Default)]
pub(super) struct Node {
    pub(super) children: BTreeMap<String, Node>,
    pub(super) operations: Vec<(Option<Doc>, String, String)>,
}

impl Node {
    pub(super) fn render(&self, depth: usize) -> String {
        let indent = " ".repeat(depth);
        let mut members = self
            .operations
            .iter()
            .map(|(doc, name, value)| {
                let key = if name == "__proto__" {
                    format!("[{}]", string(name))
                } else {
                    name.clone()
                };
                let comment = jsdoc(doc.as_ref());
                if comment.is_empty() {
                    format!("{indent}{key}: {value}")
                } else {
                    format!("{indent}{comment}\n{indent}{key}: {value}")
                }
            })
            .collect::<Vec<_>>();
        members.extend(self.children.iter().map(|(name, child)| {
            let key = if name == "__proto__" {
                format!("[{}]", string(name))
            } else {
                name.clone()
            };
            format!("{indent}{key}: {}", child.render(depth + 2))
        }));
        format!(
            "{{\n{}\n{}}}",
            members.join(",\n"),
            " ".repeat(depth.saturating_sub(2))
        )
    }
}

pub(super) fn operation_type_names(document: &Document) -> Vec<Vec<String>> {
    let mut counts = Vec::with_capacity(document.operations.len());
    let mut candidates = Vec::new();
    for operation in &document.operations {
        let id = operation.id.iter().flatten().cloned().collect::<Vec<_>>();
        let name = pascal(&id);
        candidates.extend([
            format!("{name}Args"),
            format!("{name}Response"),
            format!("{name}Error"),
        ]);
        let mut extra = Vec::new();
        match operation.stream.as_ref().map(|stream| &stream.kind) {
            Some(StreamKind::Sse { .. }) => extra.push(format!("{name}Event")),
            Some(StreamKind::Ndjson { .. }) => extra.push(format!("{name}Item")),
            Some(StreamKind::Websocket { .. }) => {
                extra.push(format!("{name}ClientMessage"));
                extra.push(format!("{name}ServerMessage"));
            }
            _ => {}
        }
        if !extra.is_empty() {
            extra.push(format!("{name}StreamError"));
        }
        counts.push(3 + extra.len());
        candidates.extend(extra);
    }
    let mut names = unique_names(candidates, &[]).into_iter();
    counts
        .into_iter()
        .map(|count| {
            let allocated = names.by_ref().take(count).collect::<Vec<_>>();
            debug_assert_eq!(allocated.len(), count);
            allocated
        })
        .collect()
}

pub(super) fn option_types(opaque_required: bool) -> String {
    let mut output = String::new();
    output.push_str("export type AuthAttachment = { headers?: Record<string, string>; query?: Record<string, string>; cookies?: Record<string, string> };\nexport type AuthAdapter = (context: { scheme: string; scopes: readonly string[]; method: string; url: URL; headers: Headers }) => AuthAttachment | Promise<AuthAttachment>;\n/** Scope and lifetime captured with a credential. `expiresAt` is a Unix millisecond timestamp; `host`, `path`, `secure`, and `origin` bind the value to the response that issued it. */\nexport type CredentialMetadata = { expiresAt?: number | undefined; path?: string | undefined; secure?: boolean | undefined; host?: string | undefined; origin?: string | undefined };\nexport type StoredCredential = { value: string } & CredentialMetadata;\n/** A store returns a `{ value, ...metadata }` record, never a bare string, and must persist the metadata it is given in `set` so that expiry and origin scope survive a new client. A record with a non-string `value` or a metadata field of the wrong type is rejected. */\nexport type TokenStore = { get(): StoredCredential | undefined | Promise<StoredCredential | undefined>; set(value: string, metadata?: CredentialMetadata): void | Promise<void>; clear(): void | Promise<void> };\nexport type CredentialSource = string | (() => string | undefined | Promise<string | undefined>) | TokenStore;\nexport type TransportRequest = { url: URL; init: RequestInit };\nexport type TransportResponse = { response: Response; url: URL; init: RequestInit };\nexport type TransportRequestHook = (context: TransportRequest) => void | Promise<void>;\nexport type TransportResponseHook = (context: TransportResponse) => void | Promise<void>;\nexport type Transport = { onRequest?: TransportRequestHook; onResponse?: TransportResponseHook };\nexport type AuthSchemeDefinition = { kind: string; location?: string | undefined; name?: string | undefined; adapter?: string | undefined };\nexport type RetryOptions = { attempts?: number; backoffMs?: number; maxBackoffMs?: number; jitterMs?: number; force?: boolean };\nexport type CallOptions = { headers?: HeadersInit; signal?: AbortSignal; timeoutMs?: number; retry?: RetryOptions | false };\n");
    output.push_str(&format!(
        "export type ClientOptions = {{ baseUrl: string; headers?: HeadersInit | (() => HeadersInit | Promise<HeadersInit>); auth?: Record<string, CredentialSource>; authAdapters?: Record<string, AuthAdapter>; codecs{}: OpaqueCodecRegistry; fetch?: typeof fetch; webSocket?: WebSocketConstructor; signal?: AbortSignal; timeoutMs?: number; retry?: RetryOptions; transport?: Transport }};\n\n",
        if opaque_required { "" } else { "?" },
    ));
    output
}

pub(super) fn auth_types(document: &Document) -> String {
    let mut entries = Vec::new();
    for (id, scheme) in &document.auth_schemes {
        let key = string(id);
        let value = match scheme {
            AuthScheme::Bearer { .. } | AuthScheme::OAuth2 { .. } => "{ kind: \"bearer\" }".into(),
            AuthScheme::Basic => "{ kind: \"basic\" }".into(),
            AuthScheme::ApiKey { location, name } => format!(
                "{{ kind: \"api_key\", location: {}, name: {} }}",
                string(match location {
                    ApiKeyLocation::Header => "header",
                    ApiKeyLocation::Query => "query",
                    ApiKeyLocation::Cookie => "cookie",
                }),
                string(name)
            ),
            AuthScheme::CookieSession { cookie } => {
                format!("{{ kind: \"cookie\", name: {} }}", string(cookie))
            }
            AuthScheme::Custom { adapter, .. } => match adapter {
                Some(adapter) => format!("{{ kind: \"custom\", adapter: {} }}", string(adapter)),
                None => "{ kind: \"custom\" }".into(),
            },
        };
        entries.push(format!("authSchemes[{key}] = {value};"));
    }
    format!(
        "const authSchemes: Record<string, AuthSchemeDefinition> = Object.create(null);\n{}\n\n",
        entries.join("\n")
    )
}

pub(super) fn response_type(
    operation: &Operation,
    declaration_names: &[String],
    document: &Document,
    names: &BTreeMap<String, String>,
    options: &TsOptions,
) -> Result<(String, String), EmitError> {
    if let Some(StreamKind::Websocket {
        client: client_message,
        server: server_message,
    }) = operation.stream.as_ref().map(|stream| &stream.kind)
    {
        let client_name = &declaration_names[3];
        let server_name = &declaration_names[4];
        let failure_name = &declaration_names[5];
        let response = format!("WebSocketConnection<{client_name}, {server_name}>");
        let declarations = format!(
            "export type {client_name} = {};\nexport type {server_name} = {};\nexport type {failure_name} = WebSocketFailure;\n",
            types::type_ref(client_message, document, names, options)?,
            types::type_ref(server_message, document, names, options)?,
        );
        return Ok((response, declarations));
    }
    if let Some(StreamKind::Sse { events }) = operation.stream.as_ref().map(|stream| &stream.kind) {
        let event_name = &declaration_names[3];
        let failure_name = &declaration_names[4];
        let variants = events
            .iter()
            .filter(|event| event.event != "error")
            .map(|event| {
                Ok(format!(
                    "{{ event: {}; data: {} }}",
                    string(&event.event),
                    types::type_ref(&event.data, document, names, options)?
                ))
            })
            .collect::<Result<Vec<_>, EmitError>>()?;
        let error_bodies = events
            .iter()
            .filter(|event| event.event == "error")
            .map(|event| types::type_ref(&event.data, document, names, options))
            .collect::<Result<Vec<_>, EmitError>>()?;
        let error_body = if error_bodies.is_empty() {
            "never".to_owned()
        } else {
            error_bodies.join(" | ")
        };
        let stream_type = format!("AsyncIterable<Result<{event_name}, {failure_name}>>");
        let response_type =
            stream_response_type(operation, &stream_type, document, names, options)?;
        return Ok((
            response_type,
            format!(
                "export type {event_name} = {};\nexport type {failure_name} = SseStreamError<{error_body}>;\n",
                if variants.is_empty() {
                    "never".into()
                } else {
                    variants.join(" | ")
                },
            ),
        ));
    }
    if let Some(StreamKind::Ndjson { item }) = operation.stream.as_ref().map(|stream| &stream.kind)
    {
        let item_name = &declaration_names[3];
        let failure_name = &declaration_names[4];
        let stream_type = format!("AsyncIterable<Result<{item_name}, {failure_name}>>");
        return Ok((
            stream_response_type(operation, &stream_type, document, names, options)?,
            format!(
                "export type {item_name} = {};\nexport type {failure_name} = NdjsonStreamError;\n",
                types::type_ref(item, document, names, options)?
            ),
        ));
    }
    let bodies = operation
        .responses
        .iter()
        .map(|response| {
            let body = match &response.body {
                Some(body) => types::type_ref(body, document, names, options),
                None => Ok("void".into()),
            }?;
            response_value_type(body, response, document, names, options)
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok((
        if bodies.is_empty() {
            "void".into()
        } else {
            bodies.join(" | ")
        },
        String::new(),
    ))
}

fn stream_response_type(
    operation: &Operation,
    stream_type: &str,
    document: &Document,
    names: &BTreeMap<String, String>,
    options: &TsOptions,
) -> Result<String, EmitError> {
    let mut responses = Vec::new();
    for response in &operation.responses {
        let body = if response.content == ContentKind::Stream {
            stream_type.to_owned()
        } else {
            match &response.body {
                Some(body) => types::type_ref(body, document, names, options)?,
                None => "void".into(),
            }
        };
        responses.push(response_value_type(
            body, response, document, names, options,
        )?);
    }
    if responses.is_empty() {
        Ok(stream_type.into())
    } else {
        Ok(responses.join(" | "))
    }
}

fn response_value_type(
    body: String,
    response: &Response,
    document: &Document,
    names: &BTreeMap<String, String>,
    options: &TsOptions,
) -> Result<String, EmitError> {
    let body = if response.headers.is_empty() {
        body
    } else {
        let headers = response_headers_type(&response.headers, document, names, options)?;
        format!("ResponseWithHeaders<{body}, {headers}>")
    };
    if matches!(
        response.status,
        ResponseStatus::Default | ResponseStatus::Unknown
    ) {
        Ok(format!("ResponseWithStatus<{body}>"))
    } else {
        Ok(body)
    }
}

fn response_headers_type(
    headers: &[ResponseHeader],
    document: &Document,
    names: &BTreeMap<String, String>,
    options: &TsOptions,
) -> Result<String, EmitError> {
    let fields = headers
        .iter()
        .map(|header| {
            let optional = if header.presence == Presence::Optional {
                "?"
            } else {
                ""
            };
            Ok(format!(
                "{}{}: {}",
                string(&header.wire),
                optional,
                types::type_ref(&header.ty, document, names, options)?
            ))
        })
        .collect::<Result<Vec<_>, EmitError>>()?;
    Ok(format!("{{ {} }}", fields.join("; ")))
}

pub(super) fn error_type(
    errors: &[&ErrorResponse],
    document: &Document,
    names: &BTreeMap<String, String>,
    options: &TsOptions,
) -> Result<String, EmitError> {
    let mut failures = vec![(None, "ClientFailure".to_owned())];
    for error in errors {
        let status = match error.status {
            ErrorStatus::Exact(status) => status.to_string(),
            ErrorStatus::Class(_) => "number".into(),
            ErrorStatus::Default => "number".into(),
        };
        let body = error
            .body
            .as_ref()
            .map(|body| types::type_ref(body, document, names, options))
            .transpose()?;
        let headers = if error.headers.is_empty() {
            "Headers".to_owned()
        } else {
            response_headers_type(&error.headers, document, names, options)?
        };
        let body = body.unwrap_or_else(|| "unknown".into());
        failures.push((
            error.doc.as_ref(),
            format!("ApiError<{status}, {body}, {headers}>"),
        ));
    }
    Ok(union_type(&failures))
}
