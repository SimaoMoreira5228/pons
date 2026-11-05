mod support;

use pons_emit_ts::{EmitMode, ErrorStrategy, emit_with_strategy};
use pons_ir::{ApiKeyLocation, ContentKind, InlineType, Presence, TypeRef};
use support::runtime::{request_input_document, run_bun, validated_files};

#[test]
fn returns_non_stream_success_values_for_stream_operations() {
    let mut raw: serde_json::Value =
        serde_json::from_str(include_str!("../../../spec/fixtures/login.json")).unwrap();
    raw["operations"][1]["responses"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::json!({
            "status": 201,
            "content": "json",
            "body": { "kind": "string" }
        }));
    let document: pons_ir::Document = serde_json::from_value(raw).unwrap();
    let files = emit_with_strategy(&document, EmitMode::Strict, ErrorStrategy::Throw)
        .unwrap()
        .files;
    assert!(
        files
            .iter()
            .find(|file| file.path == "operations/events.ts")
            .unwrap()
            .content
            .contains("| string")
    );
    let harness = r#"
import { createClient } from "./client.ts";

const client = createClient({
  baseUrl: "https://example.test",
  auth: { tenant: "tenant" },
  fetch: async () => new Response(JSON.stringify("json"), {
    status: 201,
    headers: { "content-type": "application/json" },
  }),
});
const value = await client.events.watch({ path: { cursor: "cursor" }, query: {}, headers: {} });
if (value !== "json") throw new Error(`non-stream success was not returned: ${String(value)}`);
"#;
    run_bun(&files, harness);
}

#[test]
fn generated_client_preflights_request_inputs() {
    let optional = request_input_document(false);
    let optional_files = emit_with_strategy(&optional, EmitMode::Strict, ErrorStrategy::Throw)
        .unwrap()
        .files;
    let optional_client = optional_files
        .iter()
        .find(|file| file.path == "runtime/transport/request.ts")
        .unwrap();
    assert!(optional_client.content.contains("preflightPath"));
    assert!(
        optional_client
            .content
            .contains("missing required request body")
    );
    let preflight = optional_client
        .content
        .find("const encodedPathValues")
        .unwrap();
    let url = optional_client.content.find("const url = new URL").unwrap();
    let fetch = optional_client
        .content
        .find("const redirectResult = await fetchWithRedirect")
        .unwrap();
    assert!(preflight < url && url < fetch);

    let optional_harness = r#"
import { createClient } from "./client.ts";

const response = () => new Response("", { status: 200, headers: { "content-type": "text/event-stream", "X-Trace-ID": "trace" } });
const expectFailure = async (operation, expected) => {
  try {
    await operation();
    throw new Error("operation was accepted");
  } catch (cause) {
    const text = JSON.stringify(cause);
    if (!text.includes(expected)) throw new Error(text);
  }
};
let sent = 0;
const client = createClient({
  baseUrl: "http://example.test",
  auth: { tenant: "tenant" },
  fetch: async () => {
    sent += 1;
    return response();
  },
});
await client.events.watch({ path: { cursor: 250 }, query: {}, headers: {} });
if (sent !== 1) throw new Error("valid request was not sent");
await expectFailure(() => client.events.watch({ path: { cursor: 249 }, query: {}, headers: {} }), "does not match its multiple");
if (sent !== 1) throw new Error("invalid path was sent");
await expectFailure(() => client.events.watch({ path: { cursor: 250 }, query: { limit: 249 }, headers: {} }), "does not match its multiple");
if (sent !== 1) throw new Error("invalid query was sent");
await expectFailure(() => client.events.watch({ path: { cursor: 250 }, query: {}, headers: { "X-Request-ID": 249 } }), "does not match its multiple");
if (sent !== 1) throw new Error("invalid header was sent");
await expectFailure(() => client.auth.login({}), "missing required request body");
if (sent !== 1) throw new Error("missing body was sent");
await client.events.watch({ path: { cursor: 250 }, query: { limit: undefined }, headers: {} });
if (sent !== 2) throw new Error("optional undefined was rejected");
"#;
    run_bun(&optional_files, optional_harness);

    let required = request_input_document(true);
    let required_files = emit_with_strategy(&required, EmitMode::Strict, ErrorStrategy::Throw)
        .unwrap()
        .files;
    let required_harness = r#"
import { createClient } from "./client.ts";

const response = () => new Response("", { status: 200, headers: { "content-type": "text/event-stream", "X-Trace-ID": "trace" } });
const expectFailure = async (operation, expected) => {
  try {
    await operation();
    throw new Error("operation was accepted");
  } catch (cause) {
    const text = JSON.stringify(cause);
    if (!text.includes(expected)) throw new Error(text);
  }
};
let sent = 0;
const client = createClient({
  baseUrl: "http://example.test",
  auth: { tenant: "tenant" },
  fetch: async () => {
    sent += 1;
    return response();
  },
});
await client.events.watch({ path: { cursor: 250 }, query: { limit: 250 }, headers: { "X-Request-ID": 250 } });
if (sent !== 1) throw new Error("valid required request was not sent");
await expectFailure(() => client.events.watch({ path: { cursor: 250 }, query: {}, headers: { "X-Request-ID": 250 } }), "missing required field limit");
if (sent !== 1) throw new Error("missing query was sent");
await expectFailure(() => client.events.watch({ path: { cursor: 250 }, query: { limit: 250 }, headers: {} }), "missing required request header X-Request-ID");
if (sent !== 1) throw new Error("missing header was sent");
await expectFailure(() => client.events.watch({ path: {}, query: { limit: 250 }, headers: { "X-Request-ID": 250 } }), "missing required path parameter cursor");
if (sent !== 1) throw new Error("missing path was sent");
"#;
    run_bun(&required_files, required_harness);
}

