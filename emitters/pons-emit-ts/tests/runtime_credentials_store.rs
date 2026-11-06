mod support;

use std::collections::BTreeMap;

use pons_emit_ts::{EmitMode, ErrorStrategy, emit_with_strategy};
use pons_ir::CredentialLocator;
use support::runtime::run_bun;

#[test]
fn refreshes_expired_cached_credentials() {
    let mut document: pons_ir::Document =
        serde_json::from_str(include_str!("../../../spec/fixtures/login.json")).unwrap();
    let operation = &mut document.operations[0];
    operation.auth.alternatives = vec![vec!["session".into()]];
    let provides = operation.provides.as_mut().unwrap();
    provides.value = CredentialLocator::Header {
        name: "X-Token".into(),
    };
    provides.expires_at = Some(CredentialLocator::Header {
        name: "X-Expires".into(),
    });

    let files = emit_with_strategy(&document, EmitMode::Strict, ErrorStrategy::Throw)
        .unwrap()
        .files;
    let harness = r#"
import { createClient } from "./client.ts";

const sent: string[] = [];
let gets = 0;
let clears = 0;
const store = {
  get: async () => {
    gets += 1;
     return { value: gets === 1 ? "initial" : "fresh" };
  },
  set: async () => {},
  clear: async () => {
    clears += 1;
  },
};
const client = createClient({
  baseUrl: "http://example.test",
  auth: { session: store },
  fetch: async (_url, init) => {
    sent.push(init.headers.get("authorization") ?? "");
    return new Response(JSON.stringify({ id: "1" }), {
      status: 200,
      headers: {
        "content-type": "application/json",
        "X-Token": "expired",
        "X-Expires": "2000-01-01T00:00:00Z",
      },
    });
  },
});
await client.auth.login({ body: { email: "person@example.test" } });
await client.auth.login({ body: { email: "person@example.test" } });
if (sent[0] !== "Bearer initial" || sent[1] !== "Bearer fresh") throw new Error(`unexpected credentials: ${sent.join(",")}`);
if (gets !== 2 || clears !== 1) throw new Error(`unexpected source calls: ${gets},${clears}`);
"#;
    run_bun(&files, harness);
}

#[test]
fn scopes_captured_non_cookie_credentials_by_origin() {
    let mut document: pons_ir::Document =
        serde_json::from_str(include_str!("../../../spec/fixtures/login.json")).unwrap();
    document.operations[0].provides.as_mut().unwrap().value = CredentialLocator::Header {
        name: "X-Token".into(),
    };
    document.operations[1].auth.alternatives = vec![vec!["session".into()]];
    let files = emit_with_strategy(&document, EmitMode::Strict, ErrorStrategy::Throw)
        .unwrap()
        .files;
    let harness = r#"
import { createClient } from "./client.ts";

let stored: any;
let fetched = false;
const store = {
  get: async () => stored,
  set: async (value: string, metadata: any) => { stored = { value, ...metadata }; },
  clear: async () => { stored = undefined; },
};
const first = createClient({
  baseUrl: "https://example.test:8443",
  auth: { session: store },
  fetch: async () => new Response(JSON.stringify({ id: "1" }), {
    status: 200,
    headers: { "content-type": "application/json", "X-Token": "secret" },
  }),
});
await first.auth.login({ body: { email: "person@example.test" } });
const second = createClient({
  baseUrl: "https://example.test:9443",
  auth: { session: store },
  fetch: async () => {
    fetched = true;
    return new Response(new ReadableStream({ start(controller) { controller.close(); } }), {
      status: 200,
      headers: { "content-type": "text/event-stream", "X-Trace-ID": "trace" },
    });
  },
});
let failure: any;
try {
  await second.events.watch({ path: { cursor: "cursor" }, query: {}, headers: {} });
} catch (cause) {
  failure = cause;
}
if (failure?._tag !== "AuthUnavailableError") throw new Error(`unexpected failure ${JSON.stringify(failure)}`);
if (fetched) throw new Error("credential reached another origin");
"#;
    run_bun(&files, harness);
}

