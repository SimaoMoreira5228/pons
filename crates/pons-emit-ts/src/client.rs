use std::collections::BTreeMap;

use pons_ir::{
    ApiKeyLocation, AuthScheme, ContentKind, CredentialLocator, Document, ErrorContent,
    ErrorResponse, ErrorStatus, Operation, PathSegment, Presence, Response, ResponseHeader,
    ResponseStatus, StreamKind, TypeRef,
};

use crate::runtime;
use crate::{EmitError, EmitMode, ErrorStrategy, camel, pascal, string, types};

struct Node {
    children: BTreeMap<String, Node>,
    operations: Vec<(String, String)>,
}

impl Node {
    fn new() -> Self {
        Self {
            children: BTreeMap::new(),
            operations: Vec::new(),
        }
    }

    fn render(&self, depth: usize) -> String {
        let indent = " ".repeat(depth);
        let mut members = self
            .operations
            .iter()
            .map(|(name, value)| format!("{indent}{name}: {value}"))
            .collect::<Vec<_>>();
        members.extend(
            self.children
                .iter()
                .map(|(name, child)| format!("{indent}{name}: {}", child.render(depth + 2))),
        );
        format!(
            "{{\n{}\n{}}}",
            members.join(",\n"),
            " ".repeat(depth.saturating_sub(2))
        )
    }
}

pub(super) fn emit(
    document: &Document,
    names: &BTreeMap<String, String>,
    mode: EmitMode,
    preserve_wide_integers: bool,
    error_strategy: ErrorStrategy,
) -> Result<String, EmitError> {
    let mut output = if error_strategy == ErrorStrategy::Throw {
        String::from(
            "type Result<T, E> = { isOk(): boolean; isErr(): boolean; value?: T; error?: E };\nfunction ok<T>(value: T): Result<T, never> { return { isOk: () => true, isErr: () => false, value }; }\nfunction err<E>(error: E): Result<never, E> { return { isOk: () => false, isErr: () => true, error }; }\n",
        )
    } else {
        String::from("import { err, ok, type Result } from \"neverthrow\";\n")
    };
    if !document.types.is_empty() {
        output.push_str("import type * as Types from \"./types.js\";\n");
    }
    output.push('\n');
    output.push_str(&auth_types(document));
    output.push_str(&type_definitions(document)?);
    output.push_str("\nexport type AuthAttachment = { headers?: Record<string, string>; query?: Record<string, string>; cookies?: Record<string, string> };\nexport type AuthAdapter = (context: { scheme: string; scopes: readonly string[]; method: string; url: URL; headers: Headers }) => AuthAttachment | Promise<AuthAttachment>;\nexport type TokenStore = { get(): string | undefined | Promise<string | undefined>; set(value: string, expiresAt?: number): void | Promise<void>; clear(): void | Promise<void> };\nexport type CredentialSource = string | (() => string | undefined | Promise<string | undefined>) | TokenStore;\nexport type RequestInterceptor = (context: { url: URL; init: RequestInit }) => void | Promise<void>;\nexport type ResponseInterceptor = (context: { response: Response; url: URL; init: RequestInit }) => void | Promise<void>;\nexport type Interceptors = { request?: RequestInterceptor; response?: ResponseInterceptor };\nexport type ResponseWithHeaders<T, H> = { value: T; headers: H };\nexport type ResponseWithStatus<T> = { value: T; status: number };\nexport type ClientFailure = { kind: \"network\"; cause: unknown } | { kind: \"timeout\"; afterMs: number } | { kind: \"aborted\"; cause: unknown } | { kind: \"auth_adapter\"; scheme: string; cause: unknown } | { kind: \"auth_source\"; scheme: string; cause: unknown } | { kind: \"interceptor\"; phase: \"request\" | \"response\"; cause: unknown } | { kind: \"protocol\"; message: string } | { kind: \"unexpected_http\"; status: number; body: unknown } | { kind: \"auth\"; schemes: string[] };\n\n");
    output.push_str("export type RetryOptions = { attempts?: number; backoffMs?: number; maxBackoffMs?: number; jitterMs?: number };\n");
    output.push_str("export type CallOptions = { headers?: HeadersInit; signal?: AbortSignal; timeout?: number; retry?: RetryOptions | false };\n");
    output.push_str("export type ClientOptions = { baseUrl: string; headers?: HeadersInit | (() => HeadersInit | Promise<HeadersInit>); auth?: Record<string, CredentialSource>; authAdapters?: Record<string, AuthAdapter>; fetch?: typeof fetch; signal?: AbortSignal; timeout?: number; retry?: RetryOptions; interceptors?: Interceptors };\n\n");
    output.push_str(&runtime::render());

    let mut tree = Node::new();
    let mut type_names = BTreeMap::new();
    for operation in &document.operations {
        let id = operation.id.iter().flatten().cloned().collect::<Vec<_>>();
        let base = pascal(&id);
        let count = type_names.entry(base.clone()).or_insert(0usize);
        *count += 1;
        let unique = if *count == 1 {
            base
        } else {
            format!("{base}{count}")
        };
        let (declarations, function) = operation_function(
            operation,
            &unique,
            document,
            names,
            mode,
            preserve_wide_integers,
            error_strategy,
        )?;
        output.push_str(&declarations);
        let (parents, leaf) = operation
            .id
            .split_last()
            .map(|(leaf, parents)| (parents, camel(leaf)))
            .ok_or_else(|| EmitError("operation id cannot be empty".into()))?;
        let mut node = &mut tree;
        for parent in parents {
            let segment = camel(parent);
            if node.operations.iter().any(|(name, _)| name == &segment) {
                return Err(EmitError(format!("client member collision at {segment}")));
            }
            node = node.children.entry(segment).or_insert_with(Node::new);
        }
        if node.children.contains_key(&leaf)
            || node.operations.iter().any(|(name, _)| name == &leaf)
        {
            return Err(EmitError(format!("client member collision at {leaf}")));
        }
        node.operations.push((leaf, function));
    }
    output.push_str("export function createClient(client: ClientOptions) {\n  const auth = { ...client.auth };\n  const authValues: Record<string, string> = {};\n  const authExpirations: Record<string, number> = {};\n  const configured = { ...client, auth, authValues, authExpirations };\n  return ");
    output.push_str(&tree.render(2));
    output.push_str(";\n}\n");
    Ok(output)
}

