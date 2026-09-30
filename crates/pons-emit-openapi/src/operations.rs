use std::collections::BTreeMap;

use pons_ir::{
    ContentKind, ErrorContent, ErrorStatus, Method, Operation, PathSegment, Presence,
    ResponseHeader, ResponseStatus, TypeRef, is_valid_media_type,
};
use serde_json::{Map, Value, json};

use crate::{Builder, EmitError};

impl Builder<'_> {
    pub(super) fn paths(&self) -> Result<Value, EmitError> {
        let mut paths = BTreeMap::<String, Map<String, Value>>::new();
        let mut operation_ids = std::collections::BTreeSet::new();
        let mut routes = BTreeMap::<String, String>::new();
        for operation in &self.document.operations {
            let path = openapi_path(operation)?;
            let method = method_name(operation.method);
            let id = operation_id(operation);
            if !operation_ids.insert(id.clone()) {
                return Err(EmitError(format!("duplicate OpenAPI operationId {id}")));
            }
            let route = route_pattern(operation);
            if let Some(existing) = routes.get(&route) {
                if existing != &path {
                    return Err(EmitError(format!(
                        "OpenAPI route pattern {route} uses conflicting parameter names"
                    )));
                }
            } else {
                routes.insert(route, path.clone());
            }
            let item = paths.entry(path.clone()).or_default();
            if item.contains_key(method) {
                return Err(EmitError(format!(
                    "multiple operations use {method} {path}"
                )));
            }
            item.insert(method.into(), self.operation(operation)?);
        }
        Ok(Value::Object(
            paths
                .into_iter()
                .map(|(path, item)| (path, Value::Object(item)))
                .collect(),
        ))
    }

    fn operation(&self, operation: &Operation) -> Result<Value, EmitError> {
        let mut value = Map::new();
        value.insert("operationId".into(), json!(operation_id(operation)));
        let mut parameters = self.parameters(operation)?;
        if !parameters.is_empty() {
            value.insert(
                "parameters".into(),
                Value::Array(std::mem::take(&mut parameters)),
            );
        }
        if let Some(body) = &operation.body {
            value.insert("requestBody".into(), self.request_body(body)?);
        }
        value.insert("responses".into(), self.responses(operation)?);
        if let Some(doc) = &operation.doc {
            if let Some(summary) = &doc.summary {
                value.insert("summary".into(), json!(summary));
            }
            if let Some(description) = &doc.description {
                value.insert("description".into(), json!(description));
            }
            if let Some(deprecated) = doc.deprecated {
                value.insert("deprecated".into(), json!(deprecated));
            }
            if !doc.tags.is_empty() {
                value.insert("tags".into(), json!(doc.tags));
            }
        }
        value.insert(
            "security".into(),
            Value::Array(self.operation_security(operation)?),
        );
        let metadata = json!({
            "id": operation.id,
            "auth": operation.auth,
            "semantics": operation.semantics,
            "provides": operation.provides,
            "errors_inherit": operation.errors_inherit,
            "stream": operation.stream,
            "origin": operation.origin,
            "ext": operation.ext,
            "diagnostics": operation.diagnostics,
        });
        value.insert("x-pons".into(), metadata);
        Ok(Value::Object(value))
    }

    fn parameters(&self, operation: &Operation) -> Result<Vec<Value>, EmitError> {
        let mut parameters = Vec::new();
        for segment in &operation.path {
            if let PathSegment::Param { param, ty } = segment {
                parameters.push(json!({"name":param,"in":"path","required":true,"schema":self.type_schema(ty)?}));
            }
        }
        if let Some(query) = &operation.query {
            self.query_parameters(query, &mut parameters)?;
        }
        for header in &operation.headers {
            if is_ignored_request_header(&header.wire) {
                return Err(EmitError(format!(
                    "OpenAPI ignores request header {}",
                    header.wire
                )));
            }
            parameters.push(json!({
                "name": header.wire,
                "in": "header",
                "required": header.presence == Presence::Required,
                "schema": self.type_schema(&header.ty)?,
                "x-pons":{"ident":header.ident,"presence":header.presence,"ext":header.ext},
            }));
        }
        let mut seen = std::collections::BTreeSet::new();
        for parameter in &parameters {
            let location = parameter["in"].as_str().unwrap_or_default();
            let name = parameter["name"].as_str().unwrap_or_default();
            let key = if location == "header" {
                name.to_ascii_lowercase()
            } else {
                name.to_owned()
            };
            if !seen.insert((location.to_owned(), key)) {
                return Err(EmitError(format!("duplicate {location} parameter {name}")));
            }
        }
        Ok(parameters)
    }

    fn query_parameters(
        &self,
        query: &TypeRef,
        parameters: &mut Vec<Value>,
    ) -> Result<(), EmitError> {
        for field in pons_ir::query_fields(self.document, query)
            .map_err(|rejection| EmitError(format!("OpenAPI {}", rejection.message())))?
        {
            let mut parameter = json!({
                "name": field.wire,
                "in": "query",
                "required": field.presence == Presence::Required,
                "schema": self.field_schema(field)?,
            });
            add_parameter_doc(&mut parameter, field.doc.as_ref());
            parameters.push(parameter);
        }
        Ok(())
    }

    fn request_body(&self, body: &pons_ir::Body) -> Result<Value, EmitError> {
        if body.content == ContentKind::Stream {
            return Err(EmitError(
                "OpenAPI cannot describe streaming request bodies".into(),
            ));
        }
        let media = media_type(body.content, None, body.media_type.as_deref())?
            .ok_or_else(|| EmitError("request body cannot use empty or stream content".into()))?;
        let schema = if body.content == ContentKind::Bytes {
            json!({})
        } else {
            self.type_schema(&body.ty)?
        };
        Ok(json!({
            "required":body.required,
            "content":media_content(media, schema),
            "x-pons":{"ty":body.ty,"required":body.required,"content":body.content,"media_type":body.media_type,"ext":body.ext}
        }))
    }
    fn responses(&self, operation: &Operation) -> Result<Value, EmitError> {
        let mut responses = Map::new();
        for response in &operation.responses {
            let media = media_type(
                response.content,
                operation.stream.as_ref(),
                response.media_type.as_deref(),
            )?;
            let mut value = json!({"description":"Response"});
            match response.content {
                ContentKind::Empty => {}
                ContentKind::Stream => {
                    let media = media.ok_or_else(|| {
                        EmitError("stream response is missing its stream description".into())
                    })?;
                    value["content"] = media_content(media, self.stream_schema(operation)?);
                }
                ContentKind::Bytes => {
                    let media = media.ok_or_else(|| {
                        EmitError("byte response is missing its media type".into())
                    })?;
                    if response.body.is_none() {
                        return Err(EmitError("byte response is missing its body type".into()));
                    }
                    value["content"] = media_content(media, json!({}));
                }
                _ => {
                    let body = response.body.as_ref().ok_or_else(|| {
                        EmitError(format!(
                            "response {} content {:?} is missing its body",
                            response.status, response.content
                        ))
                    })?;
                    let media = media.ok_or_else(|| {
                        EmitError(format!(
                            "response {} content {:?} has no media type",
                            response.status, response.content
                        ))
                    })?;
                    value["content"] = media_content(media, self.type_schema(body)?);
                }
            }
            add_response_headers(self, &response.headers, &mut value, "response")?;
            value["x-pons"] = json!({"status":response.status,"body":response.body,"content":response.content,"media_type":response.media_type,"ext":response.ext});
            insert_response(&mut responses, success_status(response.status), value)?;
        }
        let errors = if operation.errors_inherit {
            operation
                .errors
                .iter()
                .chain(&self.document.common_errors)
                .collect::<Vec<_>>()
        } else {
            operation.errors.iter().collect::<Vec<_>>()
        };
        for error in errors {
            let status = error_status(&error.status);
            let description = error
                .doc
                .as_ref()
                .and_then(|doc| doc.description.as_deref().or(doc.summary.as_deref()))
                .unwrap_or("Error response");
            let mut value = json!({"description":description});
            if error.content != ErrorContent::Empty {
                let body = error.body.as_ref().ok_or_else(|| {
                    EmitError(format!("error {:?} is missing its body", error.name))
                })?;
                let media = error
                    .media_type
                    .clone()
                    .unwrap_or_else(|| error_media_type(error.content).into());
                value["content"] = media_content(media, self.type_schema(body)?);
            }
            add_response_headers(self, &error.headers, &mut value, "error response")?;
            if let Some(retryable) = error.retryable {
                value["x-pons-retryable"] = json!(retryable);
            }
            value["x-pons"] = json!({"name":error.name,"body":error.body,"headers":error.headers,"content":error.content,"media_type":error.media_type,"doc":error.doc,"ext":error.ext});
            insert_response(&mut responses, status, value)?;
        }
        if responses.is_empty() {
            responses.insert(
                "default".into(),
                json!({"description":"Unspecified response"}),
            );
        }
        Ok(Value::Object(responses))
    }

    fn stream_schema(&self, operation: &Operation) -> Result<Value, EmitError> {
        let Some(stream) = &operation.stream else {
            return Err(EmitError(
                "stream response is missing its stream description".into(),
            ));
        };
        let mut schema = match &stream.kind {
            pons_ir::StreamKind::Sse { events } => {
                let events = events
                    .iter()
                    .map(|event| {
                        Ok(json!({"event":event.event,"schema":self.type_schema(&event.data)?}))
                    })
                    .collect::<Result<Vec<_>, EmitError>>()?;
                json!({"type":"string","x-pons-events":events})
            }
            pons_ir::StreamKind::Ndjson { item } => self.type_schema(item)?,
            pons_ir::StreamKind::Bytes { .. } => json!({}),
            pons_ir::StreamKind::Websocket { .. } => {
                return Err(EmitError(
                    "OpenAPI paths cannot describe WebSocket operations".into(),
                ));
            }
        };
        schema["x-pons-stream"] = json!(stream.kind);
        Ok(schema)
    }
}