#[test]
fn rejects_multipart_media_type_overrides() {
    let mut document: pons_ir::Document =
        serde_json::from_str(include_str!("../../../spec/fixtures/login.json")).unwrap();
    let body = document.operations[0].body.as_mut().unwrap();
    body.content = ContentKind::Multipart;
    body.media = Some(pons_ir::MediaDescriptor::from_declared("multipart/mixed"));
    let error = emit_with_strategy(&document, EmitMode::Strict, ErrorStrategy::Throw).unwrap_err();
    assert!(error.0.contains("multipart media type overrides"));
}

#[test]
fn resets_sse_field_limit_between_events() {
    let document: pons_ir::Document =
        serde_json::from_str(include_str!("../../../spec/fixtures/login.json")).unwrap();
    let files = emit_with_strategy(&document, EmitMode::Strict, ErrorStrategy::Throw)
        .unwrap()
        .files;
    let harness = r#"
import { createClient } from "./client.ts";

const comments = ":x\n".repeat(50_001);
const event = `event: user\ndata: {"id":"1"}\n${comments}\n`;
const body = new ReadableStream({
  start(controller) {
    controller.enqueue(new TextEncoder().encode(event + event));
    controller.close();
  },
});
const client = createClient({
  baseUrl: "https://example.test",
  auth: { tenant: "tenant" },
  fetch: async () => new Response(body, {
    status: 200,
    headers: { "content-type": "text/event-stream", "X-Trace-ID": "trace" },
  }),
});
const stream = await client.events.watch({ path: { cursor: "cursor" }, query: {}, headers: {} });
let count = 0;
for await (const item of stream.value) {
  if (item.isErr()) throw item.error;
  count += 1;
}
if (count !== 2) throw new Error(`expected 2 events, received ${count}`);
"#;
    run_bun(&files, harness);
}

#[test]
fn accepts_bare_json_when_contract_media_type_has_parameters() {
    let mut document: pons_ir::Document =
        serde_json::from_str(include_str!("../../../spec/fixtures/login.json")).unwrap();
    document.operations[0].responses[0].media = Some(pons_ir::MediaDescriptor::from_declared(
        "application/json; charset=utf-8",
    ));
    let files = emit_with_strategy(&document, EmitMode::Strict, ErrorStrategy::Throw)
        .unwrap()
        .files;
    let harness = r#"
import { createClient } from "./client.ts";

const client = createClient({
  baseUrl: "https://example.test",
  fetch: async () => new Response(JSON.stringify({ id: "1" }), {
    status: 200,
    headers: { "content-type": "application/json" },
  }),
});
const value = await client.auth.login({ body: { email: "person@example.test" } });
if (value.id !== "1") throw new Error("bare JSON response was not decoded");
"#;
    run_bun(&files, harness);
}

