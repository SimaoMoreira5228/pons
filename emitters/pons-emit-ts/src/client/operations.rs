use std::collections::BTreeMap;

use pons_ir::{
    AuthRequirement, ContentKind, CredentialLocator, CredentialProvision, Document, ErrorContent,
    ErrorResponse, Operation, PathSegment, Presence, ResponseHeader, StreamKind, TypeRef,
    operation_subject,
};

use crate::pagination::{Carrier, Plan, RequestKind};
use crate::render::string;
use crate::{DateMode, EmitError, EmitMode, ErrorStrategy, Int64, TsOptions, types, validation};

use super::signatures::{error_type, response_type};

#[allow(clippy::too_many_arguments)]
pub(super) fn operation_function(
    operation: &Operation,
    declaration_names: &[String],
    document: &Document,
    names: &BTreeMap<String, String>,
    options: &TsOptions,
    preserve_wide_integers: bool,
    validation: &mut validation::Backend,
    pagination: Option<&Plan>,
) -> Result<(String, String, Option<String>), EmitError> {
    let args_name = &declaration_names[0];
    let success_name = &declaration_names[1];
    let error_name = &declaration_names[2];
    let args = operation_args(operation, document, names, options)?;
    let (success, stream_types) =
        response_type(operation, declaration_names, document, names, options)?;
    let errors = if operation.errors_inherit {
        operation
            .errors
            .iter()
            .chain(document.common_errors.iter())
            .collect::<Vec<_>>()
    } else {
        operation.errors.iter().collect()
    };
    let failure = error_type(&errors, document, names, options)?;
    let subject = format!("operations.{}", operation_subject(operation));
    let method = operation.method.as_str();
    let (args_type, call_options) = if args.is_empty() {
        ("(_options?: CallOptions) =>".to_owned(), "_options")
    } else {
        (
            format!("(args: {args_name}, options?: CallOptions) =>"),
            "options",
        )
    };
    if let Some(StreamKind::Websocket {
        client: client_message,
        server: server_message,
    }) = operation.stream.as_ref().map(|stream| &stream.kind)
    {
        let client_name = &declaration_names[3];
        let server_name = &declaration_names[4];
        let spec = websocket_spec(
            operation,
            client_message,
            server_message,
            options,
            validation,
            &subject,
            preserve_wide_integers,
            call_options,
        )?;
        let declarations = format!(
            "export type {args_name} = {{ {args} }};\n{stream_types}export type {success_name} = {success};\nexport type {error_name} = {failure};\n",
        );
        let call = format!("websocket<{client_name}, {server_name}>(configured, {{ {spec} }})");
        let function = if options.errors == ErrorStrategy::Throw {
            format!("{args_type} requestOrThrow<{success_name}, {error_name}>({call})")
        } else {
            format!("{args_type} {call}")
        };
        return Ok((declarations, function, None));
    }
    let mut spec = vec![
        format!("method: {method:?}"),
        format!("path: {}", path_expression(operation)),
    ];
    if operation
        .path
        .iter()
        .any(|segment| matches!(segment, PathSegment::Param { .. }))
    {
        spec.push(String::from("pathValues: args.path"));
    }
    if operation.query.is_some() {
        spec.push(String::from("query: args.query"));
    }
    if !operation.headers.is_empty() {
        spec.push(String::from("headers: args.headers"));
    }
    if let Some(body) = &operation.body {
        spec.push(String::from("body: args.body"));
        spec.push(format!(
            "bodyContent: {}",
            string(content_name(body.content))
        ));
        if let Some(media) = &body.media {
            spec.push(format!("bodyMediaType: {}", string(&media.to_string())));
        }
        spec.push(format!("bodySchema: {}", type_ref_json(&body.ty, options)?));
    }
    if !operation.auth.alternatives.is_empty() {
        spec.push(format!("auth: {}", auth_requirement(&operation.auth)));
    }
    spec.push(format!(
        "inputs: {}",
        request_input_descriptor(operation, options)?
    ));
    spec.push(format!(
        "responses: [{}]",
        response_descriptors(operation, options, validation)?
    ));
    spec.push(format!(
        "errors: [{}]",
        errors
            .iter()
            .map(|error| error_descriptor(error, options, validation, &subject))
            .collect::<Result<Vec<_>, _>>()?
            .join(", ")
    ));
    if preserve_wide_integers {
        spec.push(String::from("preserveWideIntegers: true"));
    }
    if let Some(provides) = &operation.provides {
        spec.push(format!("provides: {}", credential_provision(provides)));
    }
    spec.push(format!("options: {call_options}"));
    if operation.semantics.idempotent {
        spec.push(String::from("idempotent: true"));
    }
    let declarations = format!(
        "export type {args_name} = {{ {args} }};\n{stream_types}export type {success_name} = {success};\nexport type {error_name} = {failure};\n",
    );
    let request = format!(
        "request<{success_name}, {error_name}>(configured, {{ {} }})",
        spec.join(", ")
    );
    let function = if options.errors == ErrorStrategy::Throw {
        format!("{args_type} requestOrThrow({request})")
    } else {
        format!("{args_type} {request}")
    };
    let iterator = pagination
        .map(|plan| {
            let item = types::type_ref(&plan.item, document, names, options)?;
            pagination_member(plan, &spec, &args_type, success_name, error_name, &item)
        })
        .transpose()?;
    Ok((declarations, function, iterator))
}