fn openapi_path(operation: &Operation) -> Result<String, EmitError> {
    let mut segments = Vec::with_capacity(operation.path.len());
    for segment in &operation.path {
        match segment {
            PathSegment::Literal { lit } if lit.contains('{') || lit.contains('}') => {
                return Err(EmitError(format!(
                    "path literal {lit:?} contains OpenAPI template syntax"
                )));
            }
            PathSegment::Literal { lit } if is_dot_segment(lit) => {
                return Err(EmitError(format!(
                    "path literal {lit:?} is a dot segment the TypeScript client refuses to encode"
                )));
            }
            PathSegment::Literal { lit } => segments.push(escape_path_literal(lit)),
            PathSegment::Param { param, .. } if !valid_path_parameter_name(param) => {
                return Err(EmitError(format!(
                    "path parameter {param:?} is not a valid OpenAPI path parameter name"
                )));
            }
            PathSegment::Param { param, .. } => segments.push(format!("{{{param}}}")),
        }
    }
    if segments.is_empty() {
        Ok("/".into())
    } else {
        Ok(format!("/{}", segments.join("/")))
    }
}

fn route_pattern(operation: &Operation) -> String {
    if operation.path.is_empty() {
        return "/".into();
    }
    format!(
        "/{}",
        operation
            .path
            .iter()
            .map(|segment| match segment {
                PathSegment::Literal { lit } => escape_path_literal(lit),
                PathSegment::Param { .. } => "{}".into(),
            })
            .collect::<Vec<_>>()
            .join("/")
    )
}

