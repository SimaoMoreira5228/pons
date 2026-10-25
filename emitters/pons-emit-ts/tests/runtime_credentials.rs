mod support;

use std::collections::BTreeMap;

use pons_emit_ts::{EmitMode, ErrorStrategy, emit_with_strategy};
use pons_ir::{ApiKeyLocation, CredentialLocator, InlineType, Presence, TypeRef};
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
fn follows_same_origin_redirects_before_decoding() {
    let document: pons_ir::Document =
        serde_json::from_str(include_str!("../../../spec/fixtures/login.json")).unwrap();
    let files = emit_with_strategy(&document, EmitMode::Strict, ErrorStrategy::Throw)
        .unwrap()
        .files;
    let harness = r#"
import { createClient } from "./client.ts";

let calls = 0;
const client = createClient({
  baseUrl: "https://example.test",
  fetch: async () => {
    calls += 1;
    if (calls === 1) return new Response(null, { status: 302, headers: { location: "/auth/final" } });
    return new Response(JSON.stringify({ id: "1" }), {
      status: 200,
      headers: { "content-type": "application/json" },
    });
  },
});
const result = await client.auth.login({ body: { email: "person@example.test" } });
if (calls !== 2) throw new Error(`unexpected calls ${calls}`);
if (result.id !== "1") throw new Error(`wrong result ${JSON.stringify(result)}`);
"#;
    run_bun(&files, harness);
}

#[test]
fn reattaches_query_credentials_across_same_origin_redirects() {
    let mut document: pons_ir::Document =
        serde_json::from_str(include_str!("../../../spec/fixtures/login.json")).unwrap();
    document.auth_schemes.insert(
        "queryKey".into(),
        pons_ir::AuthScheme::ApiKey {
            location: ApiKeyLocation::Query,
            name: "api_key".into(),
        },
    );
    document.auth_schemes.insert(
        "adapter".into(),
        pons_ir::AuthScheme::Custom {
            description: "query adapter".into(),
            adapter: None,
            ext: Default::default(),
        },
    );
    document.operations[1].auth.alternatives = vec![vec!["queryKey".into(), "adapter".into()]];
    let files = emit_with_strategy(&document, EmitMode::Strict, ErrorStrategy::Throw)
        .unwrap()
        .files;
    let harness = r#"
import { createClient } from "./client.ts";

const requested: string[] = [];
let calls = 0;
const client = createClient({
  baseUrl: "https://example.test",
  auth: { queryKey: "secret" },
  authAdapters: {
    adapter: () => ({ query: { adapter_token: "attached" } }),
  },
  fetch: async (url) => {
    calls += 1;
    requested.push(`${url.pathname}?${url.searchParams.toString()}`);
    if (calls === 1) return new Response(null, { status: 302, headers: { location: "/events/redirected" } });
    return new Response(new ReadableStream({ start(controller) { controller.close(); } }), {
      status: 200,
      headers: { "content-type": "text/event-stream", "X-Trace-ID": "trace" },
    });
  },
});
await client.events.watch({ path: { cursor: "cursor" }, query: {}, headers: {} });
if (calls !== 2) throw new Error(`unexpected calls ${calls}`);
if (!requested[0].includes("api_key=secret") || !requested[0].includes("adapter_token=attached")) {
  throw new Error(`first request lost its query credentials: ${requested[0]}`);
}
if (!requested[1].includes("api_key=secret")) throw new Error(`redirect lost the api key: ${requested[1]}`);
if (!requested[1].includes("adapter_token=attached")) {
  throw new Error(`redirect lost the adapter query credential: ${requested[1]}`);
}
"#;
    run_bun(&files, harness);
}

