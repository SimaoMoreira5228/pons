use std::collections::BTreeMap;

use pons_ir::{
    ApiKeyLocation, AuthScheme, ContentKind, CredentialLocator, Document, ErrorContent,
    ErrorResponse, ErrorStatus, Operation, PathSegment, Presence, StreamKind, TypeRef,
};

use crate::runtime;
use crate::{EmitError, camel, pascal, string, types};

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
) -> Result<String, EmitError> {
    let mut output = String::from(
        "import { err, ok, type Result } from \"neverthrow\";\nimport type * as Types from \"./types.js\";\n\n",
    );
    output.push_str(&auth_types(document));
    output.push_str(&type_definitions(document)?);
    output.push_str("\nexport type ClientFailure = { kind: \"network\"; cause: unknown } | { kind: \"protocol\"; message: string } | { kind: \"unexpected_http\"; status: number; body: unknown } | { kind: \"auth\"; schemes: string[] };\n\n");
    output.push_str("export type ClientOptions = { baseUrl: string; headers?: HeadersInit; auth?: Record<string, string>; fetch?: typeof fetch };\n\n");
    output.push_str(runtime::render());

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
        let (declarations, function) = operation_function(operation, &unique, document, names)?;
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
    output.push_str("export function createClient(client: ClientOptions) {\n  const auth = { ...client.auth };\n  const configured = { ...client, auth };\n  return ");
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
            serde_json::to_value(&definition.definition)
                .map(|value| (id.clone(), value))
                .map_err(|error| EmitError(format!("cannot encode type metadata: {error}")))
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
) -> Result<(String, String), EmitError> {
    let args_name = format!("{name}Args");
    let success_name = format!("{name}Response");
    let error_name = format!("{name}Error");
    let args = operation_args(operation, document, names)?;
    let (success, stream_types) = response_type(operation, name, document, names)?;
    let errors = if operation.errors_inherit {
        operation
            .errors
            .iter()
            .chain(document.common_errors.iter())
            .collect::<Vec<_>>()
    } else {
        operation.errors.iter().collect()
    };
    let failure = error_type(&errors, document, names)?;
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
    let provides = operation
        .provides
        .as_ref()
        .map(|provides| {
            let locator = match &provides.value {
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
            format!(
                "{{ scheme: {}, value: {locator} }}",
                string(&provides.scheme)
            )
        })
        .unwrap_or_else(|| "undefined".into());
    let method = format!("{:?}", operation.method).to_uppercase();
    let args_type = if args.is_empty() {
        "() =>".to_owned()
    } else {
        format!("(args: {args_name}) =>")
    };
    let declarations = format!(
        "export type {args_name} = {{ {args} }};\n{stream_types}export type {success_name} = {success};\nexport type {error_name} = {failure};\n",
    );
    let function = format!(
        "{args_type} request<{success_name}, {error_name}>(configured, {method:?}, {path}, {query}, {headers}, {body}, {content}, [{groups}], {body_schema}, [{response_schemas}], [{error_schemas}], {}, {provides})",
        operation.auth.optional,
    );
    Ok((declarations, function))
}

fn operation_args(
    operation: &Operation,
    document: &Document,
    names: &BTreeMap<String, String>,
) -> Result<String, EmitError> {
    let mut fields = Vec::new();
    let mut path_params = Vec::new();
    for segment in &operation.path {
        if let PathSegment::Param { param, ty } = segment {
            path_params.push(format!(
                "{}: {}",
                string(param),
                types::type_ref(ty, document, names)?
            ));
        }
    }
    if !path_params.is_empty() {
        fields.push(format!("path: {{ {} }}", path_params.join("; ")));
    }
    if let Some(query) = &operation.query {
        fields.push(format!(
            "query: {}",
            types::type_ref(query, document, names)?
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
                    types::type_ref(&header.ty, document, names)?
                ))
            })
            .collect::<Result<Vec<_>, EmitError>>()?;
        fields.push(format!("headers: {{ {} }}", headers.join("; ")));
    }
    if let Some(body) = &operation.body {
        fields.push(format!(
            "body: {}",
            types::type_ref(&body.ty, document, names)?
        ));
    }
    Ok(fields.join("; "))
}

fn response_type(
    operation: &Operation,
    name: &str,
    document: &Document,
    names: &BTreeMap<String, String>,
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
                    types::type_ref(&event.data, document, names)?
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
                types::type_ref(&event.data, document, names)?
            ));
        }
        return Ok((
            format!("AsyncIterable<Result<{event_name}, {failure_name}>>"),
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
    let bodies = operation
        .responses
        .iter()
        .map(|response| match &response.body {
            Some(body) => types::type_ref(body, document, names),
            None => Ok("void".into()),
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

fn error_type(
    errors: &[&ErrorResponse],
    document: &Document,
    names: &BTreeMap<String, String>,
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
            .map(|body| types::type_ref(body, document, names))
            .transpose()?;
        failures.push(match body {
            Some(body) => format!("{{ kind: \"http\"; status: {status}; body: {body} }}"),
            None => format!("{{ kind: \"http\"; status: {status}; body: unknown }}"),
        });
    }
    Ok(failures.join(" | "))
}

fn path_expression(operation: &Operation) -> String {
    if operation.path.is_empty() {
        return "\"/\"".into();
    }
    let segments = operation
        .path
        .iter()
        .map(|segment| match segment {
            PathSegment::Literal { lit } => string(lit),
            PathSegment::Param { param, .. } => {
                format!("encodeURIComponent(String(args.path[{}]))", string(param))
            }
        })
        .collect::<Vec<_>>();
    format!("[\"\", {}].join(\"/\")", segments.join(", "))
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
    let events = match operation.stream.as_ref().map(|stream| &stream.kind) {
        Some(StreamKind::Sse { events }) => Some(
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
                .join(", "),
        ),
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
            let events = if response.content == ContentKind::Stream {
                events
                    .as_ref()
                    .map_or(String::new(), |events| format!(", events: [{events}]"))
            } else {
                String::new()
            };
            let headers = response
                .headers
                .iter()
                .map(|header| {
                    format!(
                        "{{ name: {}, required: {} }}",
                        string(&header.wire),
                        header.presence == Presence::Required
                    )
                })
                .collect::<Vec<_>>()
                .join(", ");
            let headers = if headers.is_empty() {
                String::new()
            } else {
                format!(", headers: [{headers}]")
            };
            Ok(format!(
                "{{ status: {}, content: {}, schema: {schema}{events}{headers} }}",
                response.status,
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
    Ok(format!(
        "{{ status: {status}, content: {}, schema: {schema} }}",
        string(content)
    ))
}

fn type_ref_json(reference: &TypeRef) -> Result<String, EmitError> {
    serde_json::to_string(reference)
        .map_err(|error| EmitError(format!("cannot encode type reference: {error}")))
}