#[test]
fn maps_unsupported_response_charset_to_protocol_failure() {
    let mut document: pons_ir::Document =
        serde_json::from_str(include_str!("../../../spec/fixtures/login.json")).unwrap();
    let response = &mut document.operations[0].responses[0];
    response.content = ContentKind::Text;
    response.body = Some(TypeRef::Inline(InlineType::String {
        format: None,
        min_length: None,
        max_length: None,
        pattern: None,
    }));
    document.operations[0].provides = None;
    let files = emit_with_strategy(&document, EmitMode::Strict, ErrorStrategy::Throw)
        .unwrap()
        .files;
    let harness = r#"
import { createClient } from "./client.ts";

const client = createClient({
  baseUrl: "https://example.test",
  fetch: async () => new Response("value", {
    status: 200,
    headers: { "content-type": "text/plain; charset=x-unsupported-pons" },
  }),
});
let failure: any;
try {
  await client.auth.login({ body: { email: "person@example.test" } });
} catch (cause) {
  failure = cause;
}
if (failure?._tag !== "DecodeError") throw new Error(`unexpected failure ${JSON.stringify(failure)}`);
if (!String(failure.message).includes("unsupported response charset")) {
  throw new Error(`charset error was lost: ${String(failure.message)}`);
}
"#;
    run_bun(&files, harness);
}

#[test]
fn rejects_request_headers_the_runtime_overwrites() {
    let document: pons_ir::Document =
        serde_json::from_str(include_str!("../../../spec/fixtures/login.json")).unwrap();
    let mut header = document.operations[1].headers[0].clone();
    header.presence = Presence::Required;
    assert!(
        emit_with_strategy(&document, EmitMode::Strict, ErrorStrategy::Throw).is_ok(),
        "a request header the runtime keeps must still emit"
    );
    for wire in ["Content-Type", "authorization"] {
        let mut named = header.clone();
        named.wire = wire.into();
        named.ident = vec![wire.to_ascii_lowercase()];
        let mut refused = document.clone();
        if wire == "authorization" {
            refused.operations[0].headers = Vec::new();
            refused.operations[1].headers = vec![named];
        } else {
            refused.operations[0].headers = vec![named];
        }
        let rejection = emit_with_strategy(&refused, EmitMode::Strict, ErrorStrategy::Throw)
            .expect_err("a request header the runtime overwrites must be rejected");
        assert!(
            rejection.0.contains(wire)
                && rejection
                    .0
                    .contains("is owned by the request body or the security model"),
            "{rejection}"
        );
    }
}

#[test]
fn rejects_request_headers_a_header_credential_overwrites() {
    let mut document: pons_ir::Document =
        serde_json::from_str(include_str!("../../../spec/fixtures/login.json")).unwrap();
    let mut header = document.operations[1].headers[0].clone();
    header.wire = "X-Tenant".into();
    header.ident = vec!["tenant".into()];
    document.operations[1].headers = vec![header];
    for mode in [EmitMode::Strict, EmitMode::Lossy] {
        let rejection = emit_with_strategy(&document, mode, ErrorStrategy::Throw)
            .expect_err("a request header the credential overwrites must be rejected");
        assert!(
            rejection.0.contains("X-Tenant")
                && rejection.0.contains("operations[1].headers")
                && rejection.0.contains("tenant")
                && rejection.0.contains("overwrites"),
            "{}",
            rejection
        );
    }

    let mut cookie_document: pons_ir::Document =
        serde_json::from_str(include_str!("../../../spec/fixtures/login.json")).unwrap();
    let mut cookie_name = cookie_document.operations[1].headers[0].clone();
    cookie_name.wire = "X-Cookie-Key".into();
    cookie_name.ident = vec!["cookie".into(), "key".into()];
    cookie_document.auth_schemes.insert(
        "cookiekey".into(),
        pons_ir::AuthScheme::ApiKey {
            location: ApiKeyLocation::Cookie,
            name: "X-Cookie-Key".into(),
        },
    );
    cookie_document.operations[1].auth.alternatives = vec![vec!["cookiekey".into()]];
    cookie_document.operations[1].headers = vec![cookie_name];
    assert!(
        emit_with_strategy(&cookie_document, EmitMode::Strict, ErrorStrategy::Throw).is_ok(),
        "a cookie credential merges into the cookie header instead of discarding the declared one"
    );
}

