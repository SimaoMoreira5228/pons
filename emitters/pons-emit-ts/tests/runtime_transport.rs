use std::{fs, process::Command};

use pons_emit_ts::{EmitMode, ErrorStrategy, emit_with_strategy};
use pons_ir::Document;

fn fixture() -> Document {
    serde_json::from_str(include_str!("../../../spec/fixtures/login.json")).unwrap()
}

fn run(document: &Document, name: &str, harness: &str) {
    let output = emit_with_strategy(document, EmitMode::Strict, ErrorStrategy::Neverthrow)
        .expect("the login fixture emits");
    let directory = tempfile::tempdir().unwrap();
    for file in output.files {
        let destination = directory.path().join(&file.path);
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(destination, &file.content).unwrap();
    }
    let harness_path = directory.path().join(name);
    fs::write(&harness_path, harness).unwrap();
    let result = Command::new("bun")
        .arg("run")
        .arg(&harness_path)
        .current_dir(directory.path())
        .output()
        .expect("Bun is required for the transport runtime test");
    assert!(
        result.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
}

#[test]
fn the_transport_hooks_wrap_fetch_in_order_and_their_failures_stay_typed() {
    run(&fixture(), "transport.ts", HARNESS);
}

#[test]
fn retry_skips_a_non_idempotent_call_until_the_caller_forces_it() {
    run(&fixture(), "retry.ts", RETRY_HARNESS);
    let mut idempotent = fixture();
    idempotent.operations[0].semantics.idempotent = true;
    run(&idempotent, "retry.ts", IDEMPOTENT_RETRY_HARNESS);
}

const HARNESS: &str = r#"
import { strict as assert } from "node:assert";
import { createClient } from "./client.js";

const order: string[] = [];
const client = createClient({
  baseUrl: "http://example.test",
  transport: {
    onRequest: ({ url }) => {
      order.push(`request:${url.pathname}`);
    },
    onResponse: ({ response }) => {
      order.push(`response:${response.status}`);
    },
  },
  fetch: async () => {
    order.push("fetch");
    return new Response('{"id":"1"}', { status: 200, headers: { "content-type": "application/json" } });
  },
});
const value = await client.auth.login({ body: { email: "person@example.test" } });
assert.equal(value.isOk(), true);
assert.deepEqual(order, ["request:/auth/login", "fetch", "response:200"]);

const refusedRequest = createClient({
  baseUrl: "http://example.test",
  transport: {
    onRequest: () => {
      throw new Error("the request hook refused");
    },
  },
  fetch: async () => {
    throw new Error("fetch must not run after a refused request hook");
  },
});
const requestFailure = await refusedRequest.auth.login({ body: { email: "person@example.test" } });
assert.equal(requestFailure.isErr(), true);
assert.equal((requestFailure as any).error._tag, "TransportHookError");
assert.equal((requestFailure as any).error.phase, "request");

const refusedResponse = createClient({
  baseUrl: "http://example.test",
  transport: {
    onResponse: () => {
      throw new Error("the response hook refused");
    },
  },
  fetch: async () => new Response('{"id":"1"}', { status: 200, headers: { "content-type": "application/json" } }),
});
const responseFailure = await refusedResponse.auth.login({ body: { email: "person@example.test" } });
assert.equal(responseFailure.isErr(), true);
assert.equal((responseFailure as any).error._tag, "TransportHookError");
assert.equal((responseFailure as any).error.phase, "response");

console.log("transport ok");
"#;

const RETRY_HARNESS: &str = r#"
import { strict as assert } from "node:assert";
import { createClient } from "./client.js";

function counting() {
  let calls = 0;
  return {
    calls: () => calls,
    fetch: async () => {
      calls += 1;
      return new Response("", { status: 503 });
    },
  };
}
const retry = { attempts: 3, backoffMs: 0, maxBackoffMs: 0, jitterMs: 0 };
const login = { body: { email: "person@example.test" } };

const plain = counting();
const client = createClient({ baseUrl: "http://example.test", retry, fetch: plain.fetch });
assert.equal((await client.auth.login(login)).isErr(), true);
assert.equal(plain.calls(), 1);

const forced = counting();
const forcing = createClient({ baseUrl: "http://example.test", retry: { ...retry, force: true }, fetch: forced.fetch });
assert.equal((await forcing.auth.login(login)).isErr(), true);
assert.equal(forced.calls(), 3);

console.log("retry ok");
"#;

const IDEMPOTENT_RETRY_HARNESS: &str = r#"
import { strict as assert } from "node:assert";
import { createClient } from "./client.js";

let calls = 0;
const client = createClient({
  baseUrl: "http://example.test",
  retry: { attempts: 3, backoffMs: 0, maxBackoffMs: 0, jitterMs: 0 },
  fetch: async () => {
    calls += 1;
    return new Response("", { status: 503 });
  },
});
assert.equal((await client.auth.login({ body: { email: "person@example.test" } })).isErr(), true);
assert.equal(calls, 3);

console.log("idempotent retry ok");
"#;