fn pagination_member(
    plan: &Plan,
    spec: &[String],
    args_type: &str,
    success_name: &str,
    error_name: &str,
    item: &str,
) -> Result<String, EmitError> {
    let query_override = plan.carrier == Carrier::Query
        || plan
            .page_size
            .as_ref()
            .is_some_and(|size| size.carrier == Carrier::Query);
    let body_override = plan.carrier == Carrier::Body
        || plan
            .page_size
            .as_ref()
            .is_some_and(|size| size.carrier == Carrier::Body);
    let page_spec = spec
        .iter()
        .map(|part| match part.as_str() {
            "query: args.query" if query_override => {
                "query: { ...args.query, ...overrides.query }".to_owned()
            }
            "body: args.body" if body_override => {
                "body: { ...args.body, ...overrides.body }".to_owned()
            }
            _ => part.clone(),
        })
        .collect::<Vec<_>>()
        .join(", ");
    let descriptor = pagination_descriptor(plan);
    Ok(format!(
        "{args_type} paginate<{item}, {error_name}>((overrides) => \
         request<{success_name}, {error_name}>(configured, {{ {page_spec} }}), {descriptor})"
    ))
}

fn pagination_descriptor(plan: &Plan) -> String {
    let mut parts = vec![
        format!("kind: {}", string(plan.kind.name())),
        format!("field: {}", string(&plan.field)),
        format!("carrier: {}", string(plan.carrier.name())),
        format!("items: {}", string(&plan.items)),
    ];
    if let Some(next) = &plan.next {
        parts.push(format!("next: {}", string(next)));
    }
    if plan.kind == RequestKind::Offset {
        parts.push(format!(
            "initial: {}",
            field_read(plan.carrier, &plan.field)
        ));
    }
    if let Some(size) = &plan.page_size {
        let mut descriptor = vec![
            format!("field: {}", string(&size.field)),
            format!("carrier: {}", string(size.carrier.name())),
        ];
        if let Some(default) = size.default {
            descriptor.push(format!("default: {default}"));
        }
        descriptor.push(format!(
            "initial: {}",
            field_read(size.carrier, &size.field)
        ));
        parts.push(format!("pageSize: {{ {} }}", descriptor.join(", ")));
    }
    parts.push(format!(
        "wrap: {{ status: {}, headers: {} }}",
        plan.wrap_status, plan.wrap_headers
    ));
    format!("{{ {} }}", parts.join(", "))
}

fn field_read(carrier: Carrier, field: &str) -> String {
    match carrier {
        Carrier::Query => format!("args.query[{}]", string(field)),
        Carrier::Body => format!("args.body?.[{}]", string(field)),
    }
}