#[test]
fn rejects_legacy_token_store_records() {
    let mut document: pons_ir::Document =
        serde_json::from_str(include_str!("../../../spec/fixtures/login.json")).unwrap();
    let provides = document.operations[0].provides.as_mut().unwrap();
    provides.value = CredentialLocator::Header {
        name: "X-Token".into(),
    };
    provides.expires_at = Some(CredentialLocator::Header {
        name: "X-Expires".into(),
    });
    document.operations[1].auth.alternatives = vec![vec!["session".into()]];

    let files = emit_with_strategy(&document, EmitMode::Strict, ErrorStrategy::Throw)
        .unwrap()
        .files;
    let harness = r#"
import { createClient } from "./client.ts";

let fetched: string[] = [];
const watch = async (store: any) => {
  const client = createClient({
    baseUrl: "https://example.test",
    auth: { session: store },
    fetch: async (_url, init) => {
      fetched.push(new Headers(init?.headers).get("authorization") ?? "");
      return new Response(new ReadableStream({ start(controller) { controller.close(); } }), {
        status: 200,
        headers: { "content-type": "text/event-stream", "X-Trace-ID": "trace" },
      });
    },
  });
  let failure: any;
  try {
    await client.events.watch({ path: { cursor: "cursor" }, query: {}, headers: {} });
  } catch (cause) {
    failure = cause;
  }
  return failure;
};
const expectSourceFailure = (failure: any, message: string) => {
  if (failure?._tag !== "AuthSourceError") throw new Error(`unexpected failure ${JSON.stringify(failure)}`);
  if (failure?.cause?.message !== message) throw new Error(`unexpected cause ${failure?.cause?.message}`);
};

let legacy: any;
const store = {
  get: async () => legacy,
  set: async (value: string, expiresAt: any) => { legacy = { value, expiresAt }; },
  clear: async () => { legacy = undefined; },
};
const writer = createClient({
  baseUrl: "https://example.test",
  auth: { session: store },
  fetch: async () => new Response(JSON.stringify({ id: "1" }), {
    status: 200,
    headers: {
      "content-type": "application/json",
      "X-Token": "captured",
      "X-Expires": new Date(Date.now() + 3_600_000).toISOString(),
    },
  }),
});
await writer.auth.login({ body: { email: "person@example.test" } });
expectSourceFailure(await watch(store), "token store record expiresAt must be a number");
expectSourceFailure(
  await watch({ get: async () => "legacy", set: async () => {}, clear: async () => {} }),
  "token store must return a credential record, not a string",
);
expectSourceFailure(await watch({ get: async () => ({}), set: async () => {}, clear: async () => {} }), "token store record must have a string value");
expectSourceFailure(
  await watch({ get: async () => ({ value: "x", expiresAt: "2000-01-01T00:00:00Z" }), set: async () => {}, clear: async () => {} }),
  "token store record expiresAt must be a number",
);
if (fetched.length !== 0) throw new Error(`rejected credential reached fetch: ${fetched.join(",")}`);
const accepted = await watch({ get: async () => ({ value: "kept" }), set: async () => {}, clear: async () => {} });
if (accepted !== undefined) throw new Error(`valid record was rejected: ${JSON.stringify(accepted)}`);
if (fetched[0] !== "Bearer kept") throw new Error(`unexpected credential ${fetched[0]}`);
"#;
    run_bun(&files, harness);
}

#[test]
fn revalidates_captured_bearer_origin_after_transport_request_hooks() {
    let mut document: pons_ir::Document =
        serde_json::from_str(include_str!("../../../spec/fixtures/login.json")).unwrap();
    document.operations[0].provides.as_mut().unwrap().value = CredentialLocator::Header {
        name: "X-Token".into(),
    };
    document.operations[1].auth.alternatives = vec![vec!["session".into()]];

    let files = emit_with_strategy(&document, EmitMode::Strict, ErrorStrategy::Throw)
        .unwrap()
        .files;
    let harness = r#"
import { createClient } from "./client.ts";

let calls = 0;
const client = createClient({
  baseUrl: "https://example.test",
  transport: {
    onRequest: async ({ url }) => {
      if (url.pathname === "/auth/login") return;
      url.hostname = "idp.test";
    },
  },
  fetch: async () => {
    calls += 1;
    if (calls === 1) {
      return new Response(JSON.stringify({ id: "1" }), {
        status: 200,
        headers: {
          "content-type": "application/json",
          "X-Token": "secret",
        },
      });
    }
    throw new Error("captured bearer reached a rewritten host");
  },
});
await client.auth.login({ body: { email: "person@example.test" } });
let failure: any;
try {
  await client.events.watch({ path: { cursor: "cursor" }, query: {}, headers: {} });
} catch (cause) {
  failure = cause;
}
if (failure?._tag !== "AuthUnavailableError") throw new Error(`unexpected failure ${JSON.stringify(failure)}`);
if (calls !== 1) throw new Error("captured bearer reached a rewritten host");
"#;
    run_bun(&files, harness);
}

#[test]
fn reports_a_body_credential_locator_on_a_stream_as_an_auth_source_failure() {
    let mut document: pons_ir::Document =
        serde_json::from_str(include_str!("../../../spec/fixtures/login.json")).unwrap();
    document.operations[1].auth.alternatives = Vec::new();
    document.operations[1].provides = Some(pons_ir::CredentialProvision {
        scheme: "session".into(),
        value: CredentialLocator::Body {
            pointer: "/token".into(),
        },
        expires_at: None,
        ext: BTreeMap::new(),
    });
    let files = emit_with_strategy(&document, EmitMode::Strict, ErrorStrategy::Throw)
        .unwrap()
        .files;
    let harness = r#"
import { createClient } from "./client.ts";

const client = createClient({
  baseUrl: "https://example.test",
  fetch: async () => new Response(new ReadableStream({ start(controller) { controller.close(); } }), {
    status: 200,
    headers: { "content-type": "text/event-stream", "X-Trace-ID": "trace" },
  }),
});
let failure: any;
try {
  await client.events.watch({ path: { cursor: "cursor" }, query: {}, headers: {} });
} catch (cause) {
  failure = cause;
}
if (failure?._tag !== "AuthSourceError") throw new Error(`unexpected failure ${JSON.stringify(failure)}`);
if (failure.scheme !== "session") throw new Error(`unexpected scheme ${JSON.stringify(failure)}`);
if (String(failure.cause?.message).includes("stream") === false) {
  throw new Error(`unhelpful cause ${JSON.stringify(failure.cause)}`);
}
"#;
    run_bun(&files, harness);
}