fn auth_types(document: &Document) -> String {
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
            AuthScheme::Custom { .. } => "{ kind: \"custom\" }".into(),
        };
        entries.push(format!("  {key}: {value}"));
    }
    format!(
        "const authSchemes: Record<string, {{ kind: string; location?: string; name?: string }}> = {{\n{}\n}};\n",
        entries.join(",\n")
    )
}

fn type_definitions(document: &Document) -> Result<String, EmitError> {
    let definitions = document
        .types
        .iter()
        .map(|(id, definition)| {
            let mut value = serde_json::to_value(&definition.definition)
                .map_err(|error| EmitError(format!("cannot encode type metadata: {error}")))?;
            normalize_integer_constraints(&mut value);
            Ok((id.clone(), value))
        })
        .collect::<Result<BTreeMap<_, _>, _>>()?;
    let definitions = serde_json::to_string(&definitions)
        .map_err(|error| EmitError(format!("cannot encode type metadata: {error}")))?;
    Ok(format!(
        "const typeDefinitions: Record<string, any> = {definitions};\n"
    ))
}

fn operation_function(
    operation: &Operation,
    name: &str,
    document: &Document,
    names: &BTreeMap<String, String>,
    mode: EmitMode,
    preserve_wide_integers: bool,
    error_strategy: ErrorStrategy,
) -> Result<(String, String), EmitError> {
    let args_name = format!("{name}Args");
    let success_name = format!("{name}Response");
    let error_name = format!("{name}Error");
    let args = operation_args(operation, document, names, mode)?;
    let (success, stream_types) = response_type(operation, name, document, names, mode)?;
    let errors = if operation.errors_inherit {
        operation
            .errors
            .iter()
            .chain(document.common_errors.iter())
            .collect::<Vec<_>>()
    } else {
        operation.errors.iter().collect()
    };
    let failure = error_type(&errors, document, names, mode)?;
    let path = path_expression(operation);
    let query = if operation.query.is_some() {
        "args.query"
    } else {
        "undefined"
    };
    let headers = if operation.headers.is_empty() {
        "undefined"
    } else {
        "args.headers"
    };
    let body = if operation.body.is_some() {
        "args.body"
    } else {
        "undefined"
    };
    let body_schema = operation
        .body
        .as_ref()
        .map(|body| type_ref_json(&body.ty))
        .transpose()?
        .unwrap_or_else(|| "undefined".into());
    let request_inputs = request_input_descriptor(operation)?;
    let path_values = if operation
        .path
        .iter()
        .any(|segment| matches!(segment, PathSegment::Param { .. }))
    {
        "args.path"
    } else {
        "undefined"
    };
    let response_schemas = response_descriptors(operation)?;
    let error_schemas = errors
        .iter()
        .map(|error| error_descriptor(error))
        .collect::<Result<Vec<_>, _>>()?
        .join(", ");
    let content = operation
        .body
        .as_ref()
        .map(|body| string(content_name(body.content)))
        .unwrap_or_else(|| "undefined".into());
    let media_type = operation
        .body
        .as_ref()
        .and_then(|body| body.media_type.as_deref())
        .map(string)
        .unwrap_or_else(|| "undefined".into());
    let groups = operation
        .auth
        .alternatives
        .iter()
        .map(|group| {
            format!(
                "[{}]",
                group
                    .iter()
                    .map(|scheme| string(scheme))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    let scopes = operation
        .auth
        .scopes
        .iter()
        .map(|(scheme, scopes)| {
            format!(
                "{}: [{}]",
                string(scheme),
                scopes
                    .iter()
                    .map(|scope| string(scope))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    let provides = operation
        .provides
        .as_ref()
        .map(|provides| {
            let locator = |locator: &CredentialLocator| match locator {
                CredentialLocator::Body { pointer } => {
                    format!("{{ from: \"body\", pointer: {} }}", string(pointer))
                }
                CredentialLocator::Header { name } => {
                    format!("{{ from: \"header\", name: {} }}", string(name))
                }
                CredentialLocator::SetCookie { name } => {
                    format!("{{ from: \"set_cookie\", name: {} }}", string(name))
                }
            };
            let value = locator(&provides.value);
            let expires_at = provides
                .expires_at
                .as_ref()
                .map(locator)
                .unwrap_or_else(|| "undefined".into());
            format!(
                "{{ scheme: {}, value: {value}, expiresAt: {expires_at} }}",
                string(&provides.scheme)
            )
        })
        .unwrap_or_else(|| "undefined".into());
    let method = format!("{:?}", operation.method).to_uppercase();
    let (args_type, options) = if args.is_empty() {
        ("(_options?: CallOptions) =>".to_owned(), "_options")
    } else {
        (
            format!("(args: {args_name}, options?: CallOptions) =>"),
            "options",
        )
    };
    let declarations = format!(
        "export type {args_name} = {{ {args} }};\n{stream_types}export type {success_name} = {success};\nexport type {error_name} = {failure};\n",
    );
    let request = format!(
        "request<{success_name}, {error_name}>(configured, {method:?}, {path}, {path_values}, {query}, {headers}, {body}, {content}, {media_type}, [{groups}], {{{scopes}}}, {body_schema}, {request_inputs}, [{response_schemas}], [{error_schemas}], {}, {}, {provides}, {options}, {})",
        operation.auth.optional, preserve_wide_integers, operation.semantics.idempotent,
    );
    let function = if error_strategy == ErrorStrategy::Throw {
        format!("{args_type} requestOrThrow({request})")
    } else {
        format!("{args_type} {request}")
    };
    Ok((declarations, function))
}

fn operation_args(
    operation: &Operation,
    document: &Document,
    names: &BTreeMap<String, String>,
    mode: EmitMode,
) -> Result<String, EmitError> {
    let mut fields = Vec::new();
    let mut path_params = Vec::new();
    for segment in &operation.path {
        if let PathSegment::Param { param, ty } = segment {
            path_params.push(format!(
                "{}: {}",
                string(param),
                types::type_ref(ty, document, names, mode)?
            ));
        }
    }
    if !path_params.is_empty() {
        fields.push(format!("path: {{ {} }}", path_params.join("; ")));
    }
    if let Some(query) = &operation.query {
        fields.push(format!(
            "query: {}",
            types::type_ref(query, document, names, mode)?
        ));
    }
    if !operation.headers.is_empty() {
        let headers = operation
            .headers
            .iter()
            .map(|header| {
                let optional = if header.presence == pons_ir::Presence::Optional {
                    "?"
                } else {
                    ""
                };
                Ok(format!(
                    "{}{}: {}",
                    string(&header.wire),
                    optional,
                    types::type_ref(&header.ty, document, names, mode)?
                ))
            })
            .collect::<Result<Vec<_>, EmitError>>()?;
        fields.push(format!("headers: {{ {} }}", headers.join("; ")));
    }
    if let Some(body) = &operation.body {
        let optional = if body.required { "" } else { "?" };
        fields.push(format!(
            "body{optional}: {}",
            types::type_ref(&body.ty, document, names, mode)?
        ));
    }
    Ok(fields.join("; "))
}

fn response_type(
    operation: &Operation,
    name: &str,
    document: &Document,
    names: &BTreeMap<String, String>,
    mode: EmitMode,
) -> Result<(String, String), EmitError> {
    if let Some(StreamKind::Sse { events }) = operation.stream.as_ref().map(|stream| &stream.kind) {
        let event_name = format!("{name}Event");
        let failure_name = format!("{name}StreamError");
        let variants = events
            .iter()
            .filter(|event| event.event != "error")
            .map(|event| {
                Ok(format!(
                    "{{ event: {}; data: {} }}",
                    string(&event.event),
                    types::type_ref(&event.data, document, names, mode)?
                ))
            })
            .collect::<Result<Vec<_>, EmitError>>()?;
        let mut stream_failures = vec![
            "{ kind: \"stream_error\"; cause: unknown }".to_owned(),
            "{ kind: \"unknown_event\"; event: string }".to_owned(),
            "{ kind: \"invalid_event\"; event: string; cause: unknown }".to_owned(),
        ];
        for event in events.iter().filter(|event| event.event == "error") {
            stream_failures.push(format!(
                "{{ kind: \"server_error\"; body: {} }}",
                types::type_ref(&event.data, document, names, mode)?
            ));
        }
        let stream_type = format!("AsyncIterable<Result<{event_name}, {failure_name}>>");
        let response_type = stream_response_type(operation, &stream_type, document, names, mode)?;
        return Ok((
            response_type,
            format!(
                "export type {event_name} = {};\nexport type {failure_name} = {};\n",
                if variants.is_empty() {
                    "never".into()
                } else {
                    variants.join(" | ")
                },
                stream_failures.join(" | ")
            ),
        ));
    }
    if let Some(StreamKind::Ndjson { item }) = operation.stream.as_ref().map(|stream| &stream.kind)
    {
        let item_name = format!("{name}Item");
        let failure_name = format!("{name}StreamError");
        let stream_type = format!("AsyncIterable<Result<{item_name}, {failure_name}>>");
        return Ok((
            stream_response_type(operation, &stream_type, document, names, mode)?,
            format!(
                "export type {item_name} = {};\nexport type {failure_name} = {{ kind: \"stream_error\"; cause: unknown }} | {{ kind: \"invalid_item\"; cause: unknown }};\n",
                types::type_ref(item, document, names, mode)?
            ),
        ));
    }
    let bodies = operation
        .responses
        .iter()
        .map(|response| {
            let body = match &response.body {
                Some(body) => types::type_ref(body, document, names, mode),
                None => Ok("void".into()),
            }?;
            response_value_type(body, response, document, names, mode)
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
    mode: EmitMode,
) -> Result<String, EmitError> {
    let responses = operation
        .responses
        .iter()
        .filter(|response| response.content == ContentKind::Stream)
        .map(|response| response_value_type(stream_type.into(), response, document, names, mode))
        .collect::<Result<Vec<_>, _>>()?;
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
    mode: EmitMode,
) -> Result<String, EmitError> {
    let body = if response.headers.is_empty() {
        body
    } else {
        let headers = response_headers_type(&response.headers, document, names, mode)?;
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
    mode: EmitMode,
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
                types::type_ref(&header.ty, document, names, mode)?
            ))
        })
        .collect::<Result<Vec<_>, EmitError>>()?;
    Ok(format!("{{ {} }}", fields.join("; ")))
}

fn error_type(
    errors: &[&ErrorResponse],
    document: &Document,
    names: &BTreeMap<String, String>,
    mode: EmitMode,
) -> Result<String, EmitError> {
    let mut failures = vec!["ClientFailure".into()];
    for error in errors {
        let status = match error.status {
            ErrorStatus::Exact(status) => status.to_string(),
            ErrorStatus::Class(_) => "number".into(),
            ErrorStatus::Default => "number".into(),
        };
        let body = error
            .body
            .as_ref()
            .map(|body| types::type_ref(body, document, names, mode))
            .transpose()?;
        let headers = if error.headers.is_empty() {
            "Headers".to_owned()
        } else {
            response_headers_type(&error.headers, document, names, mode)?
        };
        let body = body.unwrap_or_else(|| "unknown".into());
        failures.push(format!(
            "{{ kind: \"http\"; status: {status}; body: {body}; headers: {headers} }}"
        ));
    }
    Ok(failures.join(" | "))
}

fn request_input_descriptor(operation: &Operation) -> Result<String, EmitError> {
    let path = operation
        .path
        .iter()
        .filter_map(|segment| match segment {
            PathSegment::Param { param, ty } => Some((param, ty)),
            PathSegment::Literal { .. } => None,
        })
        .map(|(param, ty)| {
            Ok(format!(
                "{{ name: {}, required: true, schema: {} }}",
                string(param),
                type_ref_json(ty)?
            ))
        })
        .collect::<Result<Vec<_>, EmitError>>()?
        .join(", ");
    let query = operation
        .query
        .as_ref()
        .map(type_ref_json)
        .transpose()?
        .unwrap_or_else(|| "undefined".into());
    let headers = operation
        .headers
        .iter()
        .map(|header| {
            Ok(format!(
                "{{ name: {}, required: {}, schema: {} }}",
                string(&header.wire),
                header.presence == Presence::Required,
                type_ref_json(&header.ty)?
            ))
        })
        .collect::<Result<Vec<_>, EmitError>>()?
        .join(", ");
    let body_required = operation.body.as_ref().is_some_and(|body| body.required);
    Ok(format!(
        "{{ path: [{path}], query: {query}, headers: [{headers}], bodyRequired: {body_required} }}"
    ))
}

fn path_expression(operation: &Operation) -> String {
    if operation.path.is_empty() {
        return "\"/\"".into();
    }
    let has_params = operation
        .path
        .iter()
        .any(|segment| matches!(segment, PathSegment::Param { .. }));
    let segments = operation
        .path
        .iter()
        .map(|segment| match segment {
            PathSegment::Literal { lit } => string(lit),
            PathSegment::Param { param, .. } => {
                format!("encodeURIComponent(String(pathValues[{}]))", string(param))
            }
        })
        .collect::<Vec<_>>();
    let path = format!("[\"\", {}].join(\"/\")", segments.join(", "));
    if has_params {
        format!("(pathValues) => {path}")
    } else {
        path
    }
}

fn content_name(content: ContentKind) -> &'static str {
    match content {
        ContentKind::Json => "json",
        ContentKind::ProblemJson => "problem_json",
        ContentKind::Form => "form",
        ContentKind::Multipart => "multipart",
        ContentKind::Text => "text",
        ContentKind::Bytes => "bytes",
        ContentKind::Stream => "stream",
        ContentKind::Empty => "empty",
    }
}

fn response_descriptors(operation: &Operation) -> Result<String, EmitError> {
    let stream_descriptor = match operation.stream.as_ref().map(|stream| &stream.kind) {
        Some(StreamKind::Sse { events }) => Some(format!(
            "stream: \"sse\", events: [{}]",
            events
                .iter()
                .map(|event| {
                    Ok(format!(
                        "{{ event: {}, schema: {} }}",
                        string(&event.event),
                        type_ref_json(&event.data)?
                    ))
                })
                .collect::<Result<Vec<_>, EmitError>>()?
                .join(", ")
        )),
        Some(StreamKind::Ndjson { item }) => Some(format!(
            "stream: \"ndjson\", item: {}",
            type_ref_json(item)?
        )),
        _ => None,
    };
    operation
        .responses
        .iter()
        .map(|response| {
            let schema = response
                .body
                .as_ref()
                .map(type_ref_json)
                .transpose()?
                .unwrap_or_else(|| "undefined".into());
            let stream = if response.content == ContentKind::Stream {
                stream_descriptor
                    .as_ref()
                    .map_or(String::new(), |stream| format!(", {stream}"))
            } else {
                String::new()
            };
            let media_type = response
                .media_type
                .as_deref()
                .map(|value| format!(", mediaType: {}", string(value)))
                .unwrap_or_default();
            let headers = response
                .headers
                .iter()
                .map(|header| {
                    Ok(format!(
                        "{{ name: {}, required: {}, schema: {} }}",
                        string(&header.wire),
                        header.presence == Presence::Required,
                        type_ref_json(&header.ty)?
                    ))
                })
                .collect::<Result<Vec<_>, EmitError>>()?
                .join(", ");
            let headers = if headers.is_empty() {
                String::new()
            } else {
                format!(", headers: [{headers}]")
            };
            let status = serde_json::to_string(&response.status)
                .map_err(|error| EmitError(format!("cannot encode response status: {error}")))?;
            Ok(format!(
                "{{ status: {status}, content: {}, schema: {schema}{media_type}{stream}{headers} }}",
                string(content_name(response.content))
            ))
        })
        .collect::<Result<Vec<_>, EmitError>>()
        .map(|descriptors| descriptors.join(", "))
}

fn error_descriptor(error: &ErrorResponse) -> Result<String, EmitError> {
    let status = serde_json::to_string(&error.status)
        .map_err(|error| EmitError(format!("cannot encode error status: {error}")))?;
    let content = match error.content {
        ErrorContent::Json => "json",
        ErrorContent::ProblemJson => "problem_json",
        ErrorContent::Text => "text",
        ErrorContent::Empty => "empty",
    };
    let schema = error
        .body
        .as_ref()
        .map(type_ref_json)
        .transpose()?
        .unwrap_or_else(|| "undefined".into());
    let headers = error
        .headers
        .iter()
        .map(|header| {
            Ok(format!(
                "{{ name: {}, required: {}, schema: {} }}",
                string(&header.wire),
                header.presence == Presence::Required,
                type_ref_json(&header.ty)?
            ))
        })
        .collect::<Result<Vec<_>, EmitError>>()?
        .join(", ");
    let headers = if headers.is_empty() {
        String::new()
    } else {
        format!(", headers: [{headers}]")
    };
    let retryable = error
        .retryable
        .map(|value| format!(", retryable: {value}"))
        .unwrap_or_default();
    let media_type = error
        .media_type
        .as_deref()
        .map(|value| format!(", mediaType: {}", string(value)))
        .unwrap_or_default();
    Ok(format!(
        "{{ status: {status}, content: {}, schema: {schema}{media_type}{retryable}{headers} }}",
        string(content)
    ))
}

fn type_ref_json(reference: &TypeRef) -> Result<String, EmitError> {
    let mut value = serde_json::to_value(reference)
        .map_err(|error| EmitError(format!("cannot encode type reference: {error}")))?;
    normalize_integer_constraints(&mut value);
    serde_json::to_string(&value)
        .map_err(|error| EmitError(format!("cannot encode type reference: {error}")))
}

fn normalize_integer_constraints(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(fields) => {
            if fields.get("kind").and_then(serde_json::Value::as_str) == Some("int") {
                for key in ["minimum", "maximum", "multiple_of"] {
                    if let Some(serde_json::Value::Number(number)) = fields.get(key) {
                        let encoded = number.to_string();
                        fields.insert(key.into(), serde_json::Value::String(encoded));
                    }
                }
            }
            for child in fields.values_mut() {
                normalize_integer_constraints(child);
            }
        }
        serde_json::Value::Array(values) => {
            for child in values {
                normalize_integer_constraints(child);
            }
        }
        _ => {}
    }
}
