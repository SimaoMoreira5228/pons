use std::collections::BTreeMap;

use pons_ir::{
    Document, InlineType, IntegerWire, ObjectOpenness, ResponseStatus, StreamKind, TypeKind,
    TypeRef,
};
use serde_json::json;

use super::{emitted, file, fixture};

#[test]
fn emits_a_client_for_the_contract_fixture() {
    let mut document: Document =
        serde_json::from_str(include_str!("../../../../spec/fixtures/login.json")).unwrap();
    document.operations[0].provides.as_mut().unwrap().expires_at =
        Some(pons_ir::CredentialLocator::Body {
            pointer: "/expires_at".into(),
        });
    let files = crate::emit(&document).unwrap();
    let files = files
        .into_iter()
        .map(|file| (file.path, file.content))
        .collect::<BTreeMap<_, _>>();
    let types = &files["types.ts"];
    let client = &files["client.ts"];
    let errors = &files["errors.ts"];
    let auth = &files["operations/auth.ts"];
    let events = &files["operations/events.ts"];
    let transport = &files["runtime/transport/request.ts"];
    let retry = &files["runtime/transport/retry.ts"];
    let response = &files["runtime/transport/response.ts"];
    let credentials = &files["runtime/credentials.ts"];
    let stream = &files["runtime/stream.ts"];
    assert!(types.contains("export type User"));
    assert!(types.contains("export type LoginError"));
    assert!(files["runtime.ts"].contains("export * from \"./runtime/index.js\";"));
    assert!(files["index.ts"].contains("export * from \"./client.js\";"));
    assert!(client.contains("export function createClient"));
    assert!(client.contains("auth"));
    assert!(errors.contains("neverthrow"));
    assert!(client.contains("kind: \"api_key\", location: \"header\""));
    assert!(client.contains("AuthAttachment"));
    assert!(client.contains("authAdapters?: Record<string, AuthAdapter>"));
    assert!(transport.contains("group.length === 0"));
    assert!(transport.contains("_tag: \"AuthAdapterError\""));
    assert!(transport.contains("attachment.headers"));
    assert!(client.contains("signal?: AbortSignal"));
    assert!(client.contains("timeoutMs?: number"));
    assert!(client.contains("export type CallOptions"));
    assert!(client.contains("headers?: HeadersInit"));
    assert!(transport.contains("callOptions?.headers"));
    assert!(transport.contains("callOptions?.timeoutMs ?? client.timeoutMs"));
    assert!(transport.contains("callOptions?.signal ?? client.signal"));
    assert!(auth.contains("options?: CallOptions"));
    assert!(client.contains("export type Transport"));
    assert!(client.contains("export type TransportRequestHook"));
    assert!(client.contains("export type TransportResponseHook"));
    assert!(client.contains("headers?: HeadersInit |"));
    assert!(client.contains("transport?: Transport"));
    assert!(transport.contains("client.transport?.onRequest"));
    assert!(transport.contains("client.transport?.onResponse"));
    assert!(transport.contains("_tag: \"TransportHookError\""));
    assert!(client.contains("export type RetryOptions"));
    assert!(client.contains("retry?: RetryOptions | false"));
    assert!(retry.contains("fetchWithRetry"));
    assert!(retry.contains("retry-after"));
    assert!(retry.contains("Date.parse(retryAfterText)"));
    assert!(retry.contains("idempotent: boolean"));
    assert!(retry.contains("setTimeout"));
    assert!(transport.contains("_tag: \"TimeoutError\""));
    assert!(transport.contains("_tag: \"AbortedError\""));
    assert!(auth.contains("ApiError<number, Extract<Types.LoginError"));
    assert!(auth.contains("ApiError<number"));
    assert!(transport.contains("headers: declaredError.headers"));
    assert!(errors.contains("ResponseWithHeaders"));
    assert!(response.contains("withResponseHeaders"));
    assert!(response.contains("return { value, headers: responseHeaders"));
    assert!(client.contains("export type TokenStore"));
    assert!(
        client.contains("A store returns a `{ value, ...metadata }` record, never a bare string")
    );
    assert!(client.contains("export type CredentialSource"));
    assert!(transport.contains("_tag: \"AuthSourceError\""));
    assert!(credentials.contains("await source.set(nextCredential, {"));
    assert!(credentials.contains("client.authRecords[scheme] = records"));
    assert!(credentials.contains("credentialRecords(client, provides.scheme)"));
    assert!(credentials.contains("storeCredentialRecord(client, provides.scheme, next)"));
    assert!(client.contains("path?: string"));
    assert!(credentials.contains("let expiresAt: number | undefined;"));
    assert!(credentials.contains("Date.parse(value)"));
    assert!(credentials.contains("source.set(nextCredential, {"));
    assert!(credentials.contains("host: credentialUrl.hostname"));
    assert!(events.contains("AsyncIterable<Result<EventsWatchEvent, EventsWatchStreamError>>"));
    assert!(events.contains(
        "ResponseWithHeaders<AsyncIterable<Result<EventsWatchEvent, EventsWatchStreamError>>"
    ));
    assert!(events.contains("event: \"user\""));
    assert!(stream.contains("name === \"error\""));
    assert!(events.contains("headers: [{ name: \"X-Trace-ID\", required: true, schema:"));
    assert!(transport.contains("missing required response header"));
    assert!(transport.contains("expectedContentTypes"));
    assert!(transport.contains("application/problem+json"));
    assert!(transport.contains("stream === \"ndjson\""));
    assert!(transport.contains("\"text/event-stream\""));
    assert!(
        transport
            .contains("responseMediaTypeMatches(actualContentType, expectedContentType ?? \"\")")
    );
    assert!(response.contains("function mediaTypeMatches"));
    assert!(transport.contains("const text = await readResponseText(response, responseContent)"));
    assert!(transport.contains("rawValue = parseJson(text, preserveWideIntegers)"));
    assert!(transport.contains("invalid JSON response"));
    assert!(!transport.contains("responseContent.includes(\"text/event-stream\")"));
    assert!(transport.contains("response.headers.has(header.name)"));
    assert!(transport.contains("header.required && !response.headers.has(header.name)"));
    assert!(transport.contains("responseSchema?.headers"));
    let missing_header_branch = transport
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
    assert!(missing_header_branch.contains("decodeFailure"));
    assert!(transport.contains("_tag: \"UnexpectedStatusError\""));
    assert!(transport.contains("matchResponse"));
    assert!(!transport.contains("response.ok"));
    let declared_success = transport.find("const declaredSuccess").unwrap();
    let declared_error = transport.find("const declaredError").unwrap();
    let undeclared_success = transport.find("_tag: \"UnexpectedStatusError\"").unwrap();
    assert!(declared_success < declared_error && declared_error < undeclared_success);
    let stream_capture = transport
        .find("captureCredential(client, response, provides, undefined, false, finalUrl)")
        .unwrap();
    let stream_return = transport
        .find("return ok(withSuccessStatus(readSse")
        .unwrap();
    assert!(stream_capture < stream_return);
}

