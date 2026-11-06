mod support;

use std::collections::BTreeMap;

use pons_emit_ts::{EmitMode, ErrorStrategy, emit_with_strategy};
use pons_ir::{ApiKeyLocation, CredentialLocator, InlineType, Presence, TypeRef};
use support::runtime::run_bun;

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
