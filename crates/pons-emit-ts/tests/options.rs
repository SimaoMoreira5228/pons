use std::{fs, path::Path, process::Command};

use pons_emit_ts::{
    DateMode, EmitMode, ErrorStrategy, FieldCase, GeneratedFile, Int64, Namespaces, Newtypes,
    Runtime, TsOptions, Validation, emit_with_options, emit_with_strategy,
};
use pons_ir::Document;
use serde_json::{Value, json};

fn fixture() -> Value {
    serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap()
}

fn emitted(document: &Document, options: TsOptions) -> Vec<GeneratedFile> {
    emit_with_options(document, options)
        .expect("the TypeScript emitter must accept the document")
        .files
}

fn file<'a>(files: &'a [GeneratedFile], path: &str) -> &'a str {
    &files
        .iter()
        .find(|file| file.path == path)
        .unwrap_or_else(|| panic!("the emitter wrote no {path}"))
        .content
}

fn file_of(document: &Document, options: TsOptions, path: &str) -> String {
    let files = emitted(document, options);
    file(&files, path).to_owned()
}

fn run_bun(files: &[GeneratedFile], harness: &str) {
    run_bun_with(files, harness, Runtime::Vendor)
}

fn run_bun_with(files: &[GeneratedFile], harness: &str, runtime: Runtime) {
    let version = Command::new("bun")
        .arg("--version")
        .output()
        .expect("Bun is required for generated TypeScript runtime tests");
    assert!(version.status.success());
    let directory = tempfile::tempdir().unwrap();
    for file in files {
        let destination = directory.path().join(&file.path);
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(destination, &file.content).unwrap();
    }
    if runtime == Runtime::Package {
        install_runtime_package(directory.path());
    }
    let harness_path = directory.path().join("options.ts");
    fs::write(&harness_path, harness).unwrap();
    let output = Command::new("bun")
        .arg("run")
        .arg(&harness_path)
        .current_dir(directory.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn install_runtime_package(output: &Path) {
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../runtime/ts");
    let installed = output.join("runtime");
    fs::create_dir_all(&installed).unwrap();
    for entry in fs::read_dir(source).unwrap() {
        let entry = entry.unwrap();
        let name = entry.file_name();
        if name.to_string_lossy().ends_with(".ts") || name == "package.json" {
            fs::copy(entry.path(), installed.join(&name)).unwrap();
        }
    }
    let scope = output.join("node_modules").join("@pons");
    fs::create_dir_all(&scope).unwrap();
    let link = scope.join("runtime");
    #[cfg(unix)]
    std::os::unix::fs::symlink(&installed, &link).unwrap();
    #[cfg(windows)]
    std::os::windows::fs::symlink_dir(&installed, &link).unwrap();
}

fn operation(id: Value, path: &str) -> Value {
    json!({
        "id": id,
        "method": "GET",
        "path": [{"lit": path}],
        "auth": {"alternatives": [], "optional": false},
        "semantics": {"safe": true, "idempotent": true},
        "responses": [{"status": 200, "content": "json", "body": {"kind": "string"}}]
    })
}

fn with_operations(operations: Vec<Value>) -> Document {
    let raw = json!({
        "ir_version": pons_ir::IR_VERSION,
        "info": {"title": "Options", "version": "1"},
        "operations": operations,
    });
    let document: Document = serde_json::from_value(raw).unwrap();
    document.validate().unwrap();
    document
}

fn two_namespaces() -> Document {
    with_operations(vec![
        operation(json!([["a"], ["get"]]), "a"),
        operation(json!([["b"], ["get"]]), "b"),
    ])
}

#[test]
fn the_default_options_emit_the_bytes_the_three_parameters_did() {
    let document: Document = serde_json::from_value(fixture()).unwrap();
    let default = TsOptions::default();
    for (mode, errors) in [
        (EmitMode::Strict, ErrorStrategy::Neverthrow),
        (EmitMode::Strict, ErrorStrategy::Throw),
        (EmitMode::Lossy, ErrorStrategy::Neverthrow),
        (EmitMode::Lossy, ErrorStrategy::Throw),
    ] {
        assert_eq!(
            emit_with_options(
                &document,
                TsOptions {
                    mode,
                    errors,
                    ..default
                }
            )
            .unwrap(),
            emit_with_strategy(&document, mode, errors).unwrap(),
            "the default {mode:?}/{errors:?} is the output the two parameters produced"
        );
    }
    assert_eq!(
        default,
        TsOptions {
            mode: EmitMode::Strict,
            errors: ErrorStrategy::Neverthrow,
            namespaces: Namespaces::Nested,
            field_case: FieldCase::Wire,
            newtypes: Newtypes::Alias,
            int64: Int64::Bigint,
            date_mode: DateMode::String,
            runtime: Runtime::Vendor,
            validation: Validation::None,
        },
        "every option has one default and the default is the output of today"
    );
}

#[test]
fn the_default_options_leave_the_vendored_runtime_the_bytes_it_was() {
    let document: Document = serde_json::from_value(fixture()).unwrap();
    let files = emitted(&document, TsOptions::default());
    for module in [
        "runtime/wire.ts",
        "runtime/codec.ts",
        "runtime/stream.ts",
        "runtime/credentials.ts",
        "runtime/transport.ts",
    ] {
        let source = fs::read_to_string(format!(
            "{}/../../runtime/ts/{}",
            env!("CARGO_MANIFEST_DIR"),
            module.strip_prefix("runtime/").unwrap()
        ))
        .unwrap();
        assert_eq!(
            file(&files, module),
            source,
            "{module} is the file this repository holds"
        );
    }
    let types = file(&files, "types.ts");
    assert!(
        !types.contains("DateOnly") && types.contains("export type User = { \"id\": string"),
        "the default options add nothing to the type the caller reads:\n{types}"
    );
    assert!(
        !types.contains("\"carrier\""),
        "no schema in the default metadata carries an option's spelling:\n{types}"
    );
}

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
if (failure.kind !== "protocol" || !failure.message.includes("missing required field id")) {
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
if (thrown.kind !== "protocol" || !thrown.message.includes("missing required field id")) {
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

fn tagged_document() -> Document {
    let mut raw = fixture();
    raw["types"]["Tag"] = json!({
        "kind": "newtype", "name": ["tag"], "direction": "both", "inner": {"kind": "string"}
    });
    raw["types"]["Session"] = json!({
        "kind": "struct", "name": ["session"], "direction": "both", "openness": "reject",
        "fields": [
            {"wire": "tag", "ident": ["tag"], "ty": {"ref": "Tag"}, "presence": "required"},
            {"wire": "alias", "ident": ["alias"], "ty": {"ref": "Tag"}, "presence": "required"}
        ]
    });
    raw["types"]["LoginRequest"] = json!({
        "kind": "struct", "name": ["login", "request"], "direction": "input", "openness": "reject",
        "fields": [
            {"wire": "tag", "ident": ["tag"], "ty": {"ref": "Tag"}, "presence": "required"}
        ]
    });
    raw["operations"][0]["responses"][0]["body"] = json!({"ref": "Session"});
    raw["operations"][0]["provides"] = Value::Null;
    let document: Document = serde_json::from_value(raw).unwrap();
    document.validate().unwrap();
    document
}

#[test]
fn a_newtype_is_an_alias_today_and_nominal_under_the_branded_option() {
    let document = tagged_document();
    let alias = file_of(&document, TsOptions::default(), "types.ts");
    assert!(
        alias.contains("export type Tag = string;") && !alias.contains("brandTag"),
        "{alias}"
    );

    let branded = file_of(
        &document,
        TsOptions {
            newtypes: Newtypes::Branded,
            ..TsOptions::default()
        },
        "types.ts",
    );
    assert!(
        branded.contains("declare const brandTag: unique symbol;\nexport type Tag = string & { readonly [brandTag]: never };"),
        "{branded}"
    );
    assert!(
        branded.contains("\"alias\": Tag; \"tag\": Tag"),
        "a field keeps the newtype it declares:\n{branded}"
    );
    assert!(
        branded.contains("\"tag\": Tag };"),
        "a request argument takes the branded type too:\n{branded}"
    );
    assert!(
        branded.contains("export const typeDefinitions"),
        "the wire metadata the runtime reads is the wire, so the brand does not reach it:\n{branded}"
    );

    let files = emitted(
        &document,
        TsOptions {
            newtypes: Newtypes::Branded,
            errors: ErrorStrategy::Throw,
            ..TsOptions::default()
        },
    );
    run_bun(
        &files,
        r#"
import { createClient } from "./client.ts";

let sent = "";
const client = createClient({
  baseUrl: "http://example.test",
  fetch: async (_url, init) => {
    sent = String(init?.body);
    return new Response('{"tag":"from-the-wire","alias":"second"}', {
      status: 200,
      headers: { "content-type": "application/json" },
    });
  },
});
const session = await client.auth.login({ body: { tag: "from-the-caller" } });
if (sent !== '{"tag":"from-the-caller"}') throw new Error(`the wire carries ${sent}`);
if (typeof session.tag !== "string" || session.tag !== "from-the-wire") {
  throw new Error(`a brand changed the value: ${String(session.tag)}`);
}
if (session.tag === session.alias) throw new Error("the response was not the declared one");
"#,
    );
}

fn wide_document() -> Document {
    let mut raw = fixture();
    raw["types"]["User"] = json!({
        "kind": "struct", "name": ["user"], "direction": "output", "openness": "reject",
        "fields": [
            {"wire": "count", "ident": ["count"], "ty": {"kind": "int", "bits": 64, "signed": false, "wire": "number"}, "presence": "required"}
        ]
    });
    raw["types"]["LoginRequest"] = json!({
        "kind": "struct", "name": ["login", "request"], "direction": "input", "openness": "reject",
        "fields": [
            {"wire": "count", "ident": ["count"], "ty": {"kind": "int", "bits": 64, "signed": false, "wire": "number"}, "presence": "required"}
        ]
    });
    raw["operations"][0]["provides"] = Value::Null;
    let document: Document = serde_json::from_value(raw).unwrap();
    document.validate().unwrap();
    document
}

#[test]
fn int64_bigint_holds_a_wire_number_a_javascript_number_cannot() {
    let document = wide_document();
    let files = emitted(
        &document,
        TsOptions {
            errors: ErrorStrategy::Throw,
            ..TsOptions::default()
        },
    );
    let types = file(&files, "types.ts");
    assert!(types.contains("\"count\": bigint"), "{types}");
    assert!(
        file(&files, "operations/auth.ts").contains("preserveWideIntegers: true"),
        "the lossless reader is the only way a bigint holds every digit:\n{}",
        file(&files, "operations/auth.ts")
    );
    run_bun(
        &files,
        r#"
import { createClient } from "./client.ts";

let sent = "";
const client = createClient({
  baseUrl: "http://example.test",
  fetch: async (_url, init) => {
    sent = String(init?.body);
    return new Response('{"count":9007199254740993}', {
      status: 200,
      headers: { "content-type": "application/json" },
    });
  },
});
const value = await client.auth.login({ body: { count: 9007199254740993n } });
if (sent !== '{"count":9007199254740993}') throw new Error(`the wire carries ${sent}`);
if (typeof value.count !== "bigint") throw new Error(`the answer is a ${typeof value.count}`);
if (value.count !== 9007199254740993n) throw new Error(`the answer is ${value.count}`);
"#,
    );
}

#[test]
fn int64_number_refuses_a_wide_integer_in_strict_mode_and_loses_it_in_lossy() {
    let document = wide_document();
    let number = TsOptions {
        int64: Int64::Number,
        ..TsOptions::default()
    };
    let refused = emit_with_options(&document, number).unwrap_err();
    assert_eq!(
        refused.0,
        "the TypeScript emitter reported lossy coverage in strict mode: \
         operations.auth.login, operations.events.watch, types.LoginRequest, types.User",
        "a number cannot hold every digit, so strict mode says so instead of emitting one:\n{}",
        refused.0
    );

    let lossy = emit_with_options(
        &document,
        TsOptions {
            mode: EmitMode::Lossy,
            errors: ErrorStrategy::Throw,
            ..number
        },
    )
    .unwrap();
    assert!(
        lossy
            .warnings
            .iter()
            .any(|warning| warning.contains("wide numeric integers as number")),
        "{:?}",
        lossy.warnings
    );
    let files = lossy.files;
    assert!(file(&files, "types.ts").contains("\"count\": number"));
    assert!(
        !file(&files, "operations/auth.ts").contains("preserveWideIntegers"),
        "the lossless reader is what a number cannot use:\n{}",
        file(&files, "operations/auth.ts")
    );
    run_bun(
        &files,
        r#"
import { createClient } from "./client.ts";

let sent = "";
const client = createClient({
  baseUrl: "http://example.test",
  fetch: async (_url, init) => {
    sent = String(init?.body);
    return new Response('{"count":9007199254740993}', {
      status: 200,
      headers: { "content-type": "application/json" },
    });
  },
});
const value = await client.auth.login({ body: { count: 9007199254740993 } });
if (typeof value.count !== "number") throw new Error(`the answer is a ${typeof value.count}`);
if (value.count !== 9007199254740992) throw new Error(`the answer is ${value.count}`);
if (sent !== '{"count":9007199254740992}') {
  throw new Error(`the literal 9007199254740993 is already 9007199254740992, so the request rounds too: ${sent}`);
}
"#,
    );
}

#[test]
fn the_zod_option_writes_the_schemas_the_response_is_checked_against() {
    let document: Document = serde_json::from_value(fixture()).unwrap();
    let zod = emitted(
        &document,
        TsOptions {
            validation: Validation::Zod,
            ..TsOptions::default()
        },
    );
    let schemas = file(&zod, "schemas.ts");
    assert!(schemas.contains("import { z } from \"zod\";"), "{schemas}");
    assert!(
        schemas.contains("export const UserSchema"),
        "a named type carries a schema: {schemas}"
    );
    assert!(
        schemas.contains("export function mismatch(schema: z.ZodType)"),
        "the check the client calls is exported beside the schemas: {schemas}"
    );
    let operations = file(&zod, "operations/auth.ts");
    assert!(
        operations.contains("import { mismatch,"),
        "the operation file imports what it calls: {operations}"
    );
    assert!(
        operations.contains("validate: mismatch(UserSchema)"),
        "a declared response carries the check: {operations}"
    );

    assert!(
        file(&zod, "index.ts").contains("export * from \"./schemas.js\";"),
        "the entry point reaches the schemas it checked against: {}",
        file(&zod, "index.ts")
    );

    let none = emitted(&document, TsOptions::default());
    assert!(!paths(&none).contains(&"schemas.ts".to_owned()));
    assert!(
        !file(&none, "operations/auth.ts").contains("mismatch"),
        "the default writes the bytes it wrote before the option did anything"
    );
    assert_eq!(
        file(&none, "index.ts"),
        "export * from \"./client.js\";\n",
        "the default's entry point is one line"
    );
    assert_eq!(
        paths(&none),
        paths(&zod)
            .into_iter()
            .filter(|path| path != "schemas.ts")
            .collect::<Vec<_>>(),
        "the schema file is the only file the option adds"
    );
}

fn paths(files: &[GeneratedFile]) -> Vec<String> {
    let mut paths = files
        .iter()
        .map(|file| file.path.clone())
        .collect::<Vec<_>>();
    paths.sort();
    paths
}

#[test]
fn runtime_package_writes_no_runtime_and_imports_the_package_instead() {
    let document: Document = serde_json::from_value(fixture()).unwrap();
    let vendored_options = TsOptions {
        errors: ErrorStrategy::Throw,
        ..TsOptions::default()
    };
    let vendor = emitted(&document, vendored_options);
    let package = emitted(
        &document,
        TsOptions {
            runtime: Runtime::Package,
            ..vendored_options
        },
    );
    assert_eq!(
        paths(&vendor),
        [
            "client.ts",
            "errors.ts",
            "index.ts",
            "operations/auth.ts",
            "operations/events.ts",
            "runtime.ts",
            "runtime/codec.ts",
            "runtime/credentials.ts",
            "runtime/index.ts",
            "runtime/stream.ts",
            "runtime/transport.ts",
            "runtime/wire.ts",
            "types.ts",
        ]
        .map(String::from)
        .to_vec(),
        "the default writes the runtime"
    );
    assert_eq!(
        paths(&package),
        [
            "client.ts",
            "errors.ts",
            "index.ts",
            "operations/auth.ts",
            "operations/events.ts",
            "types.ts",
        ]
        .map(String::from)
        .to_vec(),
        "the package supplies the runtime, so the emitter writes neither it nor its entry point"
    );
    const MARKER: &str = "// runtime = \"package\": the runtime is the @pons/runtime package beside this file, not the vendored runtime/*.ts the default writes here\n";
    for emitted_file in &package {
        let swapped = file(&vendor, &emitted_file.path)
            .replace("from \"../runtime.js\"", "from \"@pons/runtime\"")
            .replace("from \"./runtime.js\"", "from \"@pons/runtime\"");
        let expected = if emitted_file.path == "client.ts" {
            format!("{MARKER}{swapped}")
        } else {
            swapped
        };
        assert_eq!(
            emitted_file.content, expected,
            "only the runtime import changes, and the client names the mode: {}",
            emitted_file.path
        );
    }
    assert!(
        file(&package, "client.ts").starts_with(MARKER),
        "the file set no longer says which mode wrote it, so the client says it: {}",
        file(&package, "client.ts")
    );
    assert!(
        file(&package, "client.ts").contains(
            "import type { ConfiguredClient, CredentialValue, OpaqueCodecRegistry } from \
             \"@pons/runtime\";"
        ) && file(&package, "client.ts").contains(
            "export type { OpaqueCodec, OpaqueCodecRegistry, OpaqueWire } from \"@pons/runtime\";"
        ),
        "{}",
        file(&package, "client.ts")
    );
    assert!(
        file(&package, "operations/auth.ts").contains(
            "import { request, requestOrThrow, encodePathSegment, type ConfiguredClient } from \
             \"@pons/runtime\";"
        ),
        "{}",
        file(&package, "operations/auth.ts")
    );
    for emitted_file in &package {
        assert!(
            !emitted_file.content.contains("runtime.js"),
            "{} still names the vendored file: {}",
            emitted_file.path,
            emitted_file.content
        );
    }
    run_bun_with(
        &package,
        r#"
import { createClient } from "./client.js";

const client = createClient({
  baseUrl: "http://example.test",
  fetch: async () => new Response("{}", { status: 200, headers: { "content-type": "application/json" } }),
});
let failure: any;
try {
  await client.auth.login({ body: { email: "person@example.test" } });
} catch (cause) {
  failure = cause;
}
if (failure?.kind !== "protocol" || !failure.message.includes("missing required field id")) {
  throw new Error(`the wire validator in the package did not refuse it: ${JSON.stringify(failure)}`);
}
"#,
        Runtime::Package,
    );
}

fn camel_document() -> Document {
    let fields = json!([
        {"wire": "user_id", "ident": ["user", "id"], "ty": {"kind": "string"}, "presence": "required"},
        {"wire": "retry_after", "ident": ["retry", "after"], "ty": {"kind": "int", "bits": 32, "signed": false, "wire": "number"}, "presence": "required"}
    ]);
    let mut raw = fixture();
    raw["types"]["User"] = json!({
        "kind": "struct", "name": ["user"], "direction": "output", "openness": "preserve_opaque", "fields": fields
    });
    raw["types"]["LoginRequest"] = json!({
        "kind": "struct", "name": ["login", "request"], "direction": "input", "openness": "reject", "fields": fields
    });
    raw["operations"][0]["provides"] = Value::Null;
    let document: Document = serde_json::from_value(raw).unwrap();
    document.validate().unwrap();
    document
}

#[test]
fn field_case_camel_names_a_member_camel_and_keeps_the_wire_name() {
    let document = camel_document();
    let files = emitted(
        &document,
        TsOptions {
            field_case: FieldCase::Camel,
            errors: ErrorStrategy::Throw,
            ..TsOptions::default()
        },
    );
    assert!(
        file(&files, "types.ts")
            .contains("export type User = { \"retryAfter\": number; \"userId\": string; [key: string]: unknown };"),
        "{}",
        file(&files, "types.ts")
    );
    assert!(
        file(&files, "types.ts").contains("\\\"key\\\":\\\"userId\\\""),
        "the runtime reads the member it is given:\n{}",
        file(&files, "types.ts")
    );
    assert!(
        file(&files, "types.ts").contains("export type EventQuery = { \"limit\"?: number };"),
        "a member that is already camel does not change:\n{}",
        file(&files, "types.ts")
    );
    run_bun(
        &files,
        r#"
import { createClient } from "./client.ts";

let sent = "";
const client = createClient({
  baseUrl: "http://example.test",
  fetch: async (_url, init) => {
    sent = String(init?.body);
    return new Response('{"user_id":"u-1","retry_after":30,"trace_id":"t-1"}', {
      status: 200,
      headers: { "content-type": "application/json" },
    });
  },
});
const value = await client.auth.login({ body: { userId: "u-1", retryAfter: 30 } });
if (sent !== '{"retry_after":30,"user_id":"u-1"}') throw new Error(`the wire carries ${sent}`);
if (value.userId !== "u-1" || value.retryAfter !== 30) {
  throw new Error(`the answer is ${JSON.stringify(value)}`);
}
if (value.trace_id !== "t-1") throw new Error("an undeclared member did not survive");
if ("user_id" in value) throw new Error("the answer kept the wire spelling");
"#,
    );
    run_bun(
        &files,
        r#"
import { createClient } from "./client.ts";

const client = createClient({
  baseUrl: "http://example.test",
  fetch: async () => new Response('{"user_id":"u-1","retry_after":30}', {
    status: 200,
    headers: { "content-type": "application/json" },
  }),
});
let failure: any;
try {
  // @ts-ignore the member the caller can name is userId
  await client.auth.login({ body: { user_id: "u-1", retry_after: 30 } });
} catch (cause) {
  failure = cause;
}
if (failure === undefined) throw new Error("a wire name in the body was accepted");
if (failure.kind !== "protocol" || !failure.message.includes("unknown property user_id")) {
  throw new Error(JSON.stringify(failure));
}
"#,
    );
}

#[test]
fn field_case_camel_refuses_two_members_that_spell_the_same_way() {
    let mut raw = fixture();
    raw["types"]["User"] = json!({
        "kind": "struct", "name": ["user"], "direction": "output", "openness": "reject",
        "fields": [
            {"wire": "retry_after", "ident": ["retry", "after"], "ty": {"kind": "string"}, "presence": "required"},
            {"wire": "retryAfter", "ident": ["retry", "after"], "ty": {"kind": "string"}, "presence": "required"}
        ]
    });
    let document: Document = serde_json::from_value(raw).unwrap();
    document.validate().unwrap();
    let error = emit_with_options(
        &document,
        TsOptions {
            field_case: FieldCase::Camel,
            ..TsOptions::default()
        },
    )
    .unwrap_err();
    assert_eq!(
        error.0,
        "the TypeScript emitter cannot spell the fields of the type User in camelCase, because \
         the wire names retryAfter and retry_after both spell retryAfter in the emitted client"
    );
    assert!(
        emit_with_options(&document, TsOptions::default()).is_ok(),
        "the two members are only one member under the option that keeps the wire name"
    );
}

#[test]
fn int64_string_carries_a_wide_integer_as_a_canonical_decimal_string() {
    let document = wide_document();
    let files = emitted(
        &document,
        TsOptions {
            int64: Int64::String,
            errors: ErrorStrategy::Throw,
            ..TsOptions::default()
        },
    );
    let types = file(&files, "types.ts");
    assert!(types.contains("\"count\": string"), "{types}");
    assert!(
        types.contains("\\\"carrier\\\":\\\"string\\\""),
        "the runtime is told which representation the caller holds:\n{types}"
    );
    assert!(
        file(&files, "operations/auth.ts").contains("preserveWideIntegers: true"),
        "the lossless reader is the only way a decimal string holds every digit:\n{}",
        file(&files, "operations/auth.ts")
    );
    run_bun(
        &files,
        r#"
import { createClient } from "./client.ts";

let sent = "";
const client = createClient({
  baseUrl: "http://example.test",
  fetch: async (_url, init) => {
    sent = String(init?.body);
    return new Response('{"count":9007199254740993}', {
      status: 200,
      headers: { "content-type": "application/json" },
    });
  },
});
const value = await client.auth.login({ body: { count: "9007199254740993" } });
if (sent !== '{"count":9007199254740993}') throw new Error(`the wire carries ${sent}, a JSON string`);
if (typeof value.count !== "string" || value.count !== "9007199254740993") {
  throw new Error(`the answer is ${String(value.count)}`);
}
if (BigInt(value.count) !== 9007199254740993n) throw new Error("the answer lost a digit");
"#,
    );
    run_bun(
        &files,
        r#"
import { createClient } from "./client.ts";

const client = createClient({
  baseUrl: "http://example.test",
  fetch: async () => new Response("{}", { status: 200, headers: { "content-type": "application/json" } }),
});
for (const count of ["9007199254740993.0", " 12", "12 ", "+1", "", "0x10"]) {
  let failure: any;
  try {
    await client.auth.login({ body: { count } });
  } catch (cause) {
    failure = cause;
  }
  if (failure?.message !== "expected integer string") {
    throw new Error(`${JSON.stringify(count)} was accepted: ${JSON.stringify(failure)}`);
  }
}
let number: any;
try {
  // @ts-ignore the option says the member is a string
  await client.auth.login({ body: { count: 9007199254740993 } });
} catch (cause) {
  number = cause;
}
if (number?.message !== "expected integer string") throw new Error(JSON.stringify(number));
"#,
    );
}

fn date_document() -> Document {
    let fields = json!([
        {"wire": "day", "ident": ["day"], "ty": {"kind": "string", "format": "date"}, "presence": "required"},
        {"wire": "at", "ident": ["at"], "ty": {"kind": "string", "format": "date-time"}, "presence": "required"}
    ]);
    let mut raw = fixture();
    raw["types"]["User"] = json!({
        "kind": "struct", "name": ["user"], "direction": "output", "openness": "reject", "fields": fields
    });
    raw["types"]["LoginRequest"] = json!({
        "kind": "struct", "name": ["login", "request"], "direction": "input", "openness": "reject", "fields": fields
    });
    raw["operations"][0]["provides"] = Value::Null;
    let document: Document = serde_json::from_value(raw).unwrap();
    document.validate().unwrap();
    document
}

#[test]
fn date_mode_date_reads_a_day_and_an_instant_differently_and_writes_both_back() {
    let document = date_document();
    let files = emitted(
        &document,
        TsOptions {
            date_mode: DateMode::Date,
            ..TsOptions::default()
        },
    );
    let types = file(&files, "types.ts");
    assert!(
        types.contains("export type DateOnly = string & { readonly [brandDateOnly]: never };"),
        "a calendar day is a branded string, because no Date can hold a day:\n{types}"
    );
    assert!(
        types.contains("export type User = { \"at\": Date; \"day\": DateOnly };"),
        "{types}"
    );
    assert!(
        types.contains("\\\"carrier\\\":\\\"date\\\""),
        "the runtime is told the date-time is an instant:\n{types}"
    );
    run_bun(
        &files,
        r#"
import { createClient } from "./client.ts";
import type { DateOnly } from "./types.ts";

let sent = "";
let answer = '{"day":"2024-01-31","at":"2024-01-31T12:00:00.000Z"}';
const client = createClient({
  baseUrl: "http://example.test",
  fetch: async (_url, init) => {
    sent = String(init?.body);
    return new Response(answer, { status: 200, headers: { "content-type": "application/json" } });
  },
});
const first = await client.auth.login({
  body: { day: "2024-01-31" as DateOnly, at: new Date("2024-01-31T12:00:00Z") },
});
if (!first.isOk()) throw new Error(`the call failed: ${JSON.stringify(first.error)}`);
if (first.value.day !== "2024-01-31") throw new Error(`the day is ${String(first.value.day)}`);
if (!(first.value.at instanceof Date)) throw new Error("the instant is not a Date");
if (first.value.at.toISOString() !== "2024-01-31T12:00:00.000Z") {
  throw new Error(`the instant is ${first.value.at.toISOString()}`);
}
if (sent !== '{"at":"2024-01-31T12:00:00.000Z","day":"2024-01-31"}') {
  throw new Error(`the wire carries ${sent}`);
}

answer = '{"day":"2024-01-31","at":"2024-01-31T12:00:00.123456Z"}';
const finer = await client.auth.login({ body: { day: "2024-01-31" as DateOnly, at: new Date() } });
if (finer.isOk()) throw new Error("a nanosecond date-time was read as a Date");
if (!(finer as any).error.message.includes("not an instant a Date can hold")) {
  throw new Error(JSON.stringify(finer));
}
"#,
    );
}

#[test]
fn date_mode_date_refuses_a_day_the_option_does_not_carry_as_a_date() {
    let document = date_document();
    let files = emitted(
        &document,
        TsOptions {
            date_mode: DateMode::Date,
            errors: ErrorStrategy::Throw,
            ..TsOptions::default()
        },
    );
    run_bun(
        &files,
        r#"
import { createClient } from "./client.ts";

const client = createClient({
  baseUrl: "http://example.test",
  fetch: async () => new Response("{}", { status: 200, headers: { "content-type": "application/json" } }),
});
let asText: any;
try {
  await client.auth.login({ body: { day: "2024-01-31" as never, at: "2024-01-31T12:00:00Z" as never } });
} catch (cause) {
  asText = cause;
}
if (asText?.message !== "expected a Date") throw new Error(`a date-time went out as text: ${JSON.stringify(asText)}`);

let asDate: any;
try {
  await client.auth.login({ body: { day: new Date() as never, at: new Date() } });
} catch (cause) {
  asDate = cause;
}
if (asDate?.message !== "expected string") throw new Error(`a day went out as a Date: ${JSON.stringify(asDate)}`);
"#,
    );
}

#[test]
fn a_declared_member_is_carried_once_when_its_key_changes_under_a_typed_openness() {
    let mut raw = fixture();
    for (id, name, direction) in [
        ("User", "user", "output"),
        ("LoginRequest", "login request", "input"),
    ] {
        let name = name
            .split(' ')
            .map(|word| word.to_string())
            .collect::<Vec<_>>();
        raw["types"][id] = json!({
            "kind": "struct", "name": name, "direction": direction,
            "openness": {"preserve_typed": {"value": {"kind": "string", "format": "date-time"}}},
            "fields": [
                {"wire": "seen_at", "ident": ["seen", "at"], "ty": {"kind": "string", "format": "date-time"}, "presence": "required"}
            ]
        });
    }
    raw["operations"][0]["provides"] = Value::Null;
    let document: Document = serde_json::from_value(raw).unwrap();
    document.validate().unwrap();
    let files = emitted(
        &document,
        TsOptions {
            field_case: FieldCase::Camel,
            date_mode: DateMode::Date,
            errors: ErrorStrategy::Throw,
            ..TsOptions::default()
        },
    );
    assert!(
        file(&files, "types.ts")
            .contains("export type User = { \"seenAt\": Date; [key: string]: Date };"),
        "{}",
        file(&files, "types.ts")
    );
    run_bun(
        &files,
        r#"
import { createClient } from "./client.ts";

let sent = "";
const client = createClient({
  baseUrl: "http://example.test",
  fetch: async (_url, init) => {
    sent = String(init?.body);
    return new Response('{"seen_at":"2024-01-31T12:00:00Z","also_at":"2024-01-31T12:00:00Z"}', {
      status: 200,
      headers: { "content-type": "application/json" },
    });
  },
});
const value = await client.auth.login({ body: { seenAt: new Date("2024-01-31T12:00:00Z") } });
if (sent !== '{"seen_at":"2024-01-31T12:00:00.000Z"}') throw new Error(`the wire carries ${sent}`);
if (!(value.seenAt instanceof Date)) throw new Error("the declared member is not a Date");
if (!(value.also_at instanceof Date) || value.also_at.toISOString() !== "2024-01-31T12:00:00.000Z") {
  throw new Error(`an undeclared member is a ${typeof value.also_at}`);
}
"#,
    );
}