#[test]
fn emits_auth_scopes_for_custom_adapters() {
    let mut document: Document =
        serde_json::from_str(include_str!("../../../../spec/fixtures/login.json")).unwrap();
    document.operations[1]
        .auth
        .scopes
        .insert("tenant".into(), vec!["events:read".into()]);
    let output = crate::emit(&document).unwrap();
    assert!(file(&output, "operations/events.ts").contains("\"tenant\": [\"events:read\"]"));
    assert!(
        file(&output, "runtime/transport/request.ts")
            .contains("Object.prototype.hasOwnProperty.call(authScopes, scheme)")
    );
}

#[test]
fn emits_object_openness_types() {
    let mut document: Document =
        serde_json::from_str(include_str!("../../../../spec/fixtures/login.json")).unwrap();
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
    let output = crate::emit(&document).unwrap();
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
        serde_json::from_str(include_str!("../../../../spec/fixtures/login.json")).unwrap();
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
    let wire = crate::emit(&document)
        .unwrap()
        .into_iter()
        .find(|file| file.path == "runtime/wire.ts")
        .unwrap()
        .content;
    assert!(
        wire.contains("is read_only and cannot cross the encode direction"),
        "{wire}"
    );
    assert!(
        wire.contains("validateWire(item, extra, mode"),
        "the openness value types the extras it preserves: {wire}"
    );
    assert!(
        !wire.contains("|| key !== undefined"),
        "the unknown-key check cannot wait for a synthesized map key: {wire}"
    );
}