fn escape_path_literal(literal: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut escaped = String::with_capacity(literal.len());
    for byte in literal.bytes() {
        if byte.is_ascii_alphanumeric()
            || matches!(
                byte,
                b'-' | b'_' | b'.' | b'!' | b'~' | b'*' | b'\'' | b'(' | b')'
            )
        {
            escaped.push(char::from(byte));
        } else {
            escaped.push('%');
            escaped.push(char::from(HEX[usize::from(byte >> 4)]));
            escaped.push(char::from(HEX[usize::from(byte & 0x0F)]));
        }
    }
    escaped
}

fn is_dot_segment(literal: &str) -> bool {
    matches!(literal, "." | "..")
}

fn valid_path_parameter_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~'))
}

fn method_name(method: Method) -> &'static str {
    match method {
        Method::Get => "get",
        Method::Post => "post",
        Method::Put => "put",
        Method::Patch => "patch",
        Method::Delete => "delete",
        Method::Head => "head",
        Method::Options => "options",
    }
}

fn add_parameter_doc(parameter: &mut Value, doc: Option<&pons_ir::Doc>) {
    let Some(doc) = doc else { return };
    if let Some(description) = doc.description.as_deref().or(doc.summary.as_deref()) {
        parameter["description"] = json!(description);
    }
    if let Some(deprecated) = doc.deprecated {
        parameter["deprecated"] = json!(deprecated);
    }
}

fn is_ignored_request_header(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "accept" | "authorization" | "content-type" | "cookie"
    )
}