#[test]
fn a_catch_all_success_status_still_follows_redirects() {
    let base: pons_ir::Document =
        serde_json::from_str(include_str!("../../../spec/fixtures/login.json")).unwrap();
    let harness = r#"
import { createClient } from "./client.ts";

let hops = 0;
const client = createClient({
  baseUrl: "https://example.test",
  fetch: async (_url, init) => {
    if (init?.redirect !== "manual") throw new Error("redirects are not handled by the client");
    hops += 1;
    if (hops === 1) {
      return new Response(null, { status: 302, headers: { location: "https://example.test/v2/auth/login" } });
    }
    return new Response(JSON.stringify({ id: "1" }), {
      status: 503,
      headers: { "content-type": "application/json" },
    });
  },
});
const value = await client.auth.login({ body: { email: "person@example.test" } });
if (hops !== 2) throw new Error(`the redirect was not followed: ${hops} hops`);
if (value.status !== 503) throw new Error(`the catch-all did not match: ${JSON.stringify(value)}`);
if (value.value.id !== "1") throw new Error(`the body was not decoded: ${JSON.stringify(value)}`);
"#;
    for status in [
        pons_ir::ResponseStatus::Unknown,
        pons_ir::ResponseStatus::Default,
    ] {
        let mut document = base.clone();
        document.operations[0].responses[0].status = status;
        let files = emit_with_strategy(&document, EmitMode::Strict, ErrorStrategy::Throw)
            .unwrap()
            .files;
        run_bun(&files, harness);
    }
}

#[test]
fn a_catch_all_success_status_matches_any_status_a_fixed_one_does_not() {
    let base: pons_ir::Document =
        serde_json::from_str(include_str!("../../../spec/fixtures/login.json")).unwrap();
    let harness = r#"
import { createClient } from "./client.ts";

const unavailable = createClient({
  baseUrl: "https://example.test",
  fetch: async () => new Response(JSON.stringify({ id: "1" }), {
    status: 503,
    headers: { "content-type": "application/json" },
  }),
});
const value = await unavailable.auth.login({ body: { email: "person@example.test" } });
if (value.status !== 503) throw new Error(`the runtime status was not preserved: ${JSON.stringify(value)}`);
if (value.value.id !== "1") throw new Error(`the body was not decoded: ${JSON.stringify(value)}`);
"#;
    for status in [
        pons_ir::ResponseStatus::Unknown,
        pons_ir::ResponseStatus::Default,
    ] {
        let mut document = base.clone();
        document.operations[0].responses[0].status = status;
        let files = emit_with_strategy(&document, EmitMode::Strict, ErrorStrategy::Throw)
            .unwrap()
            .files;
        run_bun(&files, harness);
    }

    let files = emit_with_strategy(&base, EmitMode::Strict, ErrorStrategy::Throw)
        .unwrap()
        .files;
    let harness = r#"
import { createClient } from "./client.ts";

const created = createClient({
  baseUrl: "https://example.test",
  fetch: async () => new Response(JSON.stringify({ id: "1" }), {
    status: 201,
    headers: { "content-type": "application/json" },
  }),
});
try {
  await created.auth.login({ body: { email: "person@example.test" } });
  throw new Error("an undeclared 201 was accepted");
} catch (cause) {
  if (cause?._tag !== "UnexpectedStatusError") throw new Error(JSON.stringify(cause));
  if (cause.status !== 201) {
    throw new Error(`unhelpful failure ${JSON.stringify(cause)}`);
  }
}
"#;
    run_bun(&files, harness);
}

fn catch_all_success_document() -> pons_ir::Document {
    let mut document: pons_ir::Document =
        serde_json::from_str(include_str!("../../../spec/fixtures/login.json")).unwrap();
    document.operations[0].responses[0].status = pons_ir::ResponseStatus::Unknown;
    document
}

#[test]
fn a_declared_error_status_wins_over_a_catch_all_success() {
    let files = emit_with_strategy(
        &catch_all_success_document(),
        EmitMode::Strict,
        ErrorStrategy::Throw,
    )
    .unwrap()
    .files;
    let harness = r#"
import { createClient } from "./client.ts";

const missing = createClient({
  baseUrl: "https://example.test",
  fetch: async () => new Response(JSON.stringify({ id: "1", code: "invalid" }), {
    status: 404,
    headers: { "content-type": "application/json" },
  }),
});
const failure: any = await missing.auth.login({ body: { email: "person@example.test" } })
  .then(() => undefined, (cause) => cause);
if (failure?._tag !== "ApiError") throw new Error(`a declared 4xx produced a success: ${JSON.stringify(failure)}`);
if (failure.status !== 404) throw new Error(`the error status was lost: ${JSON.stringify(failure)}`);
if (failure.body?.code !== "invalid") throw new Error(`the typed error body was lost: ${JSON.stringify(failure)}`);
"#;
    run_bun(&files, harness);
}