#[test]
fn emits_fields_according_to_direction_and_access() {
    let mut raw: serde_json::Value =
        serde_json::from_str(include_str!("../../../../spec/fixtures/login.json")).unwrap();
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
    let types = crate::emit(&document)
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
        serde_json::from_str(include_str!("../../../../spec/fixtures/login.json")).unwrap();
    raw["types"]["User"]["fields"][0]["wire"] = json!("Types.field");
    raw["types"]["LoginError"]["repr"]["tag"] = json!("Types.tag");
    raw["types"]["LoginError"]["variants"][0]["wire"] = json!("Types.variant");
    raw["operations"][0]["errors"][0]["body"]["variant"] = json!("Types.variant");
    let document: Document = serde_json::from_value(raw).unwrap();
    let files = crate::emit(&document).unwrap();
    let types = files
        .into_iter()
        .find(|file| file.path == "types.ts")
        .unwrap()
        .content;
    assert!(types.contains("\"Types.field\": string"));
    assert!(types.contains("\"Types.tag\": \"Types.variant\""));
    let auth = crate::emit(&document)
        .unwrap()
        .into_iter()
        .find(|file| file.path == "operations/auth.ts")
        .unwrap()
        .content;
    assert!(auth.contains("Types.LoginRequest"));
}

#[test]
fn checks_typed_openness_against_flattened_fields() {
    let mut raw: serde_json::Value =
        serde_json::from_str(include_str!("../../../../spec/fixtures/login.json")).unwrap();
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
    let types = crate::emit(&document)
        .unwrap()
        .into_iter()
        .find(|file| file.path == "types.ts")
        .unwrap()
        .content;
    assert!(
        types
            .contains("export type Outer = { \"own\": string; [key: string]: string } & Metadata;")
    );
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
        crate::emit(&document)
            .unwrap_err()
            .0
            .contains("direct or flattened fields"),
        "one shape under two names is the check the IR shape rule cannot make"
    );
}

#[test]
fn emits_optional_request_body_arguments() {
    let mut document: Document =
        serde_json::from_str(include_str!("../../../../spec/fixtures/login.json")).unwrap();
    document.operations[0].body.as_mut().unwrap().required = false;
    let output = crate::emit(&document).unwrap();
    let client = output
        .into_iter()
        .find(|file| file.path == "operations/auth.ts")
        .unwrap();
    assert!(client.content.contains("body?:"));
}

#[test]
fn emits_unknown_success_statuses() {
    let mut document: Document =
        serde_json::from_str(include_str!("../../../../spec/fixtures/login.json")).unwrap();
    document.operations[0].responses[0].status = ResponseStatus::Unknown;
    let output = crate::emit(&document).unwrap();
    let auth = file(&output, "operations/auth.ts");
    let transport = file(&output, "runtime/transport/request.ts");
    assert!(auth.contains("ResponseWithStatus<"));
    assert!(auth.contains("status: \"unknown\""));
    assert!(
        transport.contains("const matched = matchResponse(responses, errors, response.status)")
    );
    assert!(transport.contains("const declaredSuccess = matched.success"));
    assert!(transport.contains("const declaredError = matched.error"));
    assert!(!transport.contains("findSuccess"));

    document.operations[0].responses[0].status = ResponseStatus::Default;
    let output = crate::emit(&document).unwrap();
    assert!(file(&output, "operations/auth.ts").contains("ResponseWithStatus<"));
}

