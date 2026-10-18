use std::{fs, process::Command};

use pons_emit_ts::{EmitMode, ErrorStrategy, emit_with_strategy};
use pons_ir::Document;

#[test]
fn the_transport_hooks_wrap_fetch_in_order_and_their_failures_stay_typed() {
    let document: Document =
        serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
    let output = emit_with_strategy(&document, EmitMode::Strict, ErrorStrategy::Neverthrow)
        .expect("the login fixture emits");
    let directory = tempfile::tempdir().unwrap();
    for file in output.files {
        let destination = directory.path().join(&file.path);
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(destination, &file.content).unwrap();
    }
    let harness = directory.path().join("transport.ts");
    fs::write(&harness, HARNESS).unwrap();
    let result = Command::new("bun")
        .arg("run")
        .arg(&harness)
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
assert.equal((requestFailure as any).error.kind, "transport");
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
assert.equal((responseFailure as any).error.kind, "transport");
assert.equal((responseFailure as any).error.phase, "response");

console.log("transport ok");
"#;
