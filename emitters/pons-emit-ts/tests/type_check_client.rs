mod support;

use pons_ir::Document;

use support::type_check::{client_of, files, files_with, fixture, type_checks, type_checks_with};

const CLIENT_PROBE: &str = r#"
import { createClient } from "./client.js";

const client = createClient({ baseUrl: "https://api.example.test" });
const loggedIn = await client.auth.login({ body: { email: "person@example.test" } });
const watched = await client.events.watch({ path: { cursor: "c" }, query: { limit: 1 } });
for await (const event of watched.value) {
  if (event.isOk() && event.value !== undefined) console.log(event.value.data.id);
}
console.log(loggedIn);
"#;

#[test]
fn the_generated_client_compiles_under_the_strictest_flags() {
    let document: Document = serde_json::from_value(fixture()).unwrap();
    type_checks(&files(&document), CLIENT_PROBE);
}

#[test]
fn a_client_that_imports_the_runtime_package_compiles_beside_it() {
    let document: Document = serde_json::from_value(fixture()).unwrap();
    let files = files_with(
        &document,
        pons_emit_ts::TsOptions {
            runtime: pons_emit_ts::Runtime::Package,
            ..pons_emit_ts::TsOptions::default()
        },
    );
    assert!(
        files.iter().all(|file| !file.path.starts_with("runtime")),
        "the package supplies the runtime, so the emitter writes none of it: {:?}",
        files.iter().map(|file| &file.path).collect::<Vec<_>>()
    );
    type_checks_with(&files, CLIENT_PROBE, true);
}

#[test]
fn a_flat_client_is_reachable_through_the_quoted_member() {
    let document: Document = serde_json::from_value(fixture()).unwrap();
    let files = files_with(
        &document,
        pons_emit_ts::TsOptions {
            namespaces: pons_emit_ts::Namespaces::Flat,
            ..pons_emit_ts::TsOptions::default()
        },
    );
    let client = client_of(&files);
    assert!(
        client.contains(
            "import { operations as auth_loginOperations } from \"./operations/auth.login.js\";"
        ),
        "the binding is an identifier the dotted member is not:\n{client}"
    );
    type_checks(
        &files,
        r#"
import { createClient } from "./client.js";

const client = createClient({ baseUrl: "https://api.example.test" });
export const loggedIn = await client["auth.login"]({ body: { email: "person@example.test" } });
export const watched = await client["events.watch"]({ path: { cursor: "c" }, query: { limit: 1 } });
for await (const event of watched.value) {
  if (event.isOk() && event.value !== undefined) console.log(event.value.data.id);
}
// @ts-expect-error the namespace is part of the member name
export const nested = client.auth.login;
console.log(loggedIn, nested);
"#,
    );
}

#[test]
fn the_client_options_are_named_as_the_design_names_them() {
    let document: Document = serde_json::from_value(fixture()).unwrap();
    let files = files(&document);
    type_checks(
        &files,
        r#"
import { createClient, type CallOptions, type ClientOptions, type RetryOptions } from "./client.js";

const retry: RetryOptions = { attempts: 2, backoffMs: 10, maxBackoffMs: 100, jitterMs: 5, force: true };
const options: ClientOptions = {
  baseUrl: "https://api.example.test",
  timeoutMs: 1_000,
  codecs: {},
  retry,
  transport: {
    onRequest: async ({ url, init }) => {
      url.searchParams.set("trace", "1");
      init.headers = { "X-Trace": "1" };
    },
    onResponse: ({ response, url, init }) => {
      console.log(response.status, url.host, init.method);
    },
  },
};
const call: CallOptions = { timeoutMs: 50, retry: { attempts: 1 } };
export const client = createClient(options);
export const loggedIn = client.auth.login({ body: { email: "person@example.test" } }, call);
// @ts-expect-error the timeout option is spelled timeoutMs
const oldTimeout: ClientOptions = { baseUrl: "https://api.example.test", timeout: 1_000 };
// @ts-expect-error the codec registry is spelled codecs
const oldCodecs: ClientOptions = { baseUrl: "https://api.example.test", opaqueCodecs: {} };
// @ts-expect-error a call option is the same name as the client option
const oldCall: CallOptions = { timeout: 50 };
// @ts-expect-error force is a boolean, not the string the caller typed
const wrongForce: RetryOptions = { force: "yes" };
// @ts-expect-error the hooks live under transport
const oldHooks: ClientOptions = { baseUrl: "https://api.example.test", interceptors: { request: () => {} } };
console.log(oldTimeout, oldCodecs, oldCall, oldHooks, wrongForce);
"#,
    );
}