#[test]
fn emits_custom_media_types_for_json_and_bytes() {
    let mut document: Document =
        serde_json::from_str(include_str!("../../../../spec/fixtures/login.json")).unwrap();
    let login = document
        .operations
        .iter_mut()
        .find(|operation| operation.id[0][0] == "auth")
        .unwrap();
    login.body.as_mut().unwrap().media = Some(pons_ir::MediaDescriptor::from_declared(
        "application/vnd.acme+json",
    ));
    login.responses[0].media = Some(pons_ir::MediaDescriptor::from_declared(
        "application/vnd.acme.session+json",
    ));
    let output = crate::emit(&document).unwrap();
    let auth = file(&output, "operations/auth.ts");
    assert!(auth.contains("\"application/vnd.acme+json\""));
    assert!(auth.contains("mediaType: \"application/vnd.acme.session+json\""));
}

#[test]
fn emits_throwing_error_strategy() {
    let document: Document =
        serde_json::from_str(include_str!("../../../../spec/fixtures/login.json")).unwrap();
    let output = crate::emit_with_strategy(
        &document,
        crate::EmitMode::Strict,
        crate::ErrorStrategy::Throw,
    )
    .unwrap();
    let errors = file(&output.files, "errors.ts");
    assert!(file(&output.files, "operations/auth.ts").contains("requestOrThrow(request<"));
    assert!(!errors.contains("from \"neverthrow\""));
    assert!(errors.contains("type Result<T, E>"));
}

#[test]
fn emits_optional_response_headers() {
    let mut document: Document =
        serde_json::from_str(include_str!("../../../../spec/fixtures/login.json")).unwrap();
    document.operations[1].responses[0].headers[0].presence = pons_ir::Presence::Optional;
    let files = crate::emit(&document).unwrap();
    let events = file(&files, "operations/events.ts");
    assert!(events.contains("headers: [{ name: \"X-Trace-ID\", required: false, schema:"));
}

#[test]
fn emits_declared_error_response_headers() {
    let mut document: Document =
        serde_json::from_str(include_str!("../../../../spec/fixtures/login.json")).unwrap();
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
    let files = crate::emit(&document).unwrap();
    let auth = file(&files, "operations/auth.ts");
    assert!(auth.contains("ApiError<number"));
    assert!(auth.contains("\"Retry-After\": string"));
    assert!(auth.contains("retryable: true"));
    assert!(auth.contains("Retry-After"));
}

#[test]
fn preserves_wide_integers_in_error_response_headers() {
    let mut document: Document =
        serde_json::from_str(include_str!("../../../../spec/fixtures/login.json")).unwrap();
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

    let output = crate::emit(&document).unwrap();
    let transport = file(&output, "runtime/transport/response.ts");
    assert!(file(&output, "operations/auth.ts").contains("\"X-Retry-Count\": [bigint]"));
    assert!(transport.contains(
        "decodeWire(parseJson(value, preserveWideIntegers), schema, preserveWideIntegers, opaque)"
    ));
    assert!(!transport.contains("decodeWire(JSON.parse(value), schema)"));

    let output = crate::emit_with_mode(&document, crate::EmitMode::Lossy).unwrap();
    let warning = output
        .warnings
        .iter()
        .find(|warning| warning.contains("operations[0].errors[0].headers[0].ty"))
        .unwrap();
    assert!(warning.contains("operations[0].errors[0].headers[0].ty"));
    assert!(warning.contains("common_errors[0].headers[0].ty"));
}

#[test]
fn types_sse_error_events_as_stream_failures() {
    let mut document: Document =
        serde_json::from_str(include_str!("../../../../spec/fixtures/login.json")).unwrap();
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

    let files = crate::emit(&document).unwrap();
    let events = file(&files, "operations/events.ts");
    assert!(events.contains("SseStreamError<string>"));
    assert!(!events.contains("event: \"error\"; data:"));
}

