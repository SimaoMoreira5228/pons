mod support;

use pons_emit_ts::{EmitMode, ErrorStrategy, emit_with_strategy};
use support::runtime::run_bun;

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