fn auth_requirement(auth: &AuthRequirement) -> String {
    let alternatives = auth
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
    let mut keys = vec![format!("alternatives: [{alternatives}]")];
    if !auth.scopes.is_empty() {
        let scopes = auth
            .scopes
            .iter()
            .map(|(scheme, scopes)| {
                let key = string(scheme);
                let key = if scheme == "__proto__" {
                    format!("[{key}]")
                } else {
                    key
                };
                format!(
                    "{key}: [{}]",
                    scopes
                        .iter()
                        .map(|scope| string(scope))
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            })
            .collect::<Vec<_>>()
            .join(", ");
        keys.push(format!("scopes: {{{scopes}}}"));
    }
    if auth.optional {
        keys.push(String::from("optional: true"));
    }
    format!("{{ {} }}", keys.join(", "))
}

fn credential_provision(provides: &CredentialProvision) -> String {
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
}
fn operation_args(
    operation: &Operation,
    document: &Document,
    names: &BTreeMap<String, String>,
    options: &TsOptions,
) -> Result<String, EmitError> {
    let mut fields = Vec::new();
    let mut path_params = Vec::new();
    for segment in &operation.path {
        if let PathSegment::Param { param, ty } = segment {
            path_params.push(format!(
                "{}: {}",
                string(param),
                types::type_ref(ty, document, names, options)?
            ));
        }
    }
    if !path_params.is_empty() {
        fields.push(format!("path: {{ {} }}", path_params.join("; ")));
    }
    if let Some(query) = &operation.query {
        fields.push(format!(
            "query: {}",
            types::type_ref(query, document, names, options)?
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
                    types::type_ref(&header.ty, document, names, options)?
                ))
            })
            .collect::<Result<Vec<_>, EmitError>>()?;
        let bag = if operation
            .headers
            .iter()
            .all(|header| header.presence == pons_ir::Presence::Optional)
        {
            "?"
        } else {
            ""
        };
        fields.push(format!("headers{bag}: {{ {} }}", headers.join("; ")));
    }
    if let Some(body) = &operation.body {
        let optional = if body.required { "" } else { "?" };
        fields.push(format!(
            "body{optional}: {}",
            types::type_ref(&body.ty, document, names, options)?
        ));
    }
    Ok(fields.join("; "))
}

