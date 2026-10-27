mod support;

use pons_emit_ts::{EmitMode, ErrorStrategy, emit_with_strategy};
use pons_ir::{ApiKeyLocation, CredentialLocator, PathSegment};
use support::runtime::run_bun;

#[test]
fn persists_cookie_metadata_across_clients() {
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

let stored: any;
let protectedFetches = 0;
const store = {
  get: async () => stored,
  set: async (value: string, metadata: any) => {
    stored = { value, ...metadata };
  },
  clear: async () => {
    stored = undefined;
  },
};
const first = createClient({
  baseUrl: "https://example.test/tenant-a",
  auth: { session: store },
  fetch: async () => new Response(JSON.stringify({ id: "1" }), {
    status: 200,
    headers: {
      "content-type": "application/json",
      "set-cookie": "session=secret; Path=/auth; Secure",
    },
  }),
});
await first.auth.login({ body: { email: "person@example.test" } });
if (stored?.path !== "/auth" || stored?.secure !== true || stored?.host !== "example.test") {
  throw new Error(`cookie metadata was not persisted: ${JSON.stringify(stored)}`);
}
const expectAuthFailure = async (baseUrl: string) => {
  const client = createClient({
    baseUrl,
    auth: { session: store },
    fetch: async () => {
      protectedFetches += 1;
      throw new Error("out-of-scope cookie reached fetch");
    },
  });
  let failure: any;
  try {
    await client.events.watch({ path: { cursor: "cursor" }, query: {}, headers: {} });
  } catch (cause) {
    failure = cause;
  }
  if (failure?._tag !== "AuthUnavailableError") throw new Error(`unexpected failure ${JSON.stringify(failure)}`);
};
await expectAuthFailure("https://example.test");
await expectAuthFailure("https://other.test");
if (protectedFetches !== 0) throw new Error("out-of-scope cookie reached fetch");
"#;
    run_bun(&files, harness);
}

#[test]
fn rejects_unscoped_cookie_token_store_records() {
    let mut document: pons_ir::Document =
        serde_json::from_str(include_str!("../../../spec/fixtures/login.json")).unwrap();
    document.auth_schemes.insert(
        "session".into(),
        pons_ir::AuthScheme::CookieSession {
            cookie: "session".into(),
        },
    );
    document.operations[1].auth.alternatives = vec![vec!["session".into()]];

    let files = emit_with_strategy(&document, EmitMode::Strict, ErrorStrategy::Throw)
        .unwrap()
        .files;
    let harness = r#"
import { createClient } from "./client.ts";

let fetched = false;
const client = createClient({
  baseUrl: "https://example.test",
  auth: {
    session: {
      get: async () => ({ value: "secret" }),
      set: async () => {},
      clear: async () => {},
    },
  },
  fetch: async () => {
    fetched = true;
    throw new Error("unscoped cookie reached fetch");
  },
});
let failure: any;
try {
  await client.events.watch({ path: { cursor: "cursor" }, query: {}, headers: {} });
} catch (cause) {
  failure = cause;
}
if (failure?._tag !== "AuthSourceError") throw new Error(`unexpected failure ${JSON.stringify(failure)}`);
if (fetched) throw new Error("unscoped cookie reached fetch");
"#;
    run_bun(&files, harness);
}