#[test]
fn emission_is_independent_of_operation_order() {
    let mut document: Document =
        serde_json::from_str(include_str!("../../../../spec/fixtures/login.json")).unwrap();
    let expected = crate::emit(&document).unwrap();
    document.operations.reverse();
    assert_eq!(crate::emit(&document).unwrap(), expected);
}

#[test]
fn emits_ndjson_streams_and_rejects_other_stream_kinds() {
    let document: Document =
        serde_json::from_str(include_str!("../../../../spec/fixtures/login.json")).unwrap();
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
    let files = crate::emit(&document).unwrap();
    assert!(file(&files, "runtime/stream.ts").contains("readNdjson"));
    assert!(file(&files, "runtime/transport/request.ts").contains("application/x-ndjson"));
    assert!(file(&files, "operations/events.ts").contains("AsyncIterable<Result"));

    document.operations[stream_index]
        .stream
        .as_mut()
        .unwrap()
        .kind = StreamKind::Websocket {
        client: pons_ir::TypeRef::Inline(pons_ir::InlineType::Any),
        server: pons_ir::TypeRef::Inline(pons_ir::InlineType::Any),
    };
    let files = crate::emit(&document).unwrap();
    assert!(file(&files, "runtime/websocket.ts").contains("export async function websocket"));
    assert!(file(&files, "operations/events.ts").contains("WebSocketConnection<"));

    document.operations[stream_index]
        .stream
        .as_mut()
        .unwrap()
        .kind = StreamKind::Bytes {
        content_type: "application/octet-stream".into(),
    };
    let error = crate::emit(&document).unwrap_err();
    assert!(error.0.contains("SSE, NDJSON, and WebSocket streams only"));
}

#[test]
fn the_runtime_call_reads_as_a_request_spec() {
    let (_, _, operations) = emitted(&fixture());
    let login = spec_of(
        &operations["auth"],
        "request<AuthLoginResponse, AuthLoginError>",
    );
    assert_eq!(
        login,
        r#"{ method: "POST", path: ["", encodePathSegment("auth"), encodePathSegment("login")].join("/"), body: args.body, bodyContent: "json", bodySchema: {"ref":"LoginRequest"}, inputs: { path: [], query: undefined, headers: [], bodyRequired: true }, responses: [{ status: 200, content: "json", schema: {"ref":"User"} }], errors: [{ status: "4xx", content: "json", schema: {"ref":"LoginError","variant":"invalid"} }], provides: { scheme: "session", value: { from: "body", pointer: "/token" }, expiresAt: undefined }, options: options }"#
    );
    let watch = spec_of(
        &operations["events"],
        "request<EventsWatchResponse, EventsWatchError>",
    );
    assert_eq!(
        keys_of(login),
        10,
        "a spec names what the operation uses:\n{login}"
    );
    assert!(
        keys_of(watch) <= 12,
        "a spec stays a handful of keys:\n{watch}"
    );
    for key in [
        "pathValues: args.path",
        "query: args.query",
        "headers: args.headers",
        "auth: { alternatives: [[\"tenant\"]] }",
        "options: options",
        "idempotent: true",
    ] {
        assert!(watch.contains(key), "{key} is missing:\n{watch}");
    }
}

fn spec_of<'a>(client: &'a str, call: &str) -> &'a str {
    let line = client
        .lines()
        .find(|line| line.contains(call))
        .unwrap_or_else(|| panic!("no emitted call for {call}"));
    let start = line.find("configured, {").unwrap() + "configured, ".len();
    let end = line.rfind("})").unwrap() + 1;
    &line[start..end]
}

fn keys_of(spec: &str) -> usize {
    let mut depth = 0i32;
    let mut keys = 0usize;
    for character in spec.chars() {
        match character {
            '{' | '[' | '(' => depth += 1,
            '}' | ']' | ')' => depth -= 1,
            ',' if depth == 1 => keys += 1,
            _ => {}
        }
    }
    keys + 1
}
