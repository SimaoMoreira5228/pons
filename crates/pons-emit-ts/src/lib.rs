mod client;
mod runtime;
mod runtime_auth;
mod types;
mod wire_validation;

use std::collections::{BTreeMap, BTreeSet};

use pons_ir::{
    ApiKeyLocation, AuthScheme, Coverage, DecimalWire, Direction, Document, InlineType,
    IntegerWire, ObjectOpenness, PathSegment, StreamKind, TypeDef, TypeKind, TypeRef,
    VariantPayload, coverage_for_document, operation_subject, strict_violation_message,
    type_subject,
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
    pub coverage: Coverage,
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
        operation.body.as_ref().is_some_and(|body| {
            body.content == pons_ir::ContentKind::Multipart && body.media_type.is_some()
        })
    }) {
        return Err(EmitError(
            "the TypeScript emitter does not support multipart media type overrides".into(),
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
    let api_key_headers: BTreeMap<String, &str> = document
        .auth_schemes
        .iter()
        .filter_map(|(id, scheme)| match scheme {
            AuthScheme::ApiKey {
                location: ApiKeyLocation::Header,
                name,
            } => Some((name.to_ascii_lowercase(), id.as_str())),
            _ => None,
        })
        .collect();
    for (index, operation) in document.operations.iter().enumerate() {
        for header in &operation.headers {
            let name = header.wire.to_ascii_lowercase();
            if let Some(scheme) = api_key_headers.get(&name) {
                return Err(EmitError(format!(
                    "the TypeScript emitter rejects the request header at operations[{index}].headers that the {scheme} credential overwrites: {}",
                    header.wire
                )));
            }
        }
    }
    let mut coverage =
        coverage_for_document(&document, mode == EmitMode::Lossy).map_err(EmitError)?;
    let type_scan = scan_type_references(&document);
    if let Some(path) = type_scan.fractional_constraint_paths.first() {
        return Err(EmitError(format!(
            "the TypeScript emitter cannot enforce fractional integer constraint {path}"
        )));
    }
    if mode == EmitMode::Strict && !type_scan.decimal_paths.is_empty() {
        return Err(EmitError(format!(
            "the TypeScript emitter cannot read a decimal that crosses the wire as a number at {}, because a JSON number rounds the digits the contract declares",
            type_scan.decimal_paths.join(", ")
        )));
    }
    for subject in subjects_reaching_a_decimal(&type_scan, &document) {
        if coverage.exact.remove(&subject) || coverage.opaque.remove(&subject) {
            coverage.lossy.insert(subject);
        }
    }
    let decimal_paths = type_scan.decimal_paths;
    let opaque_paths = type_scan.opaque_paths;
    let wide_integers = type_scan.wide_paths;
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
    strict_coverage_error(mode, &coverage)?;
    let mut warnings = Vec::new();
    if mode == EmitMode::Lossy {
        if !wide_integers.is_empty() {
            warnings.push(format!(
                "TypeScript emits wide numeric integers as number at {}",
                wide_integers.join(", ")
            ));
        }
        if !decimal_paths.is_empty() {
            warnings.push(format!(
                "TypeScript emits decimals that cross the wire as numbers at {}",
                decimal_paths.join(", ")
            ));
        }
        if !opaque_paths.is_empty() {
            warnings.push(format!(
                "TypeScript emits opaque values as unknown at {}",
                opaque_paths.join(", ")
            ));
        }
    }
    Ok(EmitOutput {
        files,
        warnings,
        coverage,
    })
}

fn strict_coverage_error(mode: EmitMode, coverage: &Coverage) -> Result<(), EmitError> {
    match mode {
        EmitMode::Strict => strict_violation_message(coverage)
            .map(|violations| EmitError(format!("the TypeScript emitter reported {violations}")))
            .map_or(Ok(()), Err),
        EmitMode::Lossy => Ok(()),
    }
}

#[derive(Default)]
struct TypeScan {
    wide_paths: Vec<String>,
    opaque_paths: Vec<String>,
    decimal_paths: Vec<String>,
    fractional_constraint_paths: Vec<String>,
    decimal_owners: BTreeSet<String>,
    references: BTreeMap<String, Vec<String>>,
}

const COMMON_ERRORS: &str = "common_errors";

fn subjects_reaching_a_decimal(scan: &TypeScan, document: &Document) -> BTreeSet<String> {
    let mut subjects = scan.decimal_owners.clone();
    loop {
        let reached = scan
            .references
            .iter()
            .filter(|(owner, references)| {
                !subjects.contains(*owner)
                    && references.iter().any(|target| subjects.contains(target))
            })
            .map(|(owner, _)| owner.clone())
            .collect::<Vec<_>>();
        if reached.is_empty() {
            break;
        }
        subjects.extend(reached);
    }
    if subjects.remove(COMMON_ERRORS) {
        subjects.extend(
            document
                .operations
                .iter()
                .filter(|operation| operation.errors_inherit)
                .map(|operation| format!("operations.{}", operation_subject(operation))),
        );
    }
    subjects
}

fn scan_type_references(document: &Document) -> TypeScan {
    let mut scan = TypeScan::default();
    for (id, definition) in &document.types {
        let subject = type_subject(id);
        scan_definition(&subject, &subject, definition, &mut scan);
    }
    for (index, error) in document.common_errors.iter().enumerate() {
        if let Some(body) = &error.body {
            scan_type_ref(
                body,
                &format!("common_errors[{index}].body"),
                COMMON_ERRORS,
                &mut scan,
            );
        }
        for (header_index, header) in error.headers.iter().enumerate() {
            scan_type_ref(
                &header.ty,
                &format!("common_errors[{index}].headers[{header_index}].ty"),
                COMMON_ERRORS,
                &mut scan,
            );
        }
    }
    for (index, operation) in document.operations.iter().enumerate() {
        let prefix = format!("operations[{index}]");
        let subject = format!("operations.{}", operation_subject(operation));
        for (segment_index, segment) in operation.path.iter().enumerate() {
            if let PathSegment::Param { ty, .. } = segment {
                scan_type_ref(
                    ty,
                    &format!("{prefix}.path[{segment_index}].ty"),
                    &subject,
                    &mut scan,
                );
            }
        }
        if let Some(query) = &operation.query {
            scan_type_ref(query, &format!("{prefix}.query"), &subject, &mut scan);
        }
        for (header_index, header) in operation.headers.iter().enumerate() {
            scan_type_ref(
                &header.ty,
                &format!("{prefix}.headers[{header_index}].ty"),
                &subject,
                &mut scan,
            );
        }
        if let Some(body) = &operation.body {
            scan_type_ref(&body.ty, &format!("{prefix}.body.ty"), &subject, &mut scan);
        }
        for (response_index, response) in operation.responses.iter().enumerate() {
            if let Some(body) = &response.body {
                scan_type_ref(
                    body,
                    &format!("{prefix}.responses[{response_index}].body"),
                    &subject,
                    &mut scan,
                );
            }
            for (header_index, header) in response.headers.iter().enumerate() {
                scan_type_ref(
                    &header.ty,
                    &format!("{prefix}.responses[{response_index}].headers[{header_index}].ty"),
                    &subject,
                    &mut scan,
                );
            }
        }
        for (error_index, error) in operation.errors.iter().enumerate() {
            if let Some(body) = &error.body {
                scan_type_ref(
                    body,
                    &format!("{prefix}.errors[{error_index}].body"),
                    &subject,
                    &mut scan,
                );
            }
            for (header_index, header) in error.headers.iter().enumerate() {
                scan_type_ref(
                    &header.ty,
                    &format!("{prefix}.errors[{error_index}].headers[{header_index}].ty"),
                    &subject,
                    &mut scan,
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
                            &subject,
                            &mut scan,
                        );
                    }
                }
                StreamKind::Ndjson { item } => {
                    scan_type_ref(item, &format!("{prefix}.stream.item"), &subject, &mut scan);
                }
                StreamKind::Websocket { client, server } => {
                    scan_type_ref(
                        client,
                        &format!("{prefix}.stream.client"),
                        &subject,
                        &mut scan,
                    );
                    scan_type_ref(
                        server,
                        &format!("{prefix}.stream.server"),
                        &subject,
                        &mut scan,
                    );
                }
                StreamKind::Bytes { .. } => {}
            }
        }
    }
    scan
}

