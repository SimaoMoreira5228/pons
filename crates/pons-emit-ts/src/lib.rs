mod client;
mod runtime;
mod runtime_auth;
mod types;

use std::collections::BTreeMap;

use pons_ir::{
    Document, InlineType, IntegerWire, ObjectOpenness, PathSegment, StreamKind, TypeDef, TypeKind,
    TypeRef, VariantPayload,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeneratedFile {
    pub path: String,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmitError(pub String);

impl std::fmt::Display for EmitError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for EmitError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmitMode {
    Strict,
    Lossy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorStrategy {
    Neverthrow,
    Throw,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmitOutput {
    pub files: Vec<GeneratedFile>,
    pub warnings: Vec<String>,
}

pub fn emit(document: &Document) -> Result<Vec<GeneratedFile>, EmitError> {
    emit_with_mode(document, EmitMode::Strict).map(|output| output.files)
}

pub fn emit_with_mode(document: &Document, mode: EmitMode) -> Result<EmitOutput, EmitError> {
    emit_with_strategy(document, mode, ErrorStrategy::Neverthrow)
}

pub fn emit_with_strategy(
    document: &Document,
    mode: EmitMode,
    error_strategy: ErrorStrategy,
) -> Result<EmitOutput, EmitError> {
    let mut document = document.clone();
    document.canonicalize();
    document.validate().map_err(|errors| {
        EmitError(
            errors
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join("\n"),
        )
    })?;
    if document.operations.iter().any(|operation| {
        operation.responses.iter().any(|response| {
            matches!(
                response.content,
                pons_ir::ContentKind::Form | pons_ir::ContentKind::Multipart
            )
        })
    }) {
        return Err(EmitError(
            "the TypeScript emitter does not support form or multipart responses".into(),
        ));
    }
    if document.operations.iter().any(|operation| {
        operation.stream.as_ref().is_some_and(|stream| {
            !matches!(
                &stream.kind,
                StreamKind::Sse { .. } | StreamKind::Ndjson { .. }
            )
        }) || operation.stream.is_none()
            && operation
                .responses
                .iter()
                .any(|response| response.content == pons_ir::ContentKind::Stream)
    }) {
        return Err(EmitError(
            "the TypeScript emitter supports SSE and NDJSON streams only".into(),
        ));
    }
    let wide_integers = wide_integer_references(&document);
    let names = type_names(&document);
    let preserve_wide_integers = mode == EmitMode::Strict && !wide_integers.is_empty();
    let files = vec![
        GeneratedFile {
            path: "types.ts".into(),
            content: types::emit(&document, &names, mode)?,
        },
        GeneratedFile {
            path: "client.ts".into(),
            content: client::emit(
                &document,
                &names,
                mode,
                preserve_wide_integers,
                error_strategy,
            )?,
        },
    ];
    let warnings = (mode == EmitMode::Lossy && !wide_integers.is_empty())
        .then(|| {
            format!(
                "TypeScript emits wide numeric integers as number at {}",
                wide_integers.join(", ")
            )
        })
        .into_iter()
        .collect();
    Ok(EmitOutput { files, warnings })
}

fn wide_integer_references(document: &Document) -> Vec<String> {
    let mut paths = Vec::new();
    for (id, definition) in &document.types {
        scan_definition(&format!("types.{id}"), definition, &mut paths);
    }
    for (index, error) in document.common_errors.iter().enumerate() {
        if let Some(body) = &error.body {
            scan_type_ref(body, &format!("common_errors[{index}].body"), &mut paths);
        }
    }
    for (index, operation) in document.operations.iter().enumerate() {
        let prefix = format!("operations[{index}]");
        for (segment_index, segment) in operation.path.iter().enumerate() {
            if let PathSegment::Param { ty, .. } = segment {
                scan_type_ref(
                    ty,
                    &format!("{prefix}.path[{segment_index}].ty"),
                    &mut paths,
                );
            }
        }
        if let Some(query) = &operation.query {
            scan_type_ref(query, &format!("{prefix}.query"), &mut paths);
        }
        for (header_index, header) in operation.headers.iter().enumerate() {
            scan_type_ref(
                &header.ty,
                &format!("{prefix}.headers[{header_index}].ty"),
                &mut paths,
            );
        }
        if let Some(body) = &operation.body {
            scan_type_ref(&body.ty, &format!("{prefix}.body.ty"), &mut paths);
        }
        for (response_index, response) in operation.responses.iter().enumerate() {
            if let Some(body) = &response.body {
                scan_type_ref(
                    body,
                    &format!("{prefix}.responses[{response_index}].body"),
                    &mut paths,
                );
            }
            for (header_index, header) in response.headers.iter().enumerate() {
                scan_type_ref(
                    &header.ty,
                    &format!("{prefix}.responses[{response_index}].headers[{header_index}].ty"),
                    &mut paths,
                );
            }
        }
        for (error_index, error) in operation.errors.iter().enumerate() {
            if let Some(body) = &error.body {
                scan_type_ref(
                    body,
                    &format!("{prefix}.errors[{error_index}].body"),
                    &mut paths,
                );
            }
        }
        if let Some(stream) = &operation.stream {
            match &stream.kind {
                StreamKind::Sse { events } => {
                    for (event_index, event) in events.iter().enumerate() {
                        scan_type_ref(
                            &event.data,
                            &format!("{prefix}.stream.events[{event_index}].data"),
                            &mut paths,
                        );
                    }
                }
                StreamKind::Ndjson { item } => {
                    scan_type_ref(item, &format!("{prefix}.stream.item"), &mut paths);
                }
                StreamKind::Websocket { client, server } => {
                    scan_type_ref(client, &format!("{prefix}.stream.client"), &mut paths);
                    scan_type_ref(server, &format!("{prefix}.stream.server"), &mut paths);
                }
                StreamKind::Bytes { .. } => {}
            }
        }
    }
    paths
}

fn scan_definition(path: &str, definition: &TypeDef, paths: &mut Vec<String>) {
    match &definition.definition {
        TypeKind::Struct {
            fields,
            flattened,
            openness,
        } => {
            for (index, field) in fields.iter().enumerate() {
                scan_type_ref(&field.ty, &format!("{path}.fields[{index}].ty"), paths);
            }
            for (index, reference) in flattened.iter().enumerate() {
                scan_type_ref(reference, &format!("{path}.flattened[{index}]"), paths);
            }
            if let ObjectOpenness::PreserveTyped { value } = openness {
                scan_type_ref(value, &format!("{path}.openness.value"), paths);
            }
        }
        TypeKind::Enum { variants, .. } => {
            for (variant_index, variant) in variants.iter().enumerate() {
                scan_payload(
                    &variant.payload,
                    &format!("{path}.variants[{variant_index}].payload"),
                    paths,
                );
            }
        }
        TypeKind::Newtype { inner } => scan_type_ref(inner, &format!("{path}.inner"), paths),
    }
}

fn scan_payload(payload: &VariantPayload, path: &str, paths: &mut Vec<String>) {
    match payload {
        VariantPayload::Unit => {}
        VariantPayload::Newtype { ty } => scan_type_ref(ty, &format!("{path}.ty"), paths),
        VariantPayload::Tuple { items } => {
            for (index, item) in items.iter().enumerate() {
                scan_type_ref(item, &format!("{path}.items[{index}]"), paths);
            }
        }
        VariantPayload::Struct { fields } => {
            for (index, field) in fields.iter().enumerate() {
                scan_type_ref(&field.ty, &format!("{path}.fields[{index}].ty"), paths);
            }
        }
    }
}

fn scan_type_ref(reference: &TypeRef, path: &str, paths: &mut Vec<String>) {
    let TypeRef::Inline(inline) = reference else {
        return;
    };
    match inline {
        InlineType::Int { bits, wire, .. } if *wire == IntegerWire::Number && *bits > 53 => {
            paths.push(path.to_owned())
        }
        InlineType::Nullable { inner } => scan_type_ref(inner, &format!("{path}.inner"), paths),
        InlineType::Array { item, .. } => scan_type_ref(item, &format!("{path}.item"), paths),
        InlineType::Map { key, value } => {
            scan_type_ref(key, &format!("{path}.key"), paths);
            scan_type_ref(value, &format!("{path}.value"), paths);
        }
        InlineType::Tuple { items } => {
            for (index, item) in items.iter().enumerate() {
                scan_type_ref(item, &format!("{path}.items[{index}]"), paths);
            }
        }
        _ => {}
    }
}

fn type_names(document: &Document) -> BTreeMap<String, String> {
    let mut result = BTreeMap::new();
    let mut used = BTreeMap::<String, usize>::new();
    for (id, definition) in &document.types {
        let base = pascal(&definition.name);
        let count = used.entry(base.clone()).or_default();
        *count += 1;
        let name = if *count == 1 {
            base
        } else {
            format!("{base}{count}")
        };
        result.insert(id.clone(), name);
    }
    result
}

fn pascal(words: &[String]) -> String {
    let value = words
        .iter()
        .map(|word| {
            let mut chars = word.chars();
            chars
                .next()
                .map(|first| first.to_uppercase().chain(chars).collect::<String>())
                .unwrap_or_default()
        })
        .collect::<String>();
    identifier(&value, "Type")
}

fn camel(words: &[String]) -> String {
    let Some((first, rest)) = words.split_first() else {
        return "operation".into();
    };
    let mut result = first.to_lowercase();
    for word in rest {
        let mut chars = word.chars();
        if let Some(first) = chars.next() {
            result.extend(first.to_uppercase());
            result.extend(chars);
        }
    }
    identifier(&result, "operation")
}

fn identifier(value: &str, fallback: &str) -> String {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return fallback.into();
    };
    let mut result = String::new();
    if first.is_ascii_alphabetic() || first == '_' || first == '$' {
        result.push(first);
    } else {
        result.push('_');
        result.push(first);
    }
    result.extend(chars.map(|character| {
        if character.is_ascii_alphanumeric() || character == '_' || character == '$' {
            character
        } else {
            '_'
        }
    }));
    if is_keyword(&result) {
        format!("{result}_")
    } else {
        result
    }
}

fn is_keyword(value: &str) -> bool {
    matches!(
        value,
        "break"
            | "case"
            | "catch"
            | "class"
            | "const"
            | "continue"
            | "debugger"
            | "default"
            | "delete"
            | "do"
            | "else"
            | "enum"
            | "export"
            | "extends"
            | "false"
            | "finally"
            | "for"
            | "function"
            | "if"
            | "import"
            | "in"
            | "instanceof"
            | "new"
            | "null"
            | "return"
            | "super"
            | "switch"
            | "this"
            | "throw"
            | "true"
            | "try"
            | "typeof"
            | "var"
            | "void"
            | "while"
            | "with"
            | "as"
            | "implements"
            | "interface"
            | "let"
            | "package"
            | "private"
            | "protected"
            | "public"
            | "static"
            | "yield"
            | "any"
            | "boolean"
            | "number"
            | "string"
            | "symbol"
            | "type"
            | "from"
            | "of"
    )
}

pub(crate) fn string(value: &str) -> String {
    serde_json::to_string(value).expect("serializing a string cannot fail")
}

#[cfg(test)]
mod tests {
    use pons_ir::{
        ContentKind, Document, IR_VERSION, InlineType, IntegerWire, ObjectOpenness, ResponseStatus,
        StreamKind, TypeKind, TypeRef,
    };
    use std::collections::BTreeMap;

    #[test]
    fn emits_a_client_for_the_contract_fixture() {
        let mut document: Document =
            serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
        document.operations[0].provides.as_mut().unwrap().expires_at =
            Some(pons_ir::CredentialLocator::Body {
                pointer: "/expires_at".into(),
            });
        let files = super::emit(&document).unwrap();
        let files = files
            .into_iter()
            .map(|file| (file.path, file.content))
            .collect::<BTreeMap<_, _>>();
        let types = &files["types.ts"];
        let client = &files["client.ts"];
        assert!(types.contains("export type User"));
        assert!(types.contains("export type LoginError"));
        assert!(client.contains("export function createClient"));
        assert!(client.contains("auth"));
        assert!(client.contains("neverthrow"));
        assert!(client.contains("kind: \"api_key\", location: \"header\""));
        assert!(client.contains("AuthAttachment"));
        assert!(client.contains("authAdapters?: Record<string, AuthAdapter>"));
        assert!(client.contains("kind: \"auth_adapter\""));
        assert!(client.contains("attachment.headers"));
        assert!(client.contains("signal?: AbortSignal"));
        assert!(client.contains("timeout?: number"));
        assert!(client.contains("export type CallOptions"));
        assert!(client.contains("headers?: HeadersInit"));
        assert!(client.contains("callOptions?.headers"));
        assert!(client.contains("callOptions?.timeout ?? client.timeout"));
        assert!(client.contains("callOptions?.signal ?? client.signal"));
        assert!(client.contains("options?: CallOptions"));
        assert!(client.contains("export type RequestInterceptor"));
        assert!(client.contains("export type ResponseInterceptor"));
        assert!(client.contains("headers?: HeadersInit |"));
        assert!(client.contains("client.interceptors?.request"));
        assert!(client.contains("client.interceptors?.response"));
        assert!(client.contains("export type RetryOptions"));
        assert!(client.contains("retry?: RetryOptions | false"));
        assert!(client.contains("fetchWithRetry"));
        assert!(client.contains("retry-after"));
        assert!(client.contains("Date.parse(retryAfter)"));
        assert!(client.contains("idempotent: boolean"));
        assert!(client.contains("setTimeout"));
        assert!(client.contains("kind: \"timeout\""));
        assert!(client.contains("kind: \"aborted\""));
        assert!(client.contains("body: Extract<Types.LoginError"));
        assert!(client.contains("headers: Headers"));
        assert!(client.contains("headers: declaredError.headers"));
        assert!(client.contains("ResponseWithHeaders"));
        assert!(client.contains("withResponseHeaders"));
        assert!(client.contains("return { value, headers: responseHeaders"));
        assert!(client.contains("export type TokenStore"));
        assert!(client.contains("export type CredentialSource"));
        assert!(client.contains("kind: \"auth_source\""));
        assert!(client.contains("await source.set(nextCredential, expiresAt)"));
        assert!(client.contains("client.authValues[provides.scheme]"));
        assert!(client.contains("client.authExpirations[provides.scheme]"));
        assert!(client.contains("expiresAt:"));
        assert!(client.contains("Date.parse(value)"));
        assert!(client.contains("source.set(nextCredential, expiresAt)"));
        assert!(client.contains("AsyncIterable<Result<EventsWatchEvent, EventsWatchStreamError>>"));
        assert!(client.contains(
            "ResponseWithHeaders<AsyncIterable<Result<EventsWatchEvent, EventsWatchStreamError>>"
        ));
        assert!(client.contains("event: \"user\""));
        assert!(client.contains("name === \"error\""));
        assert!(client.contains("headers: [{ name: \"X-Trace-ID\", required: true, schema:"));
        assert!(client.contains("missing required response header"));
        assert!(client.contains("expectedContentTypes"));
        assert!(client.contains("application/problem+json"));
        assert!(client.contains("stream === \"ndjson\""));
        assert!(client.contains("\"text/event-stream\""));
        assert!(client.contains("actualContentType !== mediaTypeBase(expectedContentType)"));
        assert!(client.contains("const text = await response.text()"));
        assert!(client.contains("rawValue = parseJson(text, preserveWideIntegers)"));
        assert!(client.contains("invalid JSON response"));
        assert!(!client.contains("responseContent.includes(\"text/event-stream\")"));
        assert!(client.contains("response.headers.has(header.name)"));
        assert!(client.contains("header.required && !response.headers.has(header.name)"));
        assert!(client.contains("responseSchema?.headers"));
        let missing_header_branch = client
            .split("const missingHeader")
            .nth(1)
            .unwrap()
            .split("const responseKind")
            .next()
            .unwrap();
        assert!(missing_header_branch.contains("if (missingHeader)"));
        assert!(
            missing_header_branch.contains("header.required && !response.headers.has(header.name)")
        );
        assert!(missing_header_branch.contains("kind: \"protocol\""));
        assert!(client.contains("undeclared success response"));
        assert!(client.contains("findDeclared"));
        assert!(!client.contains("response.ok"));
        let declared_success = client.find("const declaredSuccess").unwrap();
        let declared_error = client.find("const declaredError").unwrap();
        let undeclared_success = client.find("undeclared success response").unwrap();
        assert!(declared_success < declared_error && declared_error < undeclared_success);
        let stream_capture = client
            .find("captureCredential(client, response, provides, undefined, false)")
            .unwrap();
        let stream_return = client.find("return ok(withSuccessStatus(readSse").unwrap();
        assert!(stream_capture < stream_return);
    }

    #[test]
    fn emits_auth_scopes_for_custom_adapters() {
        let mut document: Document =
            serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
        document.operations[1]
            .auth
            .scopes
            .insert("tenant".into(), vec!["events:read".into()]);
        let output = super::emit(&document).unwrap();
        let client = output
            .into_iter()
            .find(|file| file.path == "client.ts")
            .unwrap();
        assert!(client.content.contains("\"tenant\": [\"events:read\"]"));
        assert!(client.content.contains("scopes: authScopes[scheme]"));
    }

    #[test]
    fn emits_object_openness_types() {
        let mut document: Document =
            serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
        let user = document.types.get_mut("User").unwrap();
        let TypeKind::Struct {
            fields, flattened, ..
        } = &user.definition
        else {
            unreachable!()
        };
        user.definition = TypeKind::Struct {
            fields: fields.clone(),
            flattened: flattened.clone(),
            openness: ObjectOpenness::PreserveOpaque,
        };
        let login = document.types.get_mut("LoginRequest").unwrap();
        let TypeKind::Struct {
            fields, flattened, ..
        } = &login.definition
        else {
            unreachable!()
        };
        login.definition = TypeKind::Struct {
            fields: fields.clone(),
            flattened: flattened.clone(),
            openness: ObjectOpenness::PreserveTyped {
                value: TypeRef::Inline(InlineType::String {
                    format: None,
                    min_length: None,
                    max_length: None,
                    pattern: None,
                }),
            },
        };
        let output = super::emit(&document).unwrap();
        let types = output
            .into_iter()
            .find(|file| file.path == "types.ts")
            .unwrap();
        assert!(types.content.contains("[key: string]: string"));
        assert!(types.content.contains("[key: string]: unknown"));
    }

    #[test]
    fn emits_optional_request_body_arguments() {
        let mut document: Document =
            serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
        document.operations[0].body.as_mut().unwrap().required = false;
        let output = super::emit(&document).unwrap();
        let client = output
            .into_iter()
            .find(|file| file.path == "client.ts")
            .unwrap();
        assert!(client.content.contains("body?:"));
    }

    #[test]
    fn emits_unknown_success_statuses() {
        let mut document: Document =
            serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
        document.operations[0].responses[0].status = ResponseStatus::Unknown;
        let output = super::emit(&document).unwrap();
        let client = output
            .into_iter()
            .find(|file| file.path == "client.ts")
            .unwrap();
        assert!(client.content.contains("ResponseWithStatus<"));
        assert!(client.content.contains("status: \"unknown\""));
        assert!(
            client
                .content
                .contains("const declaredSuccess = findSuccess")
        );
        assert!(
            client
                .content
                .contains("const declaredError = declaredSuccess === undefined")
        );
        assert!(client.content.contains("findDeclared"));

        document.operations[0].responses[0].status = ResponseStatus::Default;
        let output = super::emit(&document).unwrap();
        let client = output
            .into_iter()
            .find(|file| file.path == "client.ts")
            .unwrap();
        assert!(client.content.contains("ResponseWithStatus<"));
    }

    #[test]
    fn rejects_incompatible_typed_object_openness() {
        let mut document: Document =
            serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
        let user = document.types.get_mut("User").unwrap();
        let TypeKind::Struct {
            fields, flattened, ..
        } = &user.definition
        else {
            unreachable!()
        };
        user.definition = TypeKind::Struct {
            fields: fields.clone(),
            flattened: flattened.clone(),
            openness: ObjectOpenness::PreserveTyped {
                value: TypeRef::Inline(InlineType::Bytes {
                    encoding: pons_ir::ByteEncoding::Base64,
                }),
            },
        };
        assert!(
            super::emit(&document)
                .unwrap_err()
                .0
                .contains("typed object openness")
        );
    }

    #[test]
    fn emits_custom_media_types_for_json_and_bytes() {
        let mut document: Document =
            serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
        let login = document
            .operations
            .iter_mut()
            .find(|operation| operation.id[0][0] == "auth")
            .unwrap();
        login.body.as_mut().unwrap().media_type = Some("application/vnd.acme+json".into());
        login.responses[0].media_type = Some("application/vnd.acme.session+json".into());
        let output = super::emit(&document).unwrap();
        let client = output
            .into_iter()
            .find(|file| file.path == "client.ts")
            .unwrap();
        assert!(client.content.contains("\"application/vnd.acme+json\""));
        assert!(
            client
                .content
                .contains("mediaType: \"application/vnd.acme.session+json\"")
        );
    }

    #[test]
    fn emits_throwing_error_strategy() {
        let document: Document =
            serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
        let output = super::emit_with_strategy(
            &document,
            super::EmitMode::Strict,
            super::ErrorStrategy::Throw,
        )
        .unwrap();
        let client = output
            .files
            .iter()
            .find(|file| file.path == "client.ts")
            .unwrap();
        assert!(client.content.contains("requestOrThrow(request<"));
        assert!(!client.content.contains("from \"neverthrow\""));
        assert!(client.content.contains("type Result<T, E>"));
    }

    #[test]
    fn emits_optional_response_headers() {
        let mut document: Document =
            serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
        document.operations[1].responses[0].headers[0].presence = pons_ir::Presence::Optional;
        let files = super::emit(&document).unwrap();
        let client = files.iter().find(|file| file.path == "client.ts").unwrap();
        assert!(
            client
                .content
                .contains("headers: [{ name: \"X-Trace-ID\", required: false, schema:")
        );
    }

    #[test]
    fn emits_declared_error_response_headers() {
        let mut document: Document =
            serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
        document.operations[0].errors[0].retryable = Some(true);
        document.operations[0].errors[0]
            .headers
            .push(pons_ir::ResponseHeader {
                wire: "Retry-After".into(),
                ty: TypeRef::Inline(InlineType::String {
                    format: None,
                    min_length: None,
                    max_length: None,
                    pattern: None,
                }),
                presence: pons_ir::Presence::Required,
                ext: BTreeMap::new(),
            });
        let files = super::emit(&document).unwrap();
        let client = files.iter().find(|file| file.path == "client.ts").unwrap();
        assert!(
            client
                .content
                .contains("headers: { \"Retry-After\": string }")
        );
        assert!(client.content.contains("retryable: true"));
        assert!(
            client
                .content
                .contains("headers: [{ name: \"Retry-After\", required: true, schema:")
        );
    }

    #[test]
    fn strict_mode_uses_bigint_and_lossy_mode_warns() {
        let mut document: Document =
            serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
        let definition = document.types.get_mut("User").unwrap();
        let TypeKind::Struct { fields, .. } = &mut definition.definition else {
            panic!()
        };
        let TypeRef::Inline(InlineType::Int { wire, .. }) = &mut fields[0].ty else {
            panic!()
        };
        *wire = IntegerWire::Number;

        let files = super::emit(&document).unwrap();
        let types = files.iter().find(|file| file.path == "types.ts").unwrap();
        assert!(types.content.contains("\"id\": bigint"));
        let client = files.iter().find(|file| file.path == "client.ts").unwrap();
        assert!(
            client
                .content
                .contains("parseJson(text, preserveWideIntegers)")
        );
        assert!(client.content.contains("stringifyJson(encodedBody)"));

        let output = super::emit_with_mode(&document, super::EmitMode::Lossy).unwrap();
        assert!(
            output
                .warnings
                .iter()
                .any(|warning| warning.contains("wide numeric integers"))
        );
        let types = output
            .files
            .iter()
            .find(|file| file.path == "types.ts")
            .unwrap();
        assert!(types.content.contains("\"id\": number"));
    }

    #[test]
    fn types_sse_error_events_as_stream_failures() {
        let mut document: Document =
            serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
        let operation = document
            .operations
            .iter_mut()
            .find(|operation| operation.stream.is_some())
            .unwrap();
        let StreamKind::Sse { events } = &mut operation.stream.as_mut().unwrap().kind else {
            unreachable!()
        };
        events.push(pons_ir::StreamMessage {
            event: "error".into(),
            data: pons_ir::TypeRef::Inline(pons_ir::InlineType::String {
                format: None,
                min_length: None,
                max_length: None,
                pattern: None,
            }),
        });

        let files = super::emit(&document).unwrap();
        let client = files.iter().find(|file| file.path == "client.ts").unwrap();
        assert!(
            client
                .content
                .contains("kind: \"server_error\"; body: string")
        );
        assert!(!client.content.contains("event: \"error\"; data:"));
    }

    #[test]
    fn emission_is_independent_of_operation_order() {
        let mut document: Document =
            serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
        let expected = super::emit(&document).unwrap();
        document.operations.reverse();
        assert_eq!(super::emit(&document).unwrap(), expected);
    }

    #[test]
    fn rejects_documents_with_an_unsupported_ir_version() {
        let mut document: Document =
            serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
        document.ir_version = "9.0".into();
        assert!(super::emit(&document).is_err());
        assert_eq!(document.ir_version, "9.0");
        assert_eq!(IR_VERSION, "0.2");
    }

    #[test]
    fn rejects_form_and_multipart_responses() {
        for content in [ContentKind::Form, ContentKind::Multipart] {
            let mut document: Document =
                serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
            document.operations[0].responses[0].content = content;
            let error = super::emit(&document).unwrap_err();
            assert!(error.0.contains("form or multipart responses"));
        }
    }

    #[test]
    fn emits_ndjson_streams_and_rejects_other_stream_kinds() {
        let document: Document =
            serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
        let mut document = document;
        let stream_index = document
            .operations
            .iter()
            .position(|operation| operation.stream.is_some())
            .unwrap();
        document.operations[stream_index]
            .stream
            .as_mut()
            .unwrap()
            .kind = StreamKind::Ndjson {
            item: pons_ir::TypeRef::Inline(pons_ir::InlineType::String {
                format: None,
                min_length: None,
                max_length: None,
                pattern: None,
            }),
        };
        let files = super::emit(&document).unwrap();
        let client = files.iter().find(|file| file.path == "client.ts").unwrap();
        assert!(client.content.contains("readNdjson"));
        assert!(client.content.contains("application/x-ndjson"));
        assert!(client.content.contains("AsyncIterable<Result"));

        document.operations[stream_index]
            .stream
            .as_mut()
            .unwrap()
            .kind = StreamKind::Websocket {
            client: pons_ir::TypeRef::Inline(pons_ir::InlineType::Any),
            server: pons_ir::TypeRef::Inline(pons_ir::InlineType::Any),
        };
        let error = super::emit(&document).unwrap_err();
        assert!(error.0.contains("SSE and NDJSON streams only"));
    }
}
