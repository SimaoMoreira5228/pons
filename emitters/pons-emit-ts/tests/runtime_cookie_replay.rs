mod support;

use pons_emit_ts::{EmitMode, ErrorStrategy, emit_with_strategy};
use pons_ir::{CredentialLocator, PathSegment};
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
