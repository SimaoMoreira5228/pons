mod support;

use std::collections::BTreeMap;

use pons_emit_ts::{EmitMode, ErrorStrategy, emit_with_strategy};
use pons_ir::{ApiKeyLocation, CredentialLocator, PathSegment};
use support::runtime::run_bun;

#[test]
fn replays_captured_cookie_session_credentials() {
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

const cookies: Array<string | null> = [];
const credentials: Array<RequestCredentials | undefined> = [];
const client = createClient({
  baseUrl: "http://example.test",
  headers: { cookie: "csrf=keep; session=old" },
  fetch: async (_url, init) => {
    cookies.push(new Headers(init?.headers).get("cookie"));
    credentials.push(init?.credentials);
    if (cookies.length === 1) {
      return new Response(JSON.stringify({ id: "1" }), {
        status: 200,
        headers: {
          "content-type": "application/json",
          "set-cookie": "session=captured; Path=/",
        },
      });
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
await client.auth.login({ body: { email: "person@example.test" } });
await client.events.watch({ path: { cursor: "cursor" }, query: {}, headers: {} });
if (cookies[1] !== "csrf=keep; session=captured") throw new Error(`cookie was not replayed: ${cookies[1]}`);
    if (credentials[1] !== "include") throw new Error(`credentials were not included: ${credentials[1]}`);
    "#;
    run_bun(&files, harness);
}

#[test]
fn uses_the_last_matching_set_cookie() {
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
  baseUrl: "http://example.test",
  fetch: async (_url, init) => {
    calls += 1;
    if (calls === 1) {
      return new Response(JSON.stringify({ id: "1" }), {
        status: 200,
        headers: [
          ["content-type", "application/json"],
          ["set-cookie", "session=obsolete; Path=/; Max-Age=0"],
          ["set-cookie", "session=current; Path=/; Max-Age=60"],
        ],
      });
    }
    const headers = new Headers(init?.headers);
    if (headers.get("cookie") !== "session=current") throw new Error(`wrong cookie: ${headers.get("cookie")}`);
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
fn replays_cookie_metadata_across_ports_on_one_host() {
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
let calls = 0;
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
  baseUrl: "https://example.test:8443",
  auth: { session: store },
  fetch: async () => {
    calls += 1;
    return new Response(JSON.stringify({ id: "1" }), {
      status: 200,
      headers: {
        "content-type": "application/json",
        "set-cookie": "session=secret; Path=/; Secure",
      },
    });
  },
});
await first.auth.login({ body: { email: "person@example.test" } });
const second = createClient({
  baseUrl: "https://example.test:9443",
  auth: { session: store },
  fetch: async (_url, init) => {
    calls += 1;
    if (new Headers(init?.headers).get("cookie") !== "session=secret") throw new Error("same-host cookie was not replayed");
    return new Response(new ReadableStream({ start(controller) { controller.close(); } }), {
      status: 200,
      headers: {
        "content-type": "text/event-stream",
        "X-Trace-ID": "trace",
      },
    });
  },
});
await second.events.watch({ path: { cursor: "cursor" }, query: {}, headers: {} });
if (calls !== 2) throw new Error(`unexpected request count: ${calls}`);
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
fn does_not_resurrect_a_deleted_cookie_source() {
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
  baseUrl: "https://example.test",
  auth: {
    session: () => "revoked",
    tenant: "tenant",
  },
  fetch: async (_url, init) => {
    calls += 1;
    if (calls === 1) {
      return new Response(JSON.stringify({ id: "1" }), {
        status: 200,
        headers: {
          "content-type": "application/json",
          "set-cookie": "session=revoked; Max-Age=0",
        },
      });
    }
    const headers = new Headers(init?.headers);
    if (headers.get("cookie") !== null) throw new Error("deleted source was replayed");
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
fn falls_back_after_deleting_an_ambient_cookie() {
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
  baseUrl: "https://example.test",
  auth: { tenant: "tenant" },
  fetch: async (_url, init) => {
    calls += 1;
    if (calls === 1) {
      return new Response(JSON.stringify({ id: "1" }), {
        status: 200,
        headers: {
          "content-type": "application/json",
          "set-cookie": "session=deleted; Max-Age=0",
        },
      });
    }
    const headers = new Headers(init?.headers);
    if (headers.get("cookie") !== null) throw new Error("ambient cookie was replayed");
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
fn keeps_broader_cookies_when_a_narrower_one_arrives() {
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
    let mut scoped = document.operations[1].clone();
    scoped.id = vec![vec!["admin".into()], vec!["events".into()]];
    scoped.path.insert(
        0,
        PathSegment::Literal {
            lit: "admin".into(),
        },
    );
    document.operations.push(scoped);

    let files = emit_with_strategy(&document, EmitMode::Strict, ErrorStrategy::Throw)
        .unwrap()
        .files;
    let harness = r#"
import { createClient } from "./client.ts";

let calls = 0;
let narrow = true;
const client = createClient({
  baseUrl: "https://example.test",
  fetch: async (_url, init) => {
    calls += 1;
    if (calls === 5) {
      narrow = false;
      return new Response(JSON.stringify({ id: "1" }), {
        status: 200,
        headers: {
          "content-type": "application/json",
          "set-cookie": "session=narrow; Path=/admin; Max-Age=0",
        },
      });
    }
    if (calls <= 2) {
      const cookie = calls === 1 ? "session=broad; Path=/" : "session=narrow; Path=/admin";
      return new Response(JSON.stringify({ id: "1" }), {
        status: 200,
        headers: { "content-type": "application/json", "set-cookie": cookie },
      });
    }
    const scoped = new URL(String(_url)).pathname.startsWith("/admin");
    const expected = narrow && scoped ? "session=narrow" : "session=broad";
    const sent = new Headers(init?.headers).get("cookie");
    if (sent !== expected) throw new Error(`wrong cookie for ${expected}: ${sent}`);
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
await client.events.watch({ path: { cursor: "cursor" }, query: {}, headers: {} });
await client.admin.events({ path: { cursor: "cursor" }, query: {}, headers: {} });
await client.auth.login({ body: { email: "person@example.test" } });
await client.admin.events({ path: { cursor: "cursor" }, query: {}, headers: {} });
if (calls !== 6) throw new Error(`unexpected calls ${calls}`);
"#;
    run_bun(&files, harness);
}

#[test]
fn does_not_resurrect_an_expired_configured_cookie() {
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
  baseUrl: "https://example.test",
  auth: {
    session: () => "expired",
    tenant: "tenant",
  },
  fetch: async (_url, init) => {
    calls += 1;
    if (calls === 1) {
      return new Response(JSON.stringify({ id: "1" }), {
        status: 200,
        headers: {
          "content-type": "application/json",
          "set-cookie": "session=expired; Max-Age=0.01",
        },
      });
    }
    const headers = new Headers(init?.headers);
    if (headers.get("cookie") !== null) throw new Error("expired source was replayed");
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
await new Promise((resolve) => setTimeout(resolve, 20));
await client.events.watch({ path: { cursor: "cursor" }, query: {}, headers: {} });
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
fn does_not_replay_secure_cookies_over_http() {
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
          "set-cookie": "session=secret; Secure",
        },
      });
    }
    const headers = new Headers(init?.headers);
    if (headers.get("cookie") !== null) throw new Error("secure cookie was replayed over HTTP");
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
fn parses_combined_set_cookie_headers_without_get_set_cookie() {
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
  fetch: async (_url, init) => {
    calls += 1;
    if (calls === 1) {
      return {
        status: 200,
        url: "",
        headers: {
          get(name: string) {
            if (name.toLowerCase() === "content-type") return "application/json";
            if (name.toLowerCase() === "set-cookie") {
              return "session=old; Expires=Wed, 21 Oct 2015 07:28:00 GMT; Path=/, session=current; Path=/";
            }
            return null;
          },
        },
        text: async () => JSON.stringify({ id: "1" }),
      } as unknown as Response;
    }
    const headers = new Headers(init?.headers);
    if (headers.get("cookie") !== "session=current") throw new Error(`wrong cookie: ${headers.get("cookie")}`);
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
fn does_not_replay_a_cross_origin_cookie() {
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
  fetch: async () => {
    calls += 1;
    if (calls === 1) {
      return {
        status: 200,
        url: "https://idp.test/oauth/callback",
        headers: {
          get(name: string) {
            if (name.toLowerCase() === "content-type") return "application/json";
            if (name.toLowerCase() === "set-cookie") return "session=idp; Path=/; Secure";
            return null;
          },
        },
        text: async () => JSON.stringify({ id: "1" }),
      } as unknown as Response;
    }
    throw new Error("cross-origin cookie reached fetch");
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
if (calls !== 1) throw new Error("cross-origin cookie reached fetch");
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
fn expires_captured_cookie_credentials() {
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
    if (calls === 1) {
      return new Response(JSON.stringify({ id: "1" }), {
        status: 200,
        headers: {
          "content-type": "application/json",
          "set-cookie": "session=live; Max-Age=1",
        },
      });
    }
    const headers = new Headers(init?.headers);
    if (headers.get("cookie") !== null) throw new Error("expired cookie was replayed");
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
await new Promise((resolve) => setTimeout(resolve, 1100));
await client.events.watch({ path: { cursor: "cursor" }, query: {}, headers: {} });
if (sets.join(",") !== "live") throw new Error(`unexpected stored values: ${sets.join(",")}`);
if (clears !== 1) throw new Error(`unexpected clear count: ${clears}`);
if (gets !== 1) throw new Error(`unexpected source reads: ${gets}`);
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
