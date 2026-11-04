mod support;

use pons_emit_ts::{ErrorStrategy, TsOptions};
use pons_ir::Document;
use support::options::{emitted, file, file_of, fixture, run_bun};

#[test]
fn errors_picks_which_result_a_call_answers_with() {
    let document: Document = serde_json::from_value(fixture()).unwrap();
    let neverthrow = emitted(
        &document,
        TsOptions {
            errors: ErrorStrategy::Neverthrow,
            ..TsOptions::default()
        },
    );
    let thrown = emitted(
        &document,
        TsOptions {
            errors: ErrorStrategy::Throw,
            ..TsOptions::default()
        },
    );

    assert!(
        file(&neverthrow, "errors.ts").contains("from \"neverthrow\""),
        "{}",
        file(&neverthrow, "errors.ts")
    );
    assert!(
        file(&thrown, "errors.ts").contains("type Result<T, E>")
            && !file(&thrown, "errors.ts").contains("neverthrow"),
        "the throwing shim declares the result type itself:\n{}",
        file(&thrown, "errors.ts")
    );
    assert!(
        file(&neverthrow, "operations/auth.ts")
            .contains("request<AuthLoginResponse, AuthLoginError>("),
        "{}",
        file(&neverthrow, "operations/auth.ts")
    );
    assert!(
        file(&thrown, "operations/auth.ts")
            .contains("requestOrThrow(request<AuthLoginResponse, AuthLoginError>(configured"),
        "{}",
        file(&thrown, "operations/auth.ts")
    );

    run_bun(
        &neverthrow,
        r#"
import { createClient } from "./client.ts";

const client = createClient({
  baseUrl: "http://example.test",
  fetch: async () => new Response("{}", { status: 200, headers: { "content-type": "application/json" } }),
});
const refused = await client.auth.login({ body: { email: "person@example.test" } });
if (!refused.isErr()) throw new Error("a missing field answered with a value");
if (refused.isOk()) throw new Error("the ok branch is reachable");
const failure: any = refused.error;
if (failure._tag !== "DecodeError" || !failure.issues.some((issue: string) => issue.includes("missing required field id"))) {
  throw new Error(JSON.stringify(failure));
}
"#,
    );
    run_bun(
        &thrown,
        r#"
import { createClient } from "./client.ts";

const client = createClient({
  baseUrl: "http://example.test",
  fetch: async () => new Response("{}", { status: 200, headers: { "content-type": "application/json" } }),
});
let thrown: any;
try {
  await client.auth.login({ body: { email: "person@example.test" } });
} catch (cause) {
  thrown = cause;
}
if (thrown === undefined) throw new Error("the call answered with a value");
if (thrown._tag !== "DecodeError" || !thrown.issues.some((issue: string) => issue.includes("missing required field id"))) {
  throw new Error(JSON.stringify(thrown));
}
"#,
    );
}

#[test]
fn an_operation_that_declares_no_error_still_names_the_transport_failure() {
    let document: Document = serde_json::from_value(fixture()).unwrap();
    assert!(document.operations[1].errors.is_empty());
    assert!(document.common_errors.is_empty());
    for errors in [ErrorStrategy::Neverthrow, ErrorStrategy::Throw] {
        let events = file_of(
            &document,
            TsOptions {
                errors,
                ..TsOptions::default()
            },
            "operations/events.ts",
        );
        assert!(
            events.contains("export type EventsWatchError = ClientFailure;"),
            "{errors:?} added a member a document with no declared error does not have:\n{events}"
        );
    }
}
