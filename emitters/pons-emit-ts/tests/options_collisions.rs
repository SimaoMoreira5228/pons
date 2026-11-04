mod support;

use pons_emit_ts::{Namespaces, TsOptions, emit_with_options};
use serde_json::json;
use support::options::{emitted, file_of, operation, run_bun, with_operations};

#[test]
fn member_collision_is_refused_in_both_shapes() {
    let document = with_operations(vec![
        operation(json!([["a.b"]]), "ab"),
        operation(json!([["a_b"]]), "ab2"),
    ]);
    for namespaces in [Namespaces::Nested, Namespaces::Flat] {
        let error = emit_with_options(
            &document,
            TsOptions {
                namespaces,
                ..TsOptions::default()
            },
        )
        .unwrap_err();
        assert_eq!(
            error.0, "client member collision at a_b",
            "{namespaces:?} names both ids the same member and refuses to write one over the other"
        );
    }
}

#[test]
fn a_prototype_member_is_quoted_in_both_shapes() {
    let document = with_operations(vec![operation(json!([["__proto__"]]), "proto")]);
    for (namespaces, expected) in [
        (
            Namespaces::Nested,
            "    [\"__proto__\"]: __proto__Operations(configured)\n",
        ),
        (
            Namespaces::Flat,
            "    [\"__proto__\"]: __proto__Operations(configured)\n",
        ),
    ] {
        let client = file_of(
            &document,
            TsOptions {
                namespaces,
                ..TsOptions::default()
            },
            "client.ts",
        );
        assert!(client.contains(expected), "{namespaces:?}:\n{client}");
    }

    let document = with_operations(vec![operation(json!([["__proto__"], ["x"]]), "proto")]);
    let client = file_of(
        &document,
        TsOptions {
            namespaces: Namespaces::Flat,
            ..TsOptions::default()
        },
        "client.ts",
    );
    assert!(
        client.contains("    [\"__proto__.x\"]: __proto___xOperations(configured)\n"),
        "a dotted member is quoted whether or not it is a prototype key:\n{client}"
    );
    run_bun(
        &emitted(
            &document,
            TsOptions {
                namespaces: Namespaces::Flat,
                ..TsOptions::default()
            },
        ),
        r#"
import { createClient } from "./client.ts";

const client = createClient({
  baseUrl: "http://example.test",
  fetch: async () => new Response("\"ok\"", {
    status: 200,
    headers: { "content-type": "application/json" },
  }),
});
if (Object.getPrototypeOf(client) !== Object.prototype) throw new Error("the client prototype changed");
const answer = await client["__proto__.x"]();
if (!answer.isOk() || answer.value !== "ok") throw new Error("the prototype member was not callable");
"#,
    );
}