fn request_input_descriptor(
    operation: &Operation,
    options: &TsOptions,
) -> Result<String, EmitError> {
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
                type_ref_json(ty, options)?
            ))
        })
        .collect::<Result<Vec<_>, EmitError>>()?
        .join(", ");
    let query = operation
        .query
        .as_ref()
        .map(|ty| type_ref_json(ty, options))
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
                type_ref_json(&header.ty, options)?
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
            PathSegment::Literal { lit } => format!("encodePathSegment({})", string(lit)),
            PathSegment::Param { param, .. } => {
                format!(
                    "encodePathSegment(stringifyCarrier(pathValues[{}]))",
                    string(param)
                )
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

#[allow(clippy::too_many_arguments)]
fn websocket_spec(
    operation: &Operation,
    client_message: &TypeRef,
    server_message: &TypeRef,
    options: &TsOptions,
    validation: &mut validation::Backend,
    subject: &str,
    preserve_wide_integers: bool,
    call_options: &str,
) -> Result<String, EmitError> {
    let mut spec = vec![format!("path: {}", path_expression(operation))];
    if operation
        .path
        .iter()
        .any(|segment| matches!(segment, PathSegment::Param { .. }))
    {
        spec.push(String::from("pathValues: args.path"));
    }
    if operation.query.is_some() {
        spec.push(String::from("query: args.query"));
    }
    if !operation.headers.is_empty() {
        spec.push(String::from("headers: args.headers"));
    }
    if !operation.auth.alternatives.is_empty() {
        spec.push(format!("auth: {}", auth_requirement(&operation.auth)));
    }
    spec.push(format!(
        "inputs: {}",
        request_input_descriptor(operation, options)?
    ));
    spec.push(format!(
        "client: {{ schema: {} }}",
        type_ref_json(client_message, options)?
    ));
    let validate = validator(Some(server_message), validation, subject)?;
    spec.push(format!(
        "server: {{ schema: {}{validate} }}",
        type_ref_json(server_message, options)?
    ));
    if preserve_wide_integers {
        spec.push(String::from("preserveWideIntegers: true"));
    }
    spec.push(format!("options: {call_options}"));
    Ok(spec.join(", "))
}

fn response_descriptors(
    operation: &Operation,
    options: &TsOptions,
    validation: &mut validation::Backend,
) -> Result<String, EmitError> {
    let subject = format!("operations.{}", operation_subject(operation));
    let stream_descriptor = match operation.stream.as_ref().map(|stream| &stream.kind) {
        Some(StreamKind::Sse { events }) => {
            let events = events
                .iter()
                .map(|event| {
                    let validate = validator(Some(&event.data), validation, &subject)?;
                    Ok(format!(
                        "{{ event: {}, schema: {}{validate} }}",
                        string(&event.event),
                        type_ref_json(&event.data, options)?
                    ))
                })
                .collect::<Result<Vec<_>, EmitError>>()?
                .join(", ");
            Some(format!("stream: \"sse\", events: [{events}]"))
        }
        Some(StreamKind::Ndjson { .. }) => Some("stream: \"ndjson\"".into()),
        _ => None,
    };
    operation
        .responses
        .iter()
        .map(|response| {
            let item = match (response.content, operation.stream.as_ref()) {
                (ContentKind::Stream, Some(stream)) => match &stream.kind {
                    StreamKind::Ndjson { item } => Some(item),
                    _ => None,
                },
                _ => None,
            };
            let validate = match item {
                Some(item) => validator(Some(item), validation, &subject)?,
                None if response.content == ContentKind::Stream => String::new(),
                None => validator(response.body.as_ref(), validation, &subject)?,
            };
            let schema = match item {
                Some(item) => type_ref_json(item, options)?,
                None => match &response.body {
                    Some(body) => type_ref_json(body, options),
                    None => Ok("undefined".into()),
                }?,
            };
            let stream = if response.content == ContentKind::Stream {
                stream_descriptor
                    .as_ref()
                    .map_or(String::new(), |stream| format!(", {stream}"))
            } else {
                String::new()
            };
            let media_type = response
                .media
                .as_ref()
                .map(|media| format!(", mediaType: {}", string(&media.to_string())))
                .unwrap_or_default();
            let headers = response
                .headers
                .iter()
                .map(|header| header_descriptor(header, options, validation, &subject))
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
                "{{ status: {status}, content: {}, schema: {schema}{validate}{media_type}{stream}{headers} }}",
                string(content_name(response.content))
            ))
        })
        .collect::<Result<Vec<_>, EmitError>>()
        .map(|descriptors| descriptors.join(", "))
}

fn header_descriptor(
    header: &ResponseHeader,
    options: &TsOptions,
    validation: &mut validation::Backend,
    subject: &str,
) -> Result<String, EmitError> {
    let validate = validator(Some(&header.ty), validation, subject)?;
    Ok(format!(
        "{{ name: {}, required: {}, schema: {}{validate} }}",
        string(&header.wire),
        header.presence == Presence::Required,
        type_ref_json(&header.ty, options)?
    ))
}

fn validator(
    body: Option<&TypeRef>,
    validation: &mut validation::Backend,
    subject: &str,
) -> Result<String, EmitError> {
    let Some(body) = body else {
        return Ok(String::new());
    };
    let Some(expression) = validation.validate(body, subject)? else {
        return Ok(String::new());
    };
    Ok(format!(", validate: {expression}"))
}

fn error_descriptor(
    error: &ErrorResponse,
    options: &TsOptions,
    validation: &mut validation::Backend,
    subject: &str,
) -> Result<String, EmitError> {
    let validate = validator(error.body.as_ref(), validation, subject)?;
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
        .map(|ty| type_ref_json(ty, options))
        .transpose()?
        .unwrap_or_else(|| "undefined".into());
    let headers = error
        .headers
        .iter()
        .map(|header| header_descriptor(header, options, validation, subject))
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
        .media
        .as_ref()
        .map(|media| format!(", mediaType: {}", string(&media.to_string())))
        .unwrap_or_default();
    Ok(format!(
        "{{ status: {status}, content: {}, schema: {schema}{validate}{media_type}{retryable}{headers} }}",
        string(content)
    ))
}

