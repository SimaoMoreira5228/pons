mod support;

use pons_emit_ts::{ErrorStrategy, Namespaces, TsOptions};
use pons_ir::Document;
use serde_json::json;
use support::options::{emitted, file, operation, run_bun, with_operations};

fn two_namespaces() -> Document {
    with_operations(vec![
        operation(json!([["a"], ["get"]]), "a"),
        operation(json!([["b"], ["get"]]), "b"),
    ])
}

#[test]
fn nested_namespaces_hang_the_operations_under_their_first_id_segment() {
    let document = two_namespaces();
    let options = TsOptions {
        errors: ErrorStrategy::Throw,
        ..TsOptions::default()
    };
    let files = emitted(&document, options);
    let client = file(&files, "client.ts");
    assert!(
        client.contains("    a: aOperations(configured),\n    b: bOperations(configured)\n"),
        "{client}"
    );
    let a = file(&files, "operations/a.ts");
    assert!(a.contains("    get: (_options?: CallOptions) =>"), "{a}");
    assert!(
        !file(&files, "client.ts").contains("[\"a.get\"]"),
        "the nested shape keeps the tree"
    );
    run_bun(
        &files,
        r#"
import { createClient } from "./client.ts";

const client = createClient({
  baseUrl: "http://example.test",
  fetch: async (url) => new Response(JSON.stringify(url.pathname), {
    status: 200,
    headers: { "content-type": "application/json" },
  }),
});
if (await client.a.get() !== "/a") throw new Error("client.a.get did not answer");
if (await client.b.get() !== "/b") throw new Error("client.b.get did not answer");
"#,
    );
}

#[test]
fn flat_namespaces_name_every_operation_by_its_whole_id() {
    let document = two_namespaces();
    let options = TsOptions {
        namespaces: Namespaces::Flat,
        errors: ErrorStrategy::Throw,
        ..TsOptions::default()
    };
    let files = emitted(&document, options);
    let client = file(&files, "client.ts");
    assert!(
        client.contains("    [\"a.get\"]: a_getOperations(configured),\n    [\"b.get\"]: b_getOperations(configured)\n"),
        "both operations keep their namespace in the member name:\n{client}"
    );
    assert!(
        client.contains("import { operations as a_getOperations } from \"./operations/a.get.js\";"),
        "the import binding is an identifier the dotted member is not:\n{client}"
    );
    assert!(
        files.iter().any(|file| file.path == "operations/a.get.ts"),
        "one file per operation: {:?}",
        files.iter().map(|file| &file.path).collect::<Vec<_>>()
    );
    let a = file(&files, "operations/a.get.ts");
    assert!(
        a.contains("export function operations(configured: ConfiguredClient) {\n  return (_options?: CallOptions) =>"),
        "a member that names one operation is that operation:\n{a}"
    );
    run_bun(
        &files,
        r#"
import { createClient } from "./client.ts";

const client = createClient({
  baseUrl: "http://example.test",
  fetch: async (url) => new Response(JSON.stringify(url.pathname), {
    status: 200,
    headers: { "content-type": "application/json" },
  }),
});
if (Object.keys(client).join(",") !== "a.get,b.get") {
  throw new Error(`the client exposes ${Object.keys(client).join(",")}`);
}
if (await client["a.get"]() !== "/a") throw new Error("a.get did not answer");
if (await client["b.get"]() !== "/b") throw new Error("b.get did not answer");
"#,
    );
}