#[test]
fn ignores_cookie_deletions_for_another_path() {
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
  fetch: async (_url, init) => {
    calls += 1;
    if (calls === 1) {
      return new Response(JSON.stringify({ id: "1" }), {
        status: 200,
        headers: {
          "content-type": "application/json",
          "set-cookie": "session=live; Path=/auth; Secure",
        },
      });
    }
    if (calls === 2) {
      return new Response(JSON.stringify({ id: "1" }), {
        status: 200,
        headers: {
          "content-type": "application/json",
          "set-cookie": "session=deleted; Path=/other; Max-Age=0",
        },
      });
    }
    const headers = new Headers(init?.headers);
    if (headers.get("cookie") !== "session=live") throw new Error(`wrong cookie: ${headers.get("cookie")}`);
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
await client.events.watch({ query: {}, headers: {} });
"#;
    run_bun(&files, harness);
}

#[test]
fn scopes_captured_cookie_credentials_to_the_default_path() {
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
const client = createClient({
  baseUrl: "http://example.test",
  auth: { tenant: "tenant" },
  fetch: async (_url, init) => {
    calls += 1;
    if (calls === 1) {
      return new Response(JSON.stringify({ id: "1" }), {
        status: 200,
        headers: {
          "content-type": "application/json",
          "set-cookie": "session=secret",
        },
      });
    }
    const headers = new Headers(init?.headers);
    if (headers.get("cookie") !== null) throw new Error("out-of-path cookie was replayed");
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
await client.events.watch({ path: { cursor: "cursor" }, query: {}, headers: {} });
"#;
    run_bun(&files, harness);
}

#[test]
fn rejects_insecure_secure_prefix_cookies() {
    let mut document: pons_ir::Document =
        serde_json::from_str(include_str!("../../../spec/fixtures/login.json")).unwrap();
    document.auth_schemes.insert(
        "session".into(),
        pons_ir::AuthScheme::CookieSession {
            cookie: "__Secure-session".into(),
        },
    );
    document.operations[0].provides.as_mut().unwrap().value = CredentialLocator::SetCookie {
        name: "__Secure-session".into(),
    };
    document.operations[1].auth.alternatives = vec![vec!["session".into()], vec!["tenant".into()]];

    let files = emit_with_strategy(&document, EmitMode::Strict, ErrorStrategy::Throw)
        .unwrap()
        .files;
    let harness = r#"
import { createClient } from "./client.ts";

let calls = 0;
const client = createClient({
  baseUrl: "https://example.test",
  auth: { tenant: "tenant" },
  fetch: async (_url, init) => {
    calls += 1;
    if (calls === 1) {
      return new Response(JSON.stringify({ id: "1" }), {
        status: 200,
        headers: {
          "content-type": "application/json",
          "set-cookie": "__Secure-session=secret; Path=/",
        },
      });
    }
    const headers = new Headers(init?.headers);
    if (headers.get("cookie") !== null) throw new Error("invalid __Secure- cookie was replayed");
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
await client.events.watch({ path: { cursor: "cursor" }, query: {}, headers: {} });
"#;
    run_bun(&files, harness);
}

#[test]
fn validates_prefixes_on_the_effective_cookie_name() {
    let mut document: pons_ir::Document =
        serde_json::from_str(include_str!("../../../spec/fixtures/login.json")).unwrap();
    document.auth_schemes.insert(
        "session".into(),
        pons_ir::AuthScheme::CookieSession {
            cookie: "__Secure-session".into(),
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
const client = createClient({
  baseUrl: "https://example.test",
  auth: { tenant: "tenant" },
  fetch: async (_url, init) => {
    calls += 1;
    if (calls === 1) {
      return new Response(JSON.stringify({ id: "1" }), {
        status: 200,
        headers: {
          "content-type": "application/json",
          "set-cookie": "session=secret; Path=/",
        },
      });
    }
    const headers = new Headers(init?.headers);
    if (headers.get("cookie") !== null) throw new Error("unsecured source fed a __Secure- cookie");
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
await client.events.watch({ path: { cursor: "cursor" }, query: {}, headers: {} });
"#;
    run_bun(&files, harness);
}

#[test]
fn validates_prefixes_for_cookie_api_keys() {
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
    document.operations[1].auth.alternatives = vec![vec!["api".into()], vec!["tenant".into()]];

    let files = emit_with_strategy(&document, EmitMode::Strict, ErrorStrategy::Throw)
        .unwrap()
        .files;
    let harness = r#"
import { createClient } from "./client.ts";

let calls = 0;
const client = createClient({
  baseUrl: "https://example.test",
  auth: { tenant: "tenant" },
  fetch: async (_url, init) => {
    calls += 1;
    if (calls === 1) {
      return new Response(JSON.stringify({ id: "1" }), {
        status: 200,
        headers: {
          "content-type": "application/json",
          "set-cookie": "session=secret; Path=/",
        },
      });
    }
    const headers = new Headers(init?.headers);
    if (headers.get("cookie") !== null) throw new Error("unsecured source fed a __Secure- API key");
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
await client.events.watch({ path: { cursor: "cursor" }, query: {}, headers: {} });
"#;
    run_bun(&files, harness);
}

#[test]
fn rejects_non_root_host_prefix_cookies() {
    let mut document: pons_ir::Document =
        serde_json::from_str(include_str!("../../../spec/fixtures/login.json")).unwrap();
    document.auth_schemes.insert(
        "session".into(),
        pons_ir::AuthScheme::CookieSession {
            cookie: "__Host-session".into(),
        },
    );
    document.operations[0].provides.as_mut().unwrap().value = CredentialLocator::SetCookie {
        name: "__Host-session".into(),
    };
    document.operations[1].auth.alternatives = vec![vec!["session".into()], vec!["tenant".into()]];

    let files = emit_with_strategy(&document, EmitMode::Strict, ErrorStrategy::Throw)
        .unwrap()
        .files;
    let harness = r#"
import { createClient } from "./client.ts";

let calls = 0;
const client = createClient({
  baseUrl: "https://example.test",
  auth: { tenant: "tenant" },
  fetch: async (_url, init) => {
    calls += 1;
    if (calls === 1) {
      return new Response(JSON.stringify({ id: "1" }), {
        status: 200,
        headers: {
          "content-type": "application/json",
          "set-cookie": "__Host-session=secret; Path=/auth; Secure",
        },
      });
    }
    const headers = new Headers(init?.headers);
    if (headers.get("cookie") !== null) throw new Error("invalid __Host- cookie was replayed");
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
await client.events.watch({ path: { cursor: "cursor" }, query: {}, headers: {} });
"#;
    run_bun(&files, harness);
}

#[test]
fn normalizes_relative_cookie_paths() {
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
  fetch: async (_url, init) => {
    calls += 1;
    if (calls === 1) {
      return new Response(JSON.stringify({ id: "1" }), {
        status: 200,
        headers: {
          "content-type": "application/json",
          "set-cookie": "session=secret; Path=auth",
        },
      });
    }
    const headers = new Headers(init?.headers);
    if (headers.get("cookie") !== "session=secret") throw new Error(`wrong cookie: ${headers.get("cookie")}`);
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
await client.events.watch({ query: {}, headers: {} });
"#;
    run_bun(&files, harness);
}

#[test]
fn rejects_cookie_values_that_can_add_cookie_pairs() {
    let mut document: pons_ir::Document =
        serde_json::from_str(include_str!("../../../spec/fixtures/login.json")).unwrap();
    document.auth_schemes.insert(
        "session".into(),
        pons_ir::AuthScheme::CookieSession {
            cookie: "session".into(),
        },
    );
    document.operations[1].auth.alternatives = vec![vec!["session".into()]];

    let files = emit_with_strategy(&document, EmitMode::Strict, ErrorStrategy::Throw)
        .unwrap()
        .files;
    let harness = r#"
import { createClient } from "./client.ts";

let fetched = false;
const client = createClient({
  baseUrl: "http://example.test",
  auth: { session: "token; admin=1" },
  headers: { cookie: "admin=0" },
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
if (failure?._tag !== "AuthSourceError") throw new Error(`unexpected failure ${JSON.stringify(failure)}`);
if (fetched) throw new Error("invalid cookie reached fetch");
"#;
    run_bun(&files, harness);
}

#[test]
fn rejects_cookie_names_that_can_add_cookie_pairs() {
    let mut document: pons_ir::Document =
        serde_json::from_str(include_str!("../../../spec/fixtures/login.json")).unwrap();
    document.auth_schemes.insert(
        "session".into(),
        pons_ir::AuthScheme::CookieSession {
            cookie: "session=admin".into(),
        },
    );
    document.operations[1].auth.alternatives = vec![vec!["session".into()]];

    let files = emit_with_strategy(&document, EmitMode::Strict, ErrorStrategy::Throw)
        .unwrap()
        .files;
    let harness = r#"
import { createClient } from "./client.ts";

let fetched = false;
const client = createClient({
  baseUrl: "http://example.test",
  auth: { session: "1" },
  headers: { cookie: "session=real" },
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
if (failure?._tag !== "AuthSourceError") throw new Error(`unexpected failure ${JSON.stringify(failure)}`);
if (fetched) throw new Error("invalid cookie name reached fetch");
"#;
    run_bun(&files, harness);
}
