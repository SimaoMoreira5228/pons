mod support;

use std::collections::BTreeMap;

use pons_emit_ts::{EmitMode, ErrorStrategy, emit_with_strategy};
use pons_ir::{ApiKeyLocation, CredentialLocator, PathSegment};
use support::runtime::run_bun;

#[test]
fn revalidates_cookie_api_key_origin_after_transport_request_hooks() {
    let mut document: pons_ir::Document =
        serde_json::from_str(include_str!("../../../spec/fixtures/login.json")).unwrap();
    document.auth_schemes.insert(
        "api".into(),
        pons_ir::AuthScheme::ApiKey {
            location: ApiKeyLocation::Cookie,
            name: "__Secure-api".into(),
        },
    );
    document.operations[0].provides.as_mut().unwrap().scheme = "api".into();
    document.operations[0].provides.as_mut().unwrap().value = CredentialLocator::SetCookie {
        name: "session".into(),
    };
    document.operations[1].auth.alternatives = vec![vec!["api".into()]];

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
          "set-cookie": "session=secret; Path=/; Secure",
        },
      });
    }
    throw new Error("cookie API key reached a rewritten host");
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
if (calls !== 1) throw new Error("cookie API key reached a rewritten host");
"#;
    run_bun(&files, harness);
}

#[test]
fn revalidates_cookie_scope_after_transport_request_hooks() {
    let mut document: pons_ir::Document =
        serde_json::from_str(include_str!("../../../spec/fixtures/login.json")).unwrap();
    document.auth_schemes.insert(
        "session".into(),
        pons_ir::AuthScheme::CookieSession {
            cookie: "session".into(),
        },
    );
    document.operations[0].provides.as_mut().unwrap().value = CredentialLocator::SetCookie {
        name: "session".into(),
    };
    document.operations[1].auth.alternatives = vec![vec!["session".into()]];
    document.operations[1].path = vec![
        PathSegment::Literal { lit: "auth".into() },
        PathSegment::Literal {
            lit: "refresh".into(),
        },
    ];

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
      url.protocol = "http:";
      url.pathname = "/events/watch";
    },
  },
  fetch: async () => {
    calls += 1;
    if (calls > 1) throw new Error("request should not be sent");
    return new Response(JSON.stringify({ id: "1" }), {
      status: 200,
      headers: {
        "content-type": "application/json",
        "set-cookie": "session=secret; Path=/auth; Secure",
      },
    });
  },
});
let failure: any;
try {
  await client.auth.login({ body: { email: "person@example.test" } });
  await client.events.watch({ query: {}, headers: {} });
} catch (cause) {
  failure = cause;
}
if (failure?._tag !== "AuthUnavailableError") throw new Error(`unexpected failure ${JSON.stringify(failure)}`);
if (calls !== 1) throw new Error("rewritten request reached fetch");
"#;
    run_bun(&files, harness);
}

#[test]
fn revalidates_cookie_origin_after_transport_request_hooks() {
    let mut document: pons_ir::Document =
        serde_json::from_str(include_str!("../../../spec/fixtures/login.json")).unwrap();
    document.auth_schemes.insert(
        "session".into(),
        pons_ir::AuthScheme::CookieSession {
            cookie: "session".into(),
        },
    );
    document.operations[0].provides.as_mut().unwrap().value = CredentialLocator::SetCookie {
        name: "session".into(),
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
    if (calls > 1) throw new Error("cross-origin request reached fetch");
    return new Response(JSON.stringify({ id: "1" }), {
      status: 200,
      headers: {
        "content-type": "application/json",
        "set-cookie": "session=secret; Path=/; Secure",
      },
    });
  },
});
let failure: any;
try {
  await client.auth.login({ body: { email: "person@example.test" } });
  await client.events.watch({ path: { cursor: "cursor" }, query: {}, headers: {} });
} catch (cause) {
  failure = cause;
}
if (failure?._tag !== "AuthUnavailableError") throw new Error(`unexpected failure ${JSON.stringify(failure)}`);
if (calls !== 1) throw new Error("cross-origin request reached fetch");
"#;
    run_bun(&files, harness);
}