fn scan_definition(path: &str, owner: &str, definition: &TypeDef, scan: &mut TypeScan) {
    match &definition.definition {
        TypeKind::Struct {
            fields,
            flattened,
            openness,
        } => {
            for (index, field) in fields.iter().enumerate() {
                scan_type_ref(
                    &field.ty,
                    &format!("{path}.fields[{index}].ty"),
                    owner,
                    scan,
                );
            }
            for (index, reference) in flattened.iter().enumerate() {
                scan_type_ref(
                    reference,
                    &format!("{path}.flattened[{index}]"),
                    owner,
                    scan,
                );
            }
            if let ObjectOpenness::PreserveTyped { value } = openness {
                scan_type_ref(value, &format!("{path}.openness.value"), owner, scan);
            }
        }
        TypeKind::Enum { variants, .. } => {
            for (variant_index, variant) in variants.iter().enumerate() {
                scan_payload(
                    &variant.payload,
                    &format!("{path}.variants[{variant_index}].payload"),
                    owner,
                    scan,
                );
            }
        }
        TypeKind::Newtype { inner } => scan_type_ref(inner, &format!("{path}.inner"), owner, scan),
    }
}

fn scan_payload(payload: &VariantPayload, path: &str, owner: &str, scan: &mut TypeScan) {
    match payload {
        VariantPayload::Unit => {}
        VariantPayload::Newtype { ty } => scan_type_ref(ty, &format!("{path}.ty"), owner, scan),
        VariantPayload::Tuple { items } => {
            for (index, item) in items.iter().enumerate() {
                scan_type_ref(item, &format!("{path}.items[{index}]"), owner, scan);
            }
        }
        VariantPayload::Struct { fields } => {
            for (index, field) in fields.iter().enumerate() {
                scan_type_ref(
                    &field.ty,
                    &format!("{path}.fields[{index}].ty"),
                    owner,
                    scan,
                );
            }
        }
    }
}