#[test]
fn rejects_cross_origin_redirects_with_required_credentials() {
    let mut document: pons_ir::Document =
        serde_json::from_str(include_str!("../../../spec/fixtures/login.json")).unwrap();
    document.operations[1].auth.alternatives = vec![vec!["session".into()]];
    let files = emit_with_strategy(&document, EmitMode::Strict, ErrorStrategy::Throw)
        .unwrap()
        .files;
    let harness = r#"
import { createClient } from "./client.ts";

let calls = 0;
const client = createClient({
  baseUrl: "https://a.test",
  auth: { session: "token" },
  fetch: async () => {
    calls += 1;
    return new Response(null, { status: 302, headers: { location: "https://b.test/events" } });
  },
});
let failure: any;
try {
  await client.events.watch({ path: { cursor: "cursor" }, query: {}, headers: {} });
} catch (cause) {
  failure = cause;
}
if (failure?._tag !== "CredentialRedirectError") throw new Error(`unexpected failure ${JSON.stringify(failure)}`);
if (calls !== 1) throw new Error("credential reached the redirect target");
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
fn falls_back_to_a_later_auth_alternative() {
    let mut document: pons_ir::Document =
        serde_json::from_str(include_str!("../../../spec/fixtures/login.json")).unwrap();
    document
        .auth_schemes
        .insert("empty".into(), pons_ir::AuthScheme::Bearer { format: None });
    document.operations[1].auth.alternatives = vec![vec!["empty".into()], vec!["tenant".into()]];

    let files = emit_with_strategy(&document, EmitMode::Strict, ErrorStrategy::Throw)
        .unwrap()
        .files;
    let harness = r#"
import { createClient } from "./client.ts";

let emptyReads = 0;
const client = createClient({
  baseUrl: "http://example.test",
  auth: {
    empty: () => {
      emptyReads += 1;
      return undefined;
    },
    tenant: "tenant",
  },
  fetch: async (_url, init) => {
    const headers = new Headers(init?.headers);
    if (headers.get("authorization") !== null) throw new Error("empty alternative was attached");
    if (headers.get("X-Tenant") !== "tenant") throw new Error("fallback alternative was not attached");
    return new Response(new ReadableStream({ start(controller) { controller.close(); } }), {
      status: 200,
      headers: {
        "content-type": "text/event-stream",
        "X-Trace-ID": "trace",
      },
    });
  },
});
await client.events.watch({ path: { cursor: "cursor" }, query: {}, headers: {} });
    if (emptyReads !== 1) throw new Error(`unexpected empty source reads: ${emptyReads}`);
    "#;
    run_bun(&files, harness);
}

#[test]
fn falls_back_from_an_empty_cookie_source() {
    let mut document: pons_ir::Document =
        serde_json::from_str(include_str!("../../../spec/fixtures/login.json")).unwrap();
    document.auth_schemes.insert(
        "session".into(),
        pons_ir::AuthScheme::CookieSession {
            cookie: "session".into(),
        },
    );
    document.operations[1].auth.alternatives = vec![vec!["session".into()], vec!["tenant".into()]];

    let files = emit_with_strategy(&document, EmitMode::Strict, ErrorStrategy::Throw)
        .unwrap()
        .files;
    let harness = r#"
import { createClient } from "./client.ts";

const client = createClient({
  baseUrl: "http://example.test",
  auth: {
    session: () => undefined,
    tenant: "tenant",
  },
  fetch: async (_url, init) => {
    const headers = new Headers(init?.headers);
    if (headers.get("cookie") !== null) throw new Error("empty cookie alternative was attached");
    if (headers.get("X-Tenant") !== "tenant") throw new Error("fallback cookie alternative was not selected");
    return new Response(new ReadableStream({ start(controller) { controller.close(); } }), {
      status: 200,
      headers: {
        "content-type": "text/event-stream",
        "X-Trace-ID": "trace",
      },
    });
  },
});
await client.events.watch({ path: { cursor: "cursor" }, query: {}, headers: {} });
"#;
    run_bun(&files, harness);
}

#[test]
fn rejects_inherited_object_properties_as_credentials() {
    let mut document: pons_ir::Document =
        serde_json::from_str(include_str!("../../../spec/fixtures/login.json")).unwrap();
    document.auth_schemes.insert(
        "constructor".into(),
        pons_ir::AuthScheme::Bearer { format: None },
    );
    document.operations[1].auth.alternatives = vec![vec!["constructor".into()]];

    let files = emit_with_strategy(&document, EmitMode::Strict, ErrorStrategy::Throw)
        .unwrap()
        .files;
    let harness = r#"
import { createClient } from "./client.ts";

let fetched = false;
const client = createClient({
  baseUrl: "http://example.test",
  fetch: async () => {
    fetched = true;
    throw new Error("request should not be sent");
  },
});
let failure: any;
try {
  await client.events.watch({ path: { cursor: "cursor" }, query: {}, headers: {} });
} catch (cause) {
  failure = cause;
}
if (failure?._tag !== "AuthUnavailableError") throw new Error(`unexpected failure ${JSON.stringify(failure)}`);
if (fetched) throw new Error("missing credential reached fetch");
"#;
    run_bun(&files, harness);
}

#[test]
fn does_not_inherit_custom_auth_scopes() {
    let mut document: pons_ir::Document =
        serde_json::from_str(include_str!("../../../spec/fixtures/login.json")).unwrap();
    document.auth_schemes.insert(
        "toString".into(),
        pons_ir::AuthScheme::Custom {
            description: "scope probe".into(),
            adapter: None,
            ext: Default::default(),
        },
    );
    document.operations[1].auth.alternatives = vec![vec!["toString".into()]];

    let files = emit_with_strategy(&document, EmitMode::Strict, ErrorStrategy::Throw)
        .unwrap()
        .files;
    let harness = r#"
import { createClient } from "./client.ts";

const client = createClient({
  baseUrl: "https://example.test",
  authAdapters: {
    toString: ({ scopes }) => {
      if (scopes.length !== 0) throw new Error(`inherited scopes: ${scopes.join(",")}`);
      return {};
    },
  },
  fetch: async () => new Response(new ReadableStream({ start(controller) { controller.close(); } }), {
    status: 200,
    headers: {
      "content-type": "text/event-stream",
      "X-Trace-ID": "trace",
    },
  }),
});
await client.events.watch({ path: { cursor: "cursor" }, query: {}, headers: {} });
"#;
    run_bun(&files, harness);
}

#[test]
fn a_custom_scheme_is_served_by_the_adapter_id_it_declares() {
    let mut document: pons_ir::Document =
        serde_json::from_str(include_str!("../../../spec/fixtures/login.json")).unwrap();
    document.auth_schemes.insert(
        "tenant".into(),
        pons_ir::AuthScheme::Custom {
            description: "tenant adapter".into(),
            adapter: Some("tenantSignature".into()),
            ext: Default::default(),
        },
    );
    document.operations[1].auth.alternatives = vec![vec!["tenant".into()]];

    let files = emit_with_strategy(&document, EmitMode::Strict, ErrorStrategy::Throw)
        .unwrap()
        .files;
    let client = files.iter().find(|file| file.path == "client.ts").unwrap();
    assert!(
        client.content.contains(
            "authSchemes[\"tenant\"] = { kind: \"custom\", adapter: \"tenantSignature\" };"
        ),
        "{}",
        client.content
    );
    let harness = r#"
import { createClient } from "./client.ts";

const stream = () => new Response(new ReadableStream({ start(controller) { controller.close(); } }), {
  status: 200,
  headers: { "content-type": "text/event-stream", "X-Trace-ID": "trace" },
});

const byScheme = createClient({
  baseUrl: "https://example.test",
  authAdapters: { tenant: () => ({ headers: { "x-tenant": "signed" } }) },
  fetch: async () => stream(),
});
let failure: any;
try {
  await byScheme.events.watch({ path: { cursor: "cursor" }, query: {}, headers: {} });
} catch (cause) {
  failure = cause;
}
if (failure?._tag !== "AuthUnavailableError") throw new Error(`the scheme id served the adapter: ${JSON.stringify(failure)}`);

const declared: string[] = [];
const byAdapter = createClient({
  baseUrl: "https://example.test",
  authAdapters: {
    tenantSignature: ({ scheme }) => {
      declared.push(scheme);
      return { headers: { "x-tenant": "signed" } };
    },
  },
  fetch: async (_url, init) => {
    const headers = new Headers(init?.headers);
    if (headers.get("x-tenant") !== "signed") throw new Error("the declared adapter did not attach its header");
    return stream();
  },
});
await byAdapter.events.watch({ path: { cursor: "cursor" }, query: {}, headers: {} });
if (declared.join(",") !== "tenant") throw new Error(`adapter context named ${declared.join(",")}`);
"#;
    run_bun(&files, harness);
}

#[test]
fn preserves_proto_auth_scheme_and_custom_adapter_scopes() {
    let mut document: pons_ir::Document =
        serde_json::from_str(include_str!("../../../spec/fixtures/login.json")).unwrap();
    document.auth_schemes.insert(
        "__proto__".into(),
        pons_ir::AuthScheme::Custom {
            description: "prototype adapter".into(),
            adapter: None,
            ext: Default::default(),
        },
    );
    let operation = &mut document.operations[1];
    operation.auth.alternatives = vec![vec!["__proto__".into()]];
    operation
        .auth
        .scopes
        .insert("__proto__".into(), vec!["events:read".into()]);

    let files = emit_with_strategy(&document, EmitMode::Strict, ErrorStrategy::Throw)
        .unwrap()
        .files;
    let harness = r#"
import { createClient } from "./client.ts";

const client = createClient({
  baseUrl: "https://example.test",
  authAdapters: {
    ["__proto__"]: ({ scheme, scopes }) => {
      if (scheme !== "__proto__" || scopes.join(",") !== "events:read") {
        throw new Error(`wrong prototype auth context: ${scheme}/${scopes.join(",")}`);
      }
      return { headers: { "X-Auth": "present" } };
    },
  },
  fetch: async (_url, init) => {
    if (new Headers(init?.headers).get("X-Auth") !== "present") {
      throw new Error("prototype auth adapter was not called");
    }
    return new Response(new ReadableStream({ start(controller) { controller.close(); } }), {
      status: 200,
      headers: {
        "content-type": "text/event-stream",
        "X-Trace-ID": "trace",
      },
    });
  },
});
await client.events.watch({ path: { cursor: "cursor" }, query: {}, headers: {} });
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
fn revalidates_custom_auth_origin_after_transport_request_hooks() {
    let mut document: pons_ir::Document =
        serde_json::from_str(include_str!("../../../spec/fixtures/login.json")).unwrap();
    document.auth_schemes.insert(
        "adapter".into(),
        pons_ir::AuthScheme::Custom {
            description: "origin probe".into(),
            adapter: None,
            ext: Default::default(),
        },
    );
    document.operations[1].auth.alternatives = vec![vec!["adapter".into()]];

    let files = emit_with_strategy(&document, EmitMode::Strict, ErrorStrategy::Throw)
        .unwrap()
        .files;
    let harness = r#"
import { createClient } from "./client.ts";

let fetched = false;
const client = createClient({
  baseUrl: "https://example.test",
  authAdapters: {
    adapter: async () => ({ headers: { "X-Secret": "secret" } }),
  },
  transport: {
    onRequest: async ({ url }) => {
      if (url.pathname === "/auth/login") return;
      url.hostname = "idp.test";
    },
  },
  fetch: async () => {
    fetched = true;
    throw new Error("custom credential reached a rewritten host");
  },
});
let failure: any;
try {
  await client.events.watch({ path: { cursor: "cursor" }, query: {}, headers: {} });
} catch (cause) {
  failure = cause;
}
if (failure?._tag !== "AuthUnavailableError") throw new Error(`unexpected failure ${JSON.stringify(failure)}`);
if (fetched) throw new Error("custom credential reached a rewritten host");
"#;
    run_bun(&files, harness);
}

#[test]
fn drops_refreshed_custom_query_credentials_across_redirects() {
    let mut document: pons_ir::Document =
        serde_json::from_str(include_str!("../../../spec/fixtures/login.json")).unwrap();
    document.auth_schemes.insert(
        "adapter".into(),
        pons_ir::AuthScheme::Custom {
            description: "query adapter".into(),
            adapter: None,
            ext: Default::default(),
        },
    );
    document.operations[1].auth.alternatives = vec![vec!["adapter".into()]];
    document.operations[1].provides = Some(pons_ir::CredentialProvision {
        scheme: "adapter".into(),
        value: CredentialLocator::Header {
            name: "X-Token".into(),
        },
        expires_at: None,
        ext: Default::default(),
    });
    let files = emit_with_strategy(&document, EmitMode::Strict, ErrorStrategy::Throw)
        .unwrap()
        .files;
    let harness = r#"
import { createClient } from "./client.ts";

const requested: string[] = [];
let calls = 0;
let attachments = 0;
const client = createClient({
  baseUrl: "https://example.test",
  authAdapters: {
    adapter: () => {
      attachments += 1;
      return attachments === 1 ? { query: { adapter_token: "first" } } : {};
    },
  },
  fetch: async (url) => {
    calls += 1;
    requested.push(url.searchParams.get("adapter_token") ?? "none");
    if (calls < 3) {
      return new Response(null, { status: 302, headers: { location: `/events/hop${calls}`, "X-Token": "fresh" } });
    }
    return new Response(new ReadableStream({ start(controller) { controller.close(); } }), {
      status: 200,
      headers: { "content-type": "text/event-stream", "X-Trace-ID": "trace" },
    });
  },
});
await client.events.watch({ path: { cursor: "cursor" }, query: {}, headers: {} });
if (requested.join(",") !== "first,none,none") throw new Error(`stale credential was resurrected: ${requested.join(",")}`);
"#;
    run_bun(&files, harness);
}

#[test]
fn follows_cross_origin_redirects_that_carry_no_credentials() {
    let mut document: pons_ir::Document =
        serde_json::from_str(include_str!("../../../spec/fixtures/login.json")).unwrap();
    document.operations[1].auth.alternatives = Vec::new();
    document.operations[1].headers = vec![pons_ir::HeaderParameter {
        wire: "X-Accept".into(),
        ident: vec!["accept".into()],
        ty: TypeRef::Inline(InlineType::String {
            format: None,
            min_length: None,
            max_length: None,
            pattern: None,
        }),
        presence: Presence::Required,
        ext: BTreeMap::new(),
    }];
    let files = emit_with_strategy(&document, EmitMode::Strict, ErrorStrategy::Throw)
        .unwrap()
        .files;
    let harness = r#"
import { createClient } from "./client.ts";

const requested: string[] = [];
let calls = 0;
const client = createClient({
  baseUrl: "https://a.test",
  fetch: async (url, init) => {
    calls += 1;
    requested.push(`${url.origin}${url.pathname} ${new Headers(init?.headers).get("x-accept")}`);
    if (calls === 1) return new Response(null, { status: 302, headers: { location: "https://b.test/events/redirected" } });
    return new Response(new ReadableStream({ start(controller) { controller.close(); } }), {
      status: 200,
      headers: { "content-type": "text/event-stream", "X-Trace-ID": "trace" },
    });
  },
});
await client.events.watch({ path: { cursor: "cursor" }, query: {}, headers: { "X-Accept": "application/json" } });
if (calls !== 2) throw new Error(`cross-origin redirect was refused: ${JSON.stringify(requested)}`);
if (!requested[1].startsWith("https://b.test/events/redirected application/json")) {
  throw new Error(`wrong redirect target: ${requested[1]}`);
}
"#;
    run_bun(&files, harness);
}

#[test]
fn refuses_cross_origin_redirects_without_capturing_their_credentials() {
    let document: pons_ir::Document =
        serde_json::from_str(include_str!("../../../spec/fixtures/login.json")).unwrap();
    let files = emit_with_strategy(&document, EmitMode::Strict, ErrorStrategy::Throw)
        .unwrap()
        .files;
    let harness = r#"
import { createClient } from "./client.ts";

let stored = 0;
let calls = 0;
const client = createClient({
  baseUrl: "https://a.test",
  auth: {
    session: {
      get: async () => undefined,
      set: async () => {
        stored += 1;
      },
      clear: async () => {},
    },
  },
  fetch: async () => {
    calls += 1;
    if (calls === 1) {
      return new Response(JSON.stringify({ token: "captured", id: "1" }), {
        status: 307,
        headers: { "content-type": "application/json", location: "https://b.test/auth/login" },
      });
    }
    return new Response(JSON.stringify({ id: "1" }), {
      status: 200,
      headers: { "content-type": "application/json" },
    });
  },
});
let failure: any;
try {
  await client.auth.login({ body: { email: "person@example.test" } });
} catch (cause) {
  failure = cause;
}
if (failure?._tag !== "RedirectError") throw new Error(`unexpected failure ${JSON.stringify(failure)}`);
if (calls !== 1) throw new Error("a refused redirect was followed");
if (stored !== 0) throw new Error(`a refused redirect stored ${stored} credentials`);
"#;
    run_bun(&files, harness);
}

#[test]
fn does_not_capture_a_body_credential_from_an_allowed_cross_origin_redirect() {
    let document: pons_ir::Document =
        serde_json::from_str(include_str!("../../../spec/fixtures/login.json")).unwrap();
    let files = emit_with_strategy(&document, EmitMode::Strict, ErrorStrategy::Throw)
        .unwrap()
        .files;
    let harness = r#"
import { createClient } from "./client.ts";

const stored: string[] = [];
let calls = 0;
const client = createClient({
  baseUrl: "https://a.test",
  auth: {
    session: {
      get: async () => undefined,
      set: async (value) => { stored.push(String(value)); },
      clear: async () => {},
    },
  },
  fetch: async (_url, init) => {
    calls += 1;
    if (calls === 1) {
      return new Response(JSON.stringify({ token: "cross-origin", id: "1" }), {
        status: 303,
        headers: { "content-type": "application/json", location: "https://b.test/auth/login" },
      });
    }
    if (new Headers(init?.headers).has("authorization")) throw new Error("the redirect was authenticated");
    return new Response(JSON.stringify({ id: "1" }), {
      status: 200,
      headers: { "content-type": "application/json" },
    });
  },
});
const user = await client.auth.login({ body: { email: "person@example.test" } });
if (calls !== 2) throw new Error(`the allowed redirect was not followed: ${calls}`);
if (stored.length !== 0) throw new Error(`a cross-origin redirect stored ${JSON.stringify(stored)}`);
if (user.id !== "1") throw new Error(`unexpected login result ${JSON.stringify(user)}`);
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
