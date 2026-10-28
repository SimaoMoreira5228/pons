mod support;

use pons_emit_ts::{EmitMode, ErrorStrategy, GeneratedFile, emit_with_strategy};
use serde_json::json;
use support::runtime::{paginated_document, run_bun};

fn files(pagination: serde_json::Value) -> Vec<GeneratedFile> {
    emit_with_strategy(
        &paginated_document(pagination),
        EmitMode::Strict,
        ErrorStrategy::Neverthrow,
    )
    .expect("the paginated document is supported")
    .files
}

#[test]
fn walks_cursor_pages_until_the_next_cursor_is_empty() {
    let files = files(json!({
        "request": {"kind": "cursor", "field": "cursor"},
        "items": "/items",
        "next": "/next_cursor",
        "page_size": {"field": "limit", "default": 20}
    }));
    let harness = r#"
import { createClient } from "./client.ts";

const cursors: (string | null)[] = [];
const limits: (string | null)[] = [];
const pages = [
  { items: [{ id: "1" }, { id: "2" }], next_cursor: "c2" },
  { items: [{ id: "3" }], next_cursor: "" },
];
let call = 0;
const client = createClient({
  baseUrl: "https://example.test",
  fetch: async (url) => {
    const parsed = new URL(url);
    cursors.push(parsed.searchParams.get("cursor"));
    limits.push(parsed.searchParams.get("limit"));
    return new Response(JSON.stringify(pages[call++]), {
      status: 200,
      headers: { "content-type": "application/json" },
    });
  },
});
const ids: string[] = [];
for await (const item of client.events.listAll({ query: {} })) {
  if (item.isErr()) throw item.error;
  ids.push(item.value.id);
}
if (ids.join(",") !== "1,2,3") throw new Error(`wrong items: ${ids.join(",")}`);
if (cursors.length !== 2) throw new Error(`the iterator kept requesting: ${cursors.length}`);
if (cursors[0] !== null || cursors[1] !== "c2") {
  throw new Error(`wrong cursors: ${JSON.stringify(cursors)}`);
}
if (limits[0] !== "20" || limits[1] !== null) {
  throw new Error(`the page size default crossed more than once: ${JSON.stringify(limits)}`);
}
"#;
    run_bun(&files, harness);
}

#[test]
fn walks_offset_pages_and_stops_on_an_empty_page() {
    let files = files(json!({
        "request": {"kind": "offset", "field": "offset"},
        "items": "/items"
    }));
    let harness = r#"
import { createClient } from "./client.ts";

const offsets: (string | null)[] = [];
const pages = [
  { items: [{ id: "1" }, { id: "2" }] },
  { items: [{ id: "3" }, { id: "4" }, { id: "5" }] },
  { items: [] },
];
let call = 0;
const client = createClient({
  baseUrl: "https://example.test",
  fetch: async (url) => {
    const parsed = new URL(url);
    offsets.push(parsed.searchParams.get("offset"));
    return new Response(JSON.stringify(pages[call++]), {
      status: 200,
      headers: { "content-type": "application/json" },
    });
  },
});
const ids: string[] = [];
for await (const item of client.events.listAll({ query: {} })) {
  if (item.isErr()) throw item.error;
  ids.push(item.value.id);
}
if (ids.join(",") !== "1,2,3,4,5") throw new Error(`wrong items: ${ids.join(",")}`);
if (offsets.length !== 3) throw new Error(`the iterator kept requesting: ${offsets.length}`);
if (offsets[0] !== null || offsets[1] !== "2" || offsets[2] !== "5") {
  throw new Error(`wrong offsets: ${JSON.stringify(offsets)}`);
}
"#;
    run_bun(&files, harness);
}

#[test]
fn yields_one_failure_and_stops_when_a_page_request_fails() {
    let files = files(json!({
        "request": {"kind": "cursor", "field": "cursor"},
        "items": "/items",
        "next": "/next_cursor"
    }));
    let harness = r#"
import { createClient } from "./client.ts";

let calls = 0;
const client = createClient({
  baseUrl: "https://example.test",
  fetch: async () => {
    calls += 1;
    if (calls === 1) {
      return new Response(JSON.stringify({ items: [{ id: "1" }], next_cursor: "c2" }), {
        status: 200,
        headers: { "content-type": "application/json" },
      });
    }
    return new Response(JSON.stringify("gone"), {
      status: 404,
      headers: { "content-type": "application/json" },
    });
  },
});
const results: any[] = [];
for await (const item of client.events.listAll({ query: {} })) results.push(item);
if (results.length !== 2) throw new Error(`unexpected results: ${results.length}`);
if (!results[0]?.isOk()) throw new Error("the first page did not yield its item");
if (!results[1]?.isErr()) throw new Error("the failed page did not yield a failure");
if (results[1].error._tag !== "ApiError") throw new Error(JSON.stringify(results[1].error));
if (results[1].error.status !== 404) throw new Error(`wrong status ${results[1].error.status}`);
if (results[1].error.body !== "gone") throw new Error(`wrong body ${results[1].error.body}`);
if (calls !== 2) throw new Error(`the iterator kept requesting: ${calls}`);
"#;
    run_bun(&files, harness);
}