fn scan_type_ref(reference: &TypeRef, path: &str, owner: &str, scan: &mut TypeScan) {
    if let TypeRef::Named(named) = reference {
        scan.references
            .entry(owner.to_owned())
            .or_default()
            .push(type_subject(&named.id));
        return;
    }
    let TypeRef::Inline(inline) = reference else {
        return;
    };
    match inline {
        InlineType::Int {
            bits,
            wire,
            minimum,
            maximum,
            multiple_of,
            ..
        } => {
            if *wire == IntegerWire::Number && *bits > 53 {
                scan.wide_paths.push(path.to_owned());
            }
            for (name, value) in [
                ("minimum", minimum),
                ("maximum", maximum),
                ("multiple_of", multiple_of),
            ] {
                if value
                    .as_ref()
                    .is_some_and(|value| !is_integer_number(value))
                {
                    scan.fractional_constraint_paths
                        .push(format!("{path}.{name}"));
                }
            }
        }
        InlineType::Decimal {
            wire: DecimalWire::Number,
        } => {
            scan.decimal_paths.push(path.to_owned());
            scan.decimal_owners.insert(owner.to_owned());
        }
        InlineType::Opaque { .. } => scan.opaque_paths.push(path.to_owned()),
        InlineType::Nullable { inner } => {
            scan_type_ref(inner, &format!("{path}.inner"), owner, scan)
        }
        InlineType::Array { item, .. } => scan_type_ref(item, &format!("{path}.item"), owner, scan),
        InlineType::Map { key, value } => {
            scan_type_ref(key, &format!("{path}.key"), owner, scan);
            scan_type_ref(value, &format!("{path}.value"), owner, scan);
        }
        InlineType::Tuple { items } => {
            for (index, item) in items.iter().enumerate() {
                scan_type_ref(item, &format!("{path}.items[{index}]"), owner, scan);
            }
        }
        _ => {}
    }
}

fn is_integer_number(value: &serde_json::Value) -> bool {
    value
        .as_number()
        .is_some_and(|number| number.as_i128().is_some() || number.as_u128().is_some())
}

const RESERVED_NAMES: &[&str] = &[
    "Array",
    "Date",
    "Error",
    "Extract",
    "JSON",
    "Promise",
    "ReadonlyArray",
    "Record",
    "Uint8Array",
];

fn unique_names(bases: impl IntoIterator<Item = String>) -> Vec<String> {
    let bases = bases.into_iter().collect::<Vec<_>>();
    let reserved = bases.iter().cloned().collect::<BTreeSet<_>>();
    let mut assigned = RESERVED_NAMES
        .iter()
        .map(|name| (*name).to_owned())
        .collect::<BTreeSet<_>>();
    let mut seen = BTreeSet::new();
    bases
        .into_iter()
        .map(|base| {
            let first = seen.insert(base.clone());
            let mut suffix = if first { 1 } else { 2 };
            loop {
                let candidate = if suffix == 1 {
                    base.clone()
                } else {
                    format!("{base}{suffix}")
                };
                if (first || !reserved.contains(&candidate)) && assigned.insert(candidate.clone()) {
                    break candidate;
                }
                suffix += 1;
            }
        })
        .collect()
}

fn type_names(document: &Document) -> BTreeMap<String, String> {
    let mut entries = document
        .types
        .iter()
        .map(|(id, definition)| {
            (
                pair_name(document, definition),
                is_split_half(definition),
                id.clone(),
            )
        })
        .collect::<Vec<_>>();
    entries.sort();
    let names = unique_names(entries.iter().map(|(base, _, _)| base.clone()));
    entries
        .into_iter()
        .map(|(_, _, id)| id)
        .zip(names)
        .collect()
}

fn pair_name(document: &Document, definition: &TypeDef) -> String {
    let own = pascal(&definition.name);
    if !is_split_half(definition) {
        return own;
    }
    let base = definition
        .origin
        .as_deref()
        .and_then(|origin| document.types.get(origin))
        .filter(|partner| partner.direction == Direction::Output)
        .map_or(own, |partner| pascal(&partner.name));
    format!("{base}Input")
}