fn media_type(
    content: ContentKind,
    stream: Option<&pons_ir::StreamDescription>,
    override_type: Option<&str>,
) -> Result<Option<String>, EmitError> {
    if let Some(pons_ir::StreamKind::Bytes { content_type }) = stream.map(|stream| &stream.kind)
        && !is_valid_media_type(content_type)
    {
        return Err(EmitError(format!(
            "invalid byte stream content type {content_type:?}"
        )));
    }
    if content == ContentKind::Stream
        && let Some(value) = override_type
        && let Some(expected) = stream.and_then(|stream| stream.kind.media_type())
        && !pons_ir::media_type_matches(value, expected)
    {
        return Err(EmitError(format!(
            "response media type {value:?} does not match stream kind media type {expected:?}"
        )));
    }
    if let Some(value) = override_type {
        if !is_valid_media_type(value) {
            return Err(EmitError(format!("invalid media type {value:?}")));
        }
        return Ok(Some(value.into()));
    }
    Ok(Some(match content {
        ContentKind::Json => "application/json".into(),
        ContentKind::ProblemJson => "application/problem+json".into(),
        ContentKind::Form => "application/x-www-form-urlencoded".into(),
        ContentKind::Multipart => "multipart/form-data".into(),
        ContentKind::Text => "text/plain".into(),
        ContentKind::Bytes => "application/octet-stream".into(),
        ContentKind::Stream => {
            let Some(kind) = stream.map(|stream| &stream.kind) else {
                return Err(EmitError(
                    "stream response is missing its stream description".into(),
                ));
            };
            let Some(content_type) = kind.media_type() else {
                return Err(EmitError(
                    "OpenAPI paths cannot describe WebSocket operations".into(),
                ));
            };
            content_type.into()
        }
        ContentKind::Empty => return Ok(None),
    }))
}

fn error_media_type(content: ErrorContent) -> &'static str {
    match content {
        ErrorContent::Json => "application/json",
        ErrorContent::ProblemJson => "application/problem+json",
        ErrorContent::Text => "text/plain",
        ErrorContent::Empty => unreachable!(),
    }
}

fn media_content(media: String, schema: Value) -> Value {
    let mut content = Map::new();
    content.insert(media, json!({"schema":schema}));
    Value::Object(content)
}

fn operation_id(operation: &Operation) -> String {
    operation
        .id
        .iter()
        .map(|part| part.join("_"))
        .collect::<Vec<_>>()
        .join(".")
}

fn add_response_headers(
    builder: &Builder<'_>,
    headers: &[ResponseHeader],
    value: &mut Value,
    kind: &str,
) -> Result<(), EmitError> {
    if headers.is_empty() {
        return Ok(());
    }
    let mut response_headers = Map::new();
    for header in headers {
        if header.wire.eq_ignore_ascii_case("content-type") {
            return Err(EmitError(format!(
                "{kind} header {} is defined by the response content",
                header.wire
            )));
        }
        response_headers.insert(
            header.wire.clone(),
            json!({
                "required": header.presence == Presence::Required,
                "schema": builder.type_schema(&header.ty)?,
                "x-pons": {"presence": header.presence, "ext": header.ext}
            }),
        );
    }
    value["headers"] = Value::Object(response_headers);
    Ok(())
}

fn success_status(status: ResponseStatus) -> String {
    match status {
        ResponseStatus::Exact(status) => status.to_string(),
        ResponseStatus::Class(class) => format!("{class}XX"),
        ResponseStatus::Default | ResponseStatus::Unknown => "default".into(),
    }
}

fn error_status(status: &ErrorStatus) -> String {
    match status {
        ErrorStatus::Exact(status) => status.to_string(),
        ErrorStatus::Class(class) => format!("{class}XX"),
        ErrorStatus::Default => "default".into(),
    }
}

fn insert_response(
    responses: &mut Map<String, Value>,
    status: String,
    value: Value,
) -> Result<(), EmitError> {
    if responses.insert(status.clone(), value).is_some() {
        return Err(EmitError(format!(
            "multiple response definitions use status {status}"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use pons_ir::PathSegment;
    use serde_json::Value;

    fn fixture() -> pons_ir::Document {
        serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap()
    }

    #[test]
    fn rejects_dot_segment_path_literals() {
        for lit in [".", ".."] {
            let mut document = fixture();
            document.operations[0].path = vec![PathSegment::Literal { lit: lit.into() }];
            let error = crate::emit(&document).unwrap_err().0;
            assert!(error.contains("dot segment"), "{error}");
        }
    }

    #[test]
    fn keeps_literals_that_only_look_like_dot_segments() {
        let mut document = fixture();
        document.operations[0].path = vec![PathSegment::Literal { lit: "..a".into() }];
        let output: Value = serde_json::from_str(&crate::emit(&document).unwrap()).unwrap();
        assert!(
            output["paths"].as_object().unwrap().contains_key("/..a"),
            "{:#}",
            output["paths"]
        );
    }
}