pub(crate) fn type_ref_json(reference: &TypeRef, options: &TsOptions) -> Result<String, EmitError> {
    let mut value = serde_json::to_value(reference)
        .map_err(|error| EmitError(format!("cannot encode type reference: {error}")))?;
    normalize_integer_constraints(&mut value);
    tag_carriers(&mut value, options);
    serde_json::to_string(&value)
        .map_err(|error| EmitError(format!("cannot encode type reference: {error}")))
}

pub(crate) fn tag_carriers(value: &mut serde_json::Value, options: &TsOptions) {
    match value {
        serde_json::Value::Object(fields) => {
            if fields.get("kind").and_then(serde_json::Value::as_str) == Some("int") {
                let carrier =
                    if fields.get("wire").and_then(serde_json::Value::as_str) == Some("string") {
                        "string"
                    } else if fields
                        .get("bits")
                        .and_then(serde_json::Value::as_u64)
                        .is_some_and(|bits| bits > 53)
                    {
                        match options.int64 {
                            Int64::String => "string",
                            Int64::Bigint if options.mode == EmitMode::Strict => "bigint",
                            Int64::Bigint | Int64::Number => "number",
                        }
                    } else {
                        "number"
                    };
                fields.insert("carrier".into(), serde_json::Value::from(carrier));
            }
            if options.date_mode == DateMode::Date
                && fields.get("kind").and_then(serde_json::Value::as_str) == Some("string")
                && fields.get("format").and_then(serde_json::Value::as_str) == Some("date-time")
            {
                fields.insert("carrier".into(), serde_json::Value::from("date"));
            }
            for child in fields.values_mut() {
                tag_carriers(child, options);
            }
        }
        serde_json::Value::Array(values) => {
            for child in values {
                tag_carriers(child, options);
            }
        }
        _ => {}
    }
}

pub(crate) fn normalize_integer_constraints(value: &mut serde_json::Value) {
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

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use serde_json::{Value, json};

    fn operations_of(operations: Value) -> String {
        let raw = json!({
            "ir_version": pons_ir::IR_VERSION,
            "info": {"title": "Verbs", "version": "1"},
            "operations": operations,
        });
        let document: pons_ir::Document = serde_json::from_value(raw).unwrap();
        crate::emit(&document)
            .unwrap()
            .into_iter()
            .filter(|file| file.path.starts_with("operations/"))
            .map(|file| file.content)
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn every_operation_sends_the_verb_the_document_names_and_not_the_variant() {
        let methods = ["GET", "POST", "PUT", "PATCH", "DELETE", "HEAD", "OPTIONS"];
        let operations = methods
            .iter()
            .map(|method| {
                let name = method.to_lowercase();
                json!({
                    "id": [["verbs"], [name]],
                    "method": method,
                    "path": [{"lit": name}],
                    "auth": {"alternatives": [], "optional": false},
                    "semantics": {"safe": false, "idempotent": false},
                    "responses": [{"status": 200, "content": "empty"}],
                })
            })
            .collect::<Vec<_>>();
        let client = operations_of(Value::Array(operations));
        let sent = client
            .lines()
            .filter_map(|line| {
                let after = line.split_once("configured, { method: ")?.1;
                let verb = after.split(',').next().unwrap().trim();
                let path = line
                    .split("encodePathSegment(\"")
                    .nth(1)?
                    .split('"')
                    .next()
                    .unwrap();
                Some((path.to_owned(), verb.to_owned()))
            })
            .collect::<BTreeMap<_, _>>();
        assert_eq!(sent.len(), methods.len(), "{client}");
        for method in methods {
            let name = method.to_lowercase();
            assert_eq!(sent[&name], format!("\"{method}\""), "{client}");
        }
    }
}