#[test]
fn a_catch_all_success_still_matches_a_status_no_error_declares() {
    let files = emit_with_strategy(
        &catch_all_success_document(),
        EmitMode::Strict,
        ErrorStrategy::Throw,
    )
    .unwrap()
    .files;
    let harness = r#"
import { createClient } from "./client.ts";

const unavailable = createClient({
  baseUrl: "https://example.test",
  fetch: async () => new Response(JSON.stringify({ id: "1" }), {
    status: 503,
    headers: { "content-type": "application/json" },
  }),
});
const value = await unavailable.auth.login({ body: { email: "person@example.test" } });
if (value.status !== 503) throw new Error(`the catch-all did not match: ${JSON.stringify(value)}`);
if (value.value.id !== "1") throw new Error(`the body was not decoded: ${JSON.stringify(value)}`);
"#;
    run_bun(&files, harness);
}

#[test]
fn a_default_error_takes_every_status_no_success_declares() {
    let mut document: pons_ir::Document =
        serde_json::from_str(include_str!("../../../spec/fixtures/login.json")).unwrap();
    document.operations[0].responses.clear();
    document.operations[0].errors[0].status = pons_ir::ErrorStatus::Default;
    let files = emit_with_strategy(&document, EmitMode::Strict, ErrorStrategy::Throw)
        .unwrap()
        .files;
    let harness = r#"
import { createClient } from "./client.ts";

const answering = (status: number) => createClient({
  baseUrl: "https://example.test",
  fetch: async () => new Response(JSON.stringify({ code: "invalid" }), {
    status,
    headers: { "content-type": "application/json" },
  }),
});
for (const status of [200, 201, 503]) {
  const failure: any = await answering(status).auth.login({ body: { email: "person@example.test" } })
    .then(() => undefined, (cause) => cause);
  if (failure?._tag !== "ApiError") throw new Error(`${status} was not the default error: ${JSON.stringify(failure)}`);
  if (failure.status !== status) throw new Error(`the error status was lost: ${JSON.stringify(failure)}`);
  if (failure.body?.code !== "invalid") throw new Error(`the typed error body was lost: ${JSON.stringify(failure)}`);
}
"#;
    run_bun(&files, harness);
}

#[test]
fn rejects_an_sse_event_the_contract_refuses() {
    let mut raw: serde_json::Value =
        serde_json::from_str(include_str!("../../../spec/fixtures/login.json")).unwrap();
    raw["operations"][1]["stream"]["events"][0]["data"] =
        serde_json::json!({"kind": "string", "format": "uuid"});
    let document: pons_ir::Document = serde_json::from_value(raw).unwrap();
    let files = validated_files(&document);
    let harness = r#"
import { createClient } from "./client.ts";

const client = createClient({
  baseUrl: "https://example.test",
  auth: { tenant: "tenant" },
  fetch: async () => new Response(
    new ReadableStream({
      start(controller) {
        controller.enqueue(new TextEncoder().encode('event: user\ndata: "not-a-uuid"\n\n'));
        controller.close();
      },
    }),
    { status: 200, headers: { "content-type": "text/event-stream", "X-Trace-ID": "trace" } },
  ),
});
const stream = await client.events.watch({ path: { cursor: "cursor" }, query: {}, headers: {} });
let failure: any;
for await (const item of stream.value) {
  failure = item;
  break;
}
if (!failure?.isErr()) throw new Error("the invalid event was admitted");
if (failure.error._tag !== "StreamDecodeError") throw new Error(`unexpected failure ${JSON.stringify(failure.error)}`);
if (failure.error.event !== "user") throw new Error(`the wrong event was named: ${failure.error.event}`);
"#;
    run_bun(&files, harness);
}

#[test]
fn rejects_an_ndjson_item_the_contract_refuses() {
    let mut raw: serde_json::Value =
        serde_json::from_str(include_str!("../../../spec/fixtures/login.json")).unwrap();
    raw["operations"][1]["stream"] = serde_json::json!({
        "kind": "ndjson",
        "item": {"kind": "string", "format": "uuid"}
    });
    let document: pons_ir::Document = serde_json::from_value(raw).unwrap();
    let files = validated_files(&document);
    let harness = r#"
import { createClient } from "./client.ts";

const client = createClient({
  baseUrl: "https://example.test",
  auth: { tenant: "tenant" },
  fetch: async () => new Response('"not-a-uuid"\n', {
    status: 200,
    headers: { "content-type": "application/x-ndjson", "X-Trace-ID": "trace" },
  }),
});
const stream = await client.events.watch({ path: { cursor: "cursor" }, query: {}, headers: {} });
let failure: any;
for await (const item of stream.value) {
  failure = item;
  break;
}
if (!failure?.isErr()) throw new Error("the invalid item was admitted");
if (failure.error._tag !== "StreamDecodeError") throw new Error(`unexpected failure ${JSON.stringify(failure.error)}`);
"#;
    run_bun(&files, harness);
}