#[test]
fn snapshots_cookie_scope_before_transport_revalidation() {
    let mut document: pons_ir::Document =
        serde_json::from_str(include_str!("../../../spec/fixtures/login.json")).unwrap();
    document.auth_schemes.insert(
        "session".into(),
        pons_ir::AuthScheme::CookieSession {
            cookie: "session".into(),
        },
    );
    document.operations[0].provides.as_mut().unwrap().value = CredentialLocator::SetCookie {
        name: "session".into(),
    };
    document.operations[1].auth.alternatives = vec![vec!["session".into()]];
    document.operations[1].path = vec![
        PathSegment::Literal { lit: "auth".into() },
        PathSegment::Literal {
            lit: "refresh".into(),
        },
    ];

    let files = emit_with_strategy(&document, EmitMode::Strict, ErrorStrategy::Throw)
        .unwrap()
        .files;
    let harness = r#"
import { createClient } from "./client.ts";

let calls = 0;
let protectedFetched = false;
let release!: () => void;
const gate = new Promise<void>((resolve) => { release = resolve; });
let started!: () => void;
const entered = new Promise<void>((resolve) => { started = resolve; });
const client = createClient({
  baseUrl: "https://example.test",
  transport: {
    onRequest: async ({ url }) => {
      if (url.pathname === "/auth/login") return;
      started();
      await gate;
      url.protocol = "http:";
      url.pathname = "/events/watch";
    },
  },
  fetch: async () => {
    calls += 1;
    if (calls === 1) {
      return new Response(JSON.stringify({ id: "1" }), {
        status: 200,
        headers: {
          "content-type": "application/json",
          "set-cookie": "session=old; Path=/auth; Secure",
        },
      });
    }
    if (calls === 2) {
      return new Response(JSON.stringify({ id: "1" }), {
        status: 200,
        headers: {
          "content-type": "application/json",
          "set-cookie": "session=new; Path=/",
        },
      });
    }
    protectedFetched = true;
    throw new Error("stale cookie reached fetch");
  },
});
await client.auth.login({ body: { email: "person@example.test" } });
const request = client.events.watch({ query: {}, headers: {} }).then(() => undefined, (cause) => cause);
await entered;
await client.auth.login({ body: { email: "person@example.test" } });
release();
const failure: any = await request;
if (failure?._tag !== "AuthUnavailableError") throw new Error(`unexpected failure ${JSON.stringify(failure)}`);
if (protectedFetched) throw new Error("stale cookie reached fetch");
"#;
    run_bun(&files, harness);
}

#[test]
fn snapshots_cookie_scope_across_concurrent_capture() {
    let mut document: pons_ir::Document =
        serde_json::from_str(include_str!("../../../spec/fixtures/login.json")).unwrap();
    document.auth_schemes.insert(
        "session".into(),
        pons_ir::AuthScheme::CookieSession {
            cookie: "session".into(),
        },
    );
    document.auth_schemes.insert(
        "adapter".into(),
        pons_ir::AuthScheme::Custom {
            description: "delayed adapter".into(),
            adapter: None,
            ext: Default::default(),
        },
    );
    document.operations[0].provides.as_mut().unwrap().value = CredentialLocator::SetCookie {
        name: "session".into(),
    };
    document.operations[1].auth.alternatives = vec![vec!["session".into(), "adapter".into()]];
    document.operations[1].path = vec![
        PathSegment::Literal { lit: "auth".into() },
        PathSegment::Literal {
            lit: "refresh".into(),
        },
    ];

    let files = emit_with_strategy(&document, EmitMode::Strict, ErrorStrategy::Throw)
        .unwrap()
        .files;
    let harness = r#"
import { createClient } from "./client.ts";

let calls = 0;
let protectedFetched = false;
let release!: () => void;
const gate = new Promise<void>((resolve) => { release = resolve; });
let started!: () => void;
const entered = new Promise<void>((resolve) => { started = resolve; });
const client = createClient({
  baseUrl: "https://example.test",
  authAdapters: {
    adapter: async () => {
      started();
      await gate;
      return {};
    },
  },
  transport: {
    onRequest: async ({ url }) => {
      if (url.pathname === "/auth/login") return;
      url.protocol = "http:";
      url.pathname = "/events/watch";
    },
  },
  fetch: async () => {
    calls += 1;
    if (calls === 1) {
      return new Response(JSON.stringify({ id: "1" }), {
        status: 200,
        headers: {
          "content-type": "application/json",
          "set-cookie": "session=old; Path=/auth; Secure",
        },
      });
    }
    if (calls === 2) {
      return new Response(JSON.stringify({ id: "1" }), {
        status: 200,
        headers: {
          "content-type": "application/json",
          "set-cookie": "session=new; Path=/",
        },
      });
    }
    protectedFetched = true;
    throw new Error("stale cookie reached fetch");
  },
});
await client.auth.login({ body: { email: "person@example.test" } });
const request = client.events.watch({ query: {}, headers: {} }).then(() => undefined, (cause) => cause);
await entered;
await client.auth.login({ body: { email: "person@example.test" } });
release();
const failure: any = await request;
if (failure?._tag !== "AuthUnavailableError") throw new Error(`unexpected failure ${JSON.stringify(failure)}`);
if (protectedFetched) throw new Error("stale cookie reached fetch");
"#;
    run_bun(&files, harness);
}

