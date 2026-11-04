mod support;

use pons_ir::Document;

use support::type_check::{files, fixture, operations_of, type_checks};

#[test]
fn a_headers_bag_of_optional_headers_is_itself_optional() {
    let document: Document = serde_json::from_value(fixture()).unwrap();
    let files = files(&document);
    assert!(
        operations_of(&files, "events").contains("headers?: { \"X-Request-ID\"?: string }"),
        "{}",
        operations_of(&files, "events")
    );
    type_checks(
        &files,
        r#"
import { createClient } from "./client.js";

const client = createClient({ baseUrl: "https://api.example.test" });
export const watched = client.events.watch({ path: { cursor: "c" }, query: {} });
export const withHeader = client.events.watch({
  path: { cursor: "c" },
  query: {},
  headers: { "X-Request-ID": "1" },
});
// @ts-expect-error the path parameter is required
export const noCursor = client.events.watch({ query: {} });
"#,
    );
}

#[test]
fn a_required_header_keeps_the_bag_required() {
    let mut raw = fixture();
    raw["operations"][1]["headers"][0]["presence"] = serde_json::json!("required");
    let document: Document = serde_json::from_value(raw).unwrap();
    let files = files(&document);
    assert!(
        operations_of(&files, "events").contains("headers: { \"X-Request-ID\": string }"),
        "{}",
        operations_of(&files, "events")
    );
    type_checks(
        &files,
        r#"
import { createClient } from "./client.js";

const client = createClient({ baseUrl: "https://api.example.test" });
// @ts-expect-error the only declared request header is required
export const watched = client.events.watch({ path: { cursor: "c" }, query: {} });
export const withHeader = client.events.watch({
  path: { cursor: "c" },
  query: {},
  headers: { "X-Request-ID": "1" },
});
"#,
    );
}