fn is_split_half(definition: &TypeDef) -> bool {
    definition.origin.is_some() && definition.direction == Direction::Input
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
        ContentKind, Document, IR_VERSION, InlineType, IntegerWire, NamedRef, ObjectOpenness,
        PathSegment, ResponseStatus, StreamKind, TypeKind, TypeRef,
    };
    use serde_json::{Value, json};
    use std::collections::{BTreeMap, BTreeSet};

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
        assert!(client.contains("group.length === 0"));
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
        assert!(client.contains("Date.parse(retryAfterText)"));
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
        assert!(
            client
                .contains("A store returns a `{ value, ...metadata }` record, never a bare string")
        );
        assert!(client.contains("export type CredentialSource"));
        assert!(client.contains("kind: \"auth_source\""));
        assert!(client.contains("await source.set(nextCredential, {"));
        assert!(client.contains("client.authRecords[scheme] = records"));
        assert!(client.contains("credentialRecords(client, provides.scheme)"));
        assert!(client.contains("storeCredentialRecord(client, provides.scheme, next)"));
        assert!(client.contains("path?: string"));
        assert!(client.contains("expiresAt:"));
        assert!(client.contains("Date.parse(value)"));
        assert!(client.contains("source.set(nextCredential, {"));
        assert!(client.contains("host: credentialUrl.hostname"));
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
        assert!(
            client.contains(
                "responseMediaTypeMatches(actualContentType, expectedContentType ?? \"\")"
            )
        );
        assert!(client.contains("function mediaTypeMatches"));
        assert!(client.contains("const text = await readResponseText(response, responseContent)"));
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
        assert!(client.contains("matchResponse"));
        assert!(!client.contains("response.ok"));
        let declared_success = client.find("const declaredSuccess").unwrap();
        let declared_error = client.find("const declaredError").unwrap();
        let undeclared_success = client.find("undeclared success response").unwrap();
        assert!(declared_success < declared_error && declared_error < undeclared_success);
        let stream_capture = client
            .find("captureCredential(client, response, provides, undefined, false, finalUrl)")
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
        assert!(
            client
                .content
                .contains("Object.prototype.hasOwnProperty.call(authScopes, scheme)")
        );
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
        let mut login_fields = fields.clone();
        login_fields[0].presence = pons_ir::Presence::Optional;
        login.definition = TypeKind::Struct {
            fields: login_fields,
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
        assert!(types.content.contains("[key: string]: string | undefined"));
        assert!(types.content.contains("[key: string]: unknown"));
    }

    #[test]
    fn the_wire_validator_refuses_what_the_encoder_would_drop() {
        let mut raw: serde_json::Value =
            serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
        raw["types"]["LoginRequest"] = json!({
            "kind": "struct",
            "name": ["login", "request"],
            "direction": "both",
            "openness": {"preserve_typed": {"value": {"kind": "string"}}},
            "fields": [
                {"wire": "email", "ident": ["email"], "ty": {"kind": "string"}, "presence": "required"},
                {"wire": "id", "ident": ["id"], "ty": {"kind": "string"}, "presence": "optional", "access": "read_only"}
            ]
        });
        let document: Document = serde_json::from_value(raw).unwrap();
        let client = super::emit(&document)
            .unwrap()
            .into_iter()
            .find(|file| file.path == "client.ts")
            .unwrap()
            .content;
        assert!(
            client.contains("is read_only and cannot cross the encode direction"),
            "{client}"
        );
        assert!(
            client.contains("validateWire(item, extra, mode"),
            "the openness value types the extras it preserves: {client}"
        );
        assert!(
            !client.contains("|| key !== undefined"),
            "the unknown-key check cannot wait for a synthesized map key: {client}"
        );
    }

    #[test]
    fn emits_fields_according_to_direction_and_access() {
        let mut raw: serde_json::Value =
            serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
        raw["types"]["LoginRequest"] = json!({
            "kind": "struct",
            "name": ["login", "request"],
            "direction": "input",
            "openness": "reject",
            "fields": [
                {"wire": "email", "ident": ["email"], "ty": {"kind": "string"}, "presence": "required"},
                {"wire": "secret", "ident": ["secret"], "ty": {"kind": "string"}, "presence": "required", "access": "write_only"},
                {"wire": "server_id", "ident": ["server", "id"], "ty": {"kind": "string"}, "presence": "required", "access": "read_only", "default": "filled"}
            ]
        });
        raw["types"]["User"] = json!({
            "kind": "struct",
            "name": ["user"],
            "direction": "output",
            "openness": "reject",
            "fields": [
                {"wire": "id", "ident": ["id"], "ty": {"kind": "string"}, "presence": "required", "access": "read_only"},
                {"wire": "secret", "ident": ["secret"], "ty": {"kind": "string"}, "presence": "required", "access": "write_only", "default": "hidden"},
                {"wire": "value", "ident": ["value"], "ty": {"kind": "string"}, "presence": "required"}
            ]
        });
        raw["types"]["Both"] = json!({
            "kind": "struct",
            "name": ["both"],
            "direction": "both",
            "openness": "reject",
            "fields": [
                {"wire": "id", "ident": ["id"], "ty": {"kind": "string"}, "presence": "required", "access": "read_only", "default": "filled"},
                {"wire": "secret", "ident": ["secret"], "ty": {"kind": "string"}, "presence": "required", "access": "write_only", "default": "hidden"},
                {"wire": "value", "ident": ["value"], "ty": {"kind": "string"}, "presence": "required"}
            ]
        });
        let document: Document = serde_json::from_value(raw).unwrap();
        let types = super::emit(&document)
            .unwrap()
            .into_iter()
            .find(|file| file.path == "types.ts")
            .unwrap()
            .content;
        assert!(
            types.contains("export type LoginRequest = { \"email\": string; \"secret\": string };")
        );
        assert!(types.contains("export type User = { \"id\": string; \"value\": string };"));
        assert!(types.contains(
            "export type Both = { \"id\"?: string; \"secret\"?: string; \"value\": string };"
        ));
    }

    #[test]
    fn preserves_types_prefix_in_wire_literals() {
        let mut raw: serde_json::Value =
            serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
        raw["types"]["User"]["fields"][0]["wire"] = json!("Types.field");
        raw["types"]["LoginError"]["repr"]["tag"] = json!("Types.tag");
        raw["types"]["LoginError"]["variants"][0]["wire"] = json!("Types.variant");
        raw["operations"][0]["errors"][0]["body"]["variant"] = json!("Types.variant");
        let document: Document = serde_json::from_value(raw).unwrap();
        let files = super::emit(&document).unwrap();
        let types = files
            .into_iter()
            .find(|file| file.path == "types.ts")
            .unwrap()
            .content;
        assert!(types.contains("\"Types.field\": string"));
        assert!(types.contains("\"Types.tag\": \"Types.variant\""));
        let client = super::emit(&document)
            .unwrap()
            .into_iter()
            .find(|file| file.path == "client.ts")
            .unwrap()
            .content;
        assert!(client.contains("Types.LoginRequest"));
    }

    #[test]
    fn checks_typed_openness_against_flattened_fields() {
        let mut raw: serde_json::Value =
            serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
        raw["types"]["Tag"] = json!({
            "kind": "newtype", "name": ["tag"], "direction": "both", "inner": {"kind": "string"}
        });
        raw["types"]["Metadata"] = json!({
            "kind": "struct",
            "name": ["metadata"],
            "direction": "both",
            "openness": "reject",
            "fields": [
                {"wire": "flat", "ident": ["flat"], "ty": {"kind": "string"}, "presence": "required"}
            ]
        });
        raw["types"]["Outer"] = json!({
            "kind": "struct",
            "name": ["outer"],
            "direction": "both",
            "openness": {"preserve_typed": {"value": {"kind": "string"}}},
            "fields": [
                {"wire": "own", "ident": ["own"], "ty": {"kind": "string"}, "presence": "required"}
            ],
            "flattened": [{"ref": "Metadata"}]
        });
        let mut document: Document = serde_json::from_value(raw).unwrap();
        let types = super::emit(&document)
            .unwrap()
            .into_iter()
            .find(|file| file.path == "types.ts")
            .unwrap()
            .content;
        assert!(types.contains(
            "export type Outer = { \"own\": string; [key: string]: string } & Metadata;"
        ));
        let TypeKind::Struct { fields, .. } =
            &mut document.types.get_mut("Metadata").unwrap().definition
        else {
            unreachable!()
        };
        fields[0].ty = TypeRef::Named(pons_ir::NamedRef {
            id: "Tag".into(),
            variant: None,
        });
        assert!(
            super::emit(&document)
                .unwrap_err()
                .0
                .contains("direct or flattened fields"),
            "one shape under two names is the check the IR shape rule cannot make"
        );
    }

    #[test]
    fn rejects_structs_flattened_with_maps() {
        let mut raw: serde_json::Value =
            serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
        raw["types"]["LoginRequest"]["flattened"] = json!([{
            "kind": "map",
            "key": {"kind": "string"},
            "value": {"kind": "string"}
        }]);
        let document: Document = serde_json::from_value(raw).unwrap();
        assert!(super::emit(&document).unwrap_err().0.contains(
            "is not a struct, because a flattened member contributes its fields to one object"
        ));
    }

    #[test]
    fn rejects_query_types_without_a_wire_name() {
        let scalar = || {
            TypeRef::Inline(InlineType::String {
                format: None,
                min_length: None,
                max_length: None,
                pattern: None,
            })
        };
        let mut document: Document =
            serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
        document.operations[1].query = Some(scalar());
        let error = super::emit(&document).unwrap_err();
        assert!(
            error.0.contains("[[\"events\"], [\"watch\"]]"),
            "{}",
            error.0
        );
        assert!(error.0.contains("structs"), "{}", error.0);

        let mut raw: serde_json::Value =
            serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
        raw["types"]["QueryAlias"] = json!({
            "kind": "newtype",
            "name": ["query", "alias"],
            "direction": "input",
            "inner": {"kind": "string"}
        });
        let document: Document = serde_json::from_value(raw.clone()).unwrap();
        let mut document = document;
        document.operations[1].query = Some(TypeRef::Named(pons_ir::NamedRef {
            id: "QueryAlias".into(),
            variant: None,
        }));
        assert!(
            super::emit(&document)
                .unwrap_err()
                .0
                .contains("[[\"events\"], [\"watch\"]]")
        );

        raw["types"]["QueryAlias"] = json!({
            "kind": "newtype",
            "name": ["query", "alias"],
            "direction": "input",
            "inner": {"ref": "EventQuery"}
        });
        let mut document: Document = serde_json::from_value(raw).unwrap();
        document.operations[1].query = Some(TypeRef::Named(pons_ir::NamedRef {
            id: "QueryAlias".into(),
            variant: None,
        }));
        assert!(super::emit(&document).is_ok());
    }

    #[test]
    fn rejects_nullable_flatten_before_emitting_a_contract() {
        let mut raw: serde_json::Value =
            serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
        raw["types"]["LoginRequest"]["flattened"] = json!([{
            "kind": "nullable",
            "inner": {"ref": "User"}
        }]);
        let document: Document = serde_json::from_value(raw).unwrap();
        for mode in [super::EmitMode::Strict, super::EmitMode::Lossy] {
            assert!(
                super::emit_with_mode(&document, mode)
                    .unwrap_err()
                    .0
                    .contains("nullable flattened")
            );
        }
    }

    #[test]
    fn rejects_streaming_request_bodies() {
        let mut document: Document =
            serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
        document.operations[0].body.as_mut().unwrap().content = ContentKind::Stream;
        for mode in [super::EmitMode::Strict, super::EmitMode::Lossy] {
            let error = super::emit_with_mode(&document, mode).unwrap_err();
            assert!(
                error.0.contains(
                    "a request body cannot be stream content, because no built-in target reads one"
                ),
                "{}",
                error.0
            );
        }
    }

    #[test]
    fn rejects_required_nullable_request_headers_and_path_parameters() {
        let mut document: Document =
            serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
        let watch = &mut document.operations[1];
        watch.headers[0].presence = pons_ir::Presence::Required;
        watch.headers[0].ty = TypeRef::Inline(InlineType::Nullable {
            inner: Box::new(TypeRef::Inline(InlineType::String {
                format: None,
                min_length: None,
                max_length: None,
                pattern: None,
            })),
        });
        let error = super::emit(&document).unwrap_err();
        assert!(
            error
                .0
                .contains("required request header X-Request-ID cannot be nullable"),
            "{}",
            error.0
        );

        let mut document: Document =
            serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
        document.operations[1].path[1] = PathSegment::Param {
            param: "cursor".into(),
            ty: TypeRef::Inline(InlineType::Nullable {
                inner: Box::new(TypeRef::Inline(InlineType::String {
                    format: None,
                    min_length: None,
                    max_length: None,
                    pattern: None,
                })),
            }),
        };
        let error = super::emit(&document).unwrap_err();
        assert!(
            error
                .0
                .contains("required path parameter cursor cannot be nullable"),
            "{}",
            error.0
        );
    }

    #[test]
    fn refuses_a_use_site_whose_type_cannot_cross_the_boundary() {
        let mut raw: serde_json::Value =
            serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
        raw["operations"][0]["body"] =
            json!({"ty": {"ref": "User"}, "content": "json", "required": true});
        let document: Document = serde_json::from_value(raw).unwrap();
        let error = super::emit(&document).unwrap_err();
        assert!(
            error.0.contains(
                "operations[0].body.ty: the request body needs a type of the input or both direction, but User is declared output"
            ),
            "{}",
            error.0
        );
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
                .contains("const matched = matchResponse(responses, errors, response.status)")
        );
        assert!(
            client
                .content
                .contains("const declaredSuccess = matched.success")
        );
        assert!(
            client
                .content
                .contains("const declaredError = matched.error")
        );
        assert!(!client.content.contains("findSuccess"));

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
    fn preserves_wide_integers_in_error_response_headers() {
        let mut document: Document =
            serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
        let wide_header = || pons_ir::ResponseHeader {
            wire: "X-Retry-Count".into(),
            ty: TypeRef::Inline(InlineType::Tuple {
                items: vec![TypeRef::Inline(InlineType::Int {
                    bits: 64,
                    signed: false,
                    wire: IntegerWire::Number,
                    minimum: None,
                    maximum: None,
                    multiple_of: None,
                })],
            }),
            presence: pons_ir::Presence::Required,
            ext: BTreeMap::new(),
        };
        document.operations[0].errors[0].headers.push(wide_header());
        let mut common = document.operations[0].errors[0].clone();
        common.name = vec!["common".into()];
        common.status = pons_ir::ErrorStatus::Class(5);
        document.common_errors.push(common);

        let output = super::emit(&document).unwrap();
        let client = output.iter().find(|file| file.path == "client.ts").unwrap();
        assert!(client.content.contains("\"X-Retry-Count\": [bigint]"));
        assert!(client.content.contains(
            "decodeWire(parseJson(value, preserveWideIntegers), schema, preserveWideIntegers, opaque)"
        ));
        assert!(
            !client
                .content
                .contains("decodeWire(JSON.parse(value), schema)")
        );

        let output = super::emit_with_mode(&document, super::EmitMode::Lossy).unwrap();
        let warning = output
            .warnings
            .iter()
            .find(|warning| warning.contains("operations[0].errors[0].headers[0].ty"))
            .unwrap();
        assert!(warning.contains("operations[0].errors[0].headers[0].ty"));
        assert!(warning.contains("common_errors[0].headers[0].ty"));
    }

    #[test]
    fn strict_mode_names_every_violating_coverage_class() {
        let coverage = super::Coverage {
            lossy: ["types.User".into()].into_iter().collect(),
            unsupported: ["operations.auth.login".into()].into_iter().collect(),
            ..super::Coverage::default()
        };
        let error = super::strict_coverage_error(super::EmitMode::Strict, &coverage)
            .unwrap_err()
            .0;
        assert_eq!(
            error,
            "the TypeScript emitter reported unsupported coverage in strict mode: \
             operations.auth.login; lossy coverage in strict mode: types.User",
            "every violating class is named, worst class first"
        );
        assert!(super::strict_coverage_error(super::EmitMode::Lossy, &coverage).is_ok());
        assert!(
            super::strict_coverage_error(super::EmitMode::Strict, &super::Coverage::default())
                .is_ok()
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
    fn coverage_agrees_with_the_canonical_coverage_of_the_fixture() {
        let document: Document =
            serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
        let canonical = pons_ir::canonical_coverage(&document, false).unwrap();
        let output = super::emit_with_mode(&document, super::EmitMode::Strict).unwrap();

        assert_eq!(
            output.coverage.levels().collect::<BTreeMap<_, _>>(),
            canonical
        );
        assert!(output.warnings.is_empty());
    }

    #[test]
    fn reports_wide_integer_coverage_by_mode() {
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

        let strict = super::emit_with_mode(&document, super::EmitMode::Strict).unwrap();
        assert!(!strict.coverage.exact.contains("types.User"));
        assert!(strict.coverage.opaque.contains("types.User"));
        assert!(strict.coverage.lossy.is_empty());
        let lossy = super::emit_with_mode(&document, super::EmitMode::Lossy).unwrap();
        assert!(lossy.coverage.lossy.contains("types.User"));
        assert!(lossy.coverage.lossy.contains("operations.auth.login"));
    }

    #[test]
    fn an_untagged_enum_is_lossy_and_strict_mode_refuses_it() {
        let document = untagged_document();
        let canonical = pons_ir::canonical_coverage(&document, false).unwrap();
        assert_eq!(
            canonical["types.Choice"],
            pons_ir::CoverageLevel::Lossy,
            "{canonical:?}"
        );

        let refused = super::emit_with_mode(&document, super::EmitMode::Strict).unwrap_err();
        assert_eq!(
            refused.to_string(),
            "the TypeScript emitter reported lossy coverage in strict mode: \
             operations.post.send, types.Choice"
        );
        let emitted = super::emit_with_mode(&document, super::EmitMode::Lossy).unwrap();
        assert_eq!(
            emitted.coverage.lossy,
            ["operations.post.send", "types.Choice"]
                .into_iter()
                .map(str::to_owned)
                .collect()
        );
        assert!(emitted.coverage.exact.is_empty());
    }

    fn untagged_document() -> Document {
        let document: Document = serde_json::from_value(serde_json::json!({
            "ir_version": "0.2",
            "info": {"title": "Example", "version": "1"},
            "types": {
                "Choice": {
                    "kind": "enum", "name": ["choice"], "direction": "both",
                    "repr": {"kind": "untagged"},
                    "variants": [
                        {"wire": "text", "ident": ["text"],
                         "payload": {"kind": "newtype", "ty": {"kind": "string"}}},
                        {"wire": "count", "ident": ["count"],
                         "payload": {"kind": "newtype", "ty": {"kind": "int", "bits": 8,
                             "signed": false, "wire": "number"}}}
                    ]
                }
            },
            "operations": [{
                "id": [["post"], ["send"]],
                "method": "POST",
                "path": [{"lit": "send"}],
                "body": {"ty": {"ref": "Choice"}, "content": "json", "required": true},
                "responses": [{"status": 200, "content": "json", "body": {"kind": "string"}}],
                "auth": {"alternatives": [], "optional": false},
                "semantics": {"safe": false, "idempotent": false}
            }]
        }))
        .unwrap();
        document.validate().unwrap();
        document
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
    fn allocates_unique_generated_declaration_names() {
        let mut document: Document =
            serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
        let template = document.types.get("LoginRequest").unwrap().clone();
        for (id, name) in [
            ("Collision0", vec!["collision"]),
            ("Collision1", vec!["collision", "2"]),
            ("Collision2", vec!["collision"]),
        ] {
            let mut definition = template.clone();
            definition.name = name.into_iter().map(str::to_owned).collect();
            document.types.insert(id.into(), definition);
        }
        let stream = document
            .operations
            .iter()
            .find(|operation| operation.stream.is_some())
            .unwrap()
            .clone();
        let mut ndjson = stream.clone();
        let item = match &ndjson.stream.as_ref().unwrap().kind {
            StreamKind::Sse { events } => events[0].data.clone(),
            _ => unreachable!(),
        };
        ndjson.stream.as_mut().unwrap().kind = StreamKind::Ndjson { item };
        document.operations = [
            (vec![vec!["a"]], stream.clone()),
            (vec![vec!["a", "stream"]], stream.clone()),
            (vec![vec!["b"]], ndjson),
            (vec![vec!["b", "stream"]], stream),
        ]
        .into_iter()
        .map(|(id, mut operation)| {
            operation.id = id
                .into_iter()
                .map(|segment| segment.into_iter().map(str::to_owned).collect())
                .collect();
            operation
        })
        .collect();

        let files = super::emit(&document).unwrap();
        let types = &files
            .iter()
            .find(|file| file.path == "types.ts")
            .unwrap()
            .content;
        let client = &files
            .iter()
            .find(|file| file.path == "client.ts")
            .unwrap()
            .content;
        for source in [types, client] {
            let declarations = source.matches("export type ").count();
            let names = source
                .lines()
                .filter_map(|line| line.strip_prefix("export type "))
                .map(|line| line.split_whitespace().next().unwrap())
                .collect::<std::collections::BTreeSet<_>>();
            assert_eq!(names.len(), declarations);
        }
        for name in ["Collision", "Collision2", "Collision3"] {
            assert!(types.contains(&format!("export type {name} =")));
        }
        for name in ["AEvent", "BItem"] {
            assert!(client.contains(&format!("export type {name} =")));
        }
        for name in [
            "AStreamError",
            "AStreamError2",
            "BStreamError",
            "BStreamError2",
        ] {
            assert!(client.contains(&format!("export type {name} =")));
        }
    }

    fn carriers(definitions: &[(&str, Value)]) -> Document {
        let mut raw: Value =
            serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
        for (id, definition) in definitions {
            raw["types"][*id] = definition.clone();
        }
        serde_json::from_value(raw).unwrap()
    }

    fn carrier(name: &[&str], direction: &str, origin: Option<&str>, marker: &str) -> Value {
        json!({
            "kind": "struct",
            "name": name,
            "direction": direction,
            "origin": origin,
            "openness": "reject",
            "fields": [{
                "wire": marker,
                "ident": [marker],
                "ty": {"kind": "string"},
                "presence": "required"
            }]
        })
    }

    fn file<'a>(files: &'a [crate::GeneratedFile], path: &str) -> &'a str {
        &files.iter().find(|file| file.path == path).unwrap().content
    }

    fn aliases(document: &Document) -> BTreeMap<String, String> {
        let files = super::emit(document).unwrap();
        let types = file(&files, "types.ts");
        let aliases = types
            .lines()
            .filter_map(|line| line.strip_prefix("export type "))
            .filter_map(|line| line.split_once(" = "))
            .map(|(name, body)| {
                (
                    name.to_owned(),
                    body.strip_suffix(';').unwrap_or(body).to_owned(),
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(
            aliases.len(),
            aliases
                .iter()
                .map(|(name, _)| name)
                .collect::<BTreeSet<_>>()
                .len(),
            "{aliases:?} declares one name twice"
        );
        aliases.into_iter().collect()
    }

    #[test]
    fn names_a_direction_pair_after_the_pair_and_never_after_the_id() {
        let mut document = carriers(&[
            (
                "router::Account",
                carrier(&["account"], "output", Some("router::Account"), "id"),
            ),
            (
                "storefront::AccountRequest",
                carrier(
                    &["account", "request"],
                    "input",
                    Some("router::Account"),
                    "email",
                ),
            ),
        ]);
        let login = &mut document.operations[0];
        login.body.as_mut().unwrap().ty = TypeRef::Named(NamedRef {
            id: "storefront::AccountRequest".into(),
            variant: None,
        });
        login.responses[0].body = Some(TypeRef::Named(NamedRef {
            id: "router::Account".into(),
            variant: None,
        }));

        let files = super::emit(&document).unwrap();
        let types = file(&files, "types.ts");
        let client = file(&files, "client.ts");
        assert!(
            types.contains("export type Account = { \"id\": string };"),
            "{types}"
        );
        assert!(
            types.contains("export type AccountInput = { \"email\": string };"),
            "{types}"
        );
        assert!(
            client.contains("Types.AccountInput") && client.contains("Types.Account"),
            "{client}"
        );
        assert!(
            types.lines().all(|line| !line.contains('#')),
            "{types} leaks an id"
        );

        let lone = carriers(&[(
            "storefront::AccountRequest",
            carrier(&["account"], "input", Some("router::Account"), "email"),
        )]);
        let files = super::emit(&lone).unwrap();
        assert!(
            file(&files, "types.ts").contains("export type AccountInput = { \"email\": string };"),
            "pruning the response half leaves the request half the pair's name"
        );
        assert!(!file(&files, "types.ts").contains("export type Account ="));
    }

    #[test]
    fn a_pair_that_declares_no_origin_keeps_its_own_display_name() {
        let names = aliases(&carriers(&[
            ("a::Account", carrier(&["account"], "output", None, "id")),
            ("b::Account", carrier(&["account"], "input", None, "email")),
        ]));
        assert_eq!(names["Account"], "{ \"id\": string }");
        assert_eq!(names["Account2"], "{ \"email\": string }");
        assert!(!names.contains_key("AccountInput"));
    }

    #[test]
    fn a_suffixed_alias_never_takes_the_alias_another_type_publishes() {
        let names = aliases(&carriers(&[
            ("a::Q", carrier(&["q"], "output", Some("a::Q"), "answer")),
            (
                "a::Q#input",
                carrier(&["q"], "input", Some("a::Q"), "first"),
            ),
            (
                "b::Q#input",
                carrier(&["q"], "input", Some("b::Q"), "second"),
            ),
            ("c::qInput", carrier(&["qInput"], "both", None, "published")),
        ]));
        assert_eq!(names["QInput"], "{ \"published\": string }");
        assert_eq!(names["QInput2"], "{ \"first\": string }");
        assert_eq!(names["QInput3"], "{ \"second\": string }");
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