#[test]
fn clears_deleted_cookie_credentials_before_auth_fallback() {
    let mut document: pons_ir::Document =
        serde_json::from_str(include_str!("../../../spec/fixtures/login.json")).unwrap();
    document.auth_schemes.insert(
        "session".into(),
        pons_ir::AuthScheme::CookieSession {
            cookie: "session".into(),
        },
    );
    document.operations[0].provides.as_mut().unwrap().value = CredentialLocator::SetCookie {
        name: "session".into(),
    };
    document.operations[1].auth.alternatives = vec![vec!["session".into()], vec!["tenant".into()]];

    let files = emit_with_strategy(&document, EmitMode::Strict, ErrorStrategy::Throw)
        .unwrap()
        .files;
    let harness = r#"
import { createClient } from "./client.ts";

let calls = 0;
let gets = 0;
let clears = 0;
const sets: string[] = [];
const client = createClient({
  baseUrl: "http://example.test",
  auth: {
    session: {
      get: async () => {
        gets += 1;
        return undefined;
      },
      set: async (value: string) => {
        sets.push(value);
      },
      clear: async () => {
        clears += 1;
      },
    },
    tenant: "tenant",
  },
  fetch: async (_url, init) => {
    calls += 1;
    if (calls <= 3) {
      return new Response(JSON.stringify({ id: "1" }), {
        status: 200,
        headers: {
          "content-type": "application/json",
          "set-cookie": calls === 1
            ? "session=live; Path=/"
            : calls === 2
              ? "session=revoked; Path=/; Max-Age=0"
              : "session=expired; Path=/; Expires=Thu, 01 Jan 1970 00:00:00 GMT",
        },
      });
    }
    const headers = new Headers(init?.headers);
    if (headers.get("cookie") !== null) throw new Error("deleted cookie was replayed");
    if (headers.get("X-Tenant") !== "tenant") throw new Error("fallback was not selected");
    return new Response(new ReadableStream({ start(controller) { controller.close(); } }), {
      status: 200,
      headers: {
        "content-type": "text/event-stream",
        "X-Trace-ID": "trace",
      },
    });
  },
});
await client.auth.login({ body: { email: "person@example.test" } });
await client.auth.login({ body: { email: "person@example.test" } });
await client.auth.login({ body: { email: "person@example.test" } });
await client.events.watch({ path: { cursor: "cursor" }, query: {}, headers: {} });
if (sets.join(",") !== "live") throw new Error(`unexpected stored values: ${sets.join(",")}`);
if (clears !== 1) throw new Error(`unexpected clear count: ${clears}`);
    if (gets !== 0) throw new Error(`unexpected source reads: ${gets}`);
    "#;
    run_bun(&files, harness);
}

#[test]
fn does_not_capture_a_set_cookie_credential_from_an_allowed_cross_origin_redirect() {
    let mut document: pons_ir::Document =
        serde_json::from_str(include_str!("../../../spec/fixtures/login.json")).unwrap();
    document.operations[0].provides = Some(pons_ir::CredentialProvision {
        scheme: "session".into(),
        value: CredentialLocator::SetCookie { name: "sid".into() },
        expires_at: None,
        ext: BTreeMap::new(),
    });
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
  fetch: async () => {
    calls += 1;
    if (calls === 1) {
      return new Response(null, {
        status: 303,
        headers: { "set-cookie": "sid=from-cross-origin; Path=/", location: "https://b.test/auth/login" },
      });
    }
    return new Response(JSON.stringify({ id: "1" }), {
      status: 200,
      headers: { "content-type": "application/json" },
    });
  },
});
await client.auth.login({ body: { email: "person@example.test" } });
if (calls !== 2) throw new Error(`the allowed redirect was not followed: ${calls}`);
if (stored.length !== 0) throw new Error(`a cross-origin redirect stored ${JSON.stringify(stored)}`);
"#;
    run_bun(&files, harness);
}
