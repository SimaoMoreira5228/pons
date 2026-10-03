use std::{fs, process::Command};

use pons_emit_ts::{EmitMode, ErrorStrategy, GeneratedFile, emit_with_strategy};
use pons_ir::Document;
use serde_json::Value;

const TSCONFIG: &str = r#"{
  "compilerOptions": {
    "target": "es2022",
    "module": "esnext",
    "moduleResolution": "bundler",
    "lib": ["es2022", "dom", "dom.iterable"],
    "types": [],
    "strict": true,
    "exactOptionalPropertyTypes": true,
    "noUncheckedIndexedAccess": true,
    "skipLibCheck": true,
    "noEmit": true
  },
  "files": ["types.ts", "client.ts", "probe.ts"]
}
"#;

fn fixture() -> Value {
    serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap()
}

fn files(document: &Document) -> Vec<GeneratedFile> {
    emit_with_strategy(document, EmitMode::Strict, ErrorStrategy::Throw)
        .expect("the TypeScript emitter must accept the document")
        .files
}

fn types_of(files: &[GeneratedFile]) -> &str {
    &files
        .iter()
        .find(|file| file.path == "types.ts")
        .unwrap()
        .content
}

fn client_of(files: &[GeneratedFile]) -> &str {
    &files
        .iter()
        .find(|file| file.path == "client.ts")
        .unwrap()
        .content
}

fn tsc_command() -> (&'static str, Vec<String>) {
    for program in ["tsc", "bunx", "npx"] {
        let mut probe = Command::new(program);
        if program != "tsc" {
            probe.arg("tsc");
        }
        if let Ok(output) = probe.arg("--version").output()
            && output.status.success()
        {
            return (program, vec!["tsc".into()]);
        }
    }
    panic!("tsc is required for the generated TypeScript type tests");
}

fn type_checks(files: &[GeneratedFile], probe: &str) {
    let (program, arguments) = tsc_command();
    let directory = tempfile::tempdir().unwrap();
    for file in files {
        fs::write(directory.path().join(&file.path), &file.content).unwrap();
    }
    fs::write(directory.path().join("tsconfig.json"), TSCONFIG).unwrap();
    fs::write(directory.path().join("probe.ts"), probe).unwrap();
    let output = Command::new(program)
        .args(&arguments)
        .args(["--project", "tsconfig.json"])
        .current_dir(directory.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "tsc refused the emitted client\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn the_generated_client_compiles_under_the_strictest_flags() {
    let document: Document = serde_json::from_value(fixture()).unwrap();
    type_checks(
        &files(&document),
        r#"
import { createClient } from "./client.js";

const client = createClient({ baseUrl: "https://api.example.test" });
const loggedIn = await client.auth.login({ body: { email: "person@example.test" } });
const watched = await client.events.watch({ path: { cursor: "c" }, query: { limit: 1 } });
for await (const event of watched.value) {
  if (event.isOk() && event.value !== undefined) console.log(event.value.data.id);
}
console.log(loggedIn);
"#,
    );
}

#[test]
fn a_headers_bag_of_optional_headers_is_itself_optional() {
    let document: Document = serde_json::from_value(fixture()).unwrap();
    let files = files(&document);
    assert!(
        client_of(&files).contains("headers?: { \"X-Request-ID\"?: string }"),
        "{}",
        client_of(&files)
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
        client_of(&files).contains("headers: { \"X-Request-ID\": string }"),
        "{}",
        client_of(&files)
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

#[test]
fn a_user_type_never_shadows_the_syntax_the_emitter_prints() {
    let mut raw = fixture();
    raw["types"]["Array"] = serde_json::json!({
        "kind": "struct", "name": ["array"], "direction": "both", "openness": "reject",
        "fields": [{"wire": "a", "ident": ["a"], "ty": {"kind": "string"}, "presence": "required"}]
    });
    raw["types"]["Record"] = serde_json::json!({
        "kind": "struct", "name": ["record"], "direction": "both", "openness": "reject",
        "fields": [{"wire": "b", "ident": ["b"], "ty": {"kind": "string"}, "presence": "required"}]
    });
    raw["types"]["Extract"] = serde_json::json!({
        "kind": "struct", "name": ["extract"], "direction": "both", "openness": "reject",
        "fields": [{"wire": "c", "ident": ["c"], "ty": {"kind": "string"}, "presence": "required"}]
    });
    raw["types"]["Uint8Array"] = serde_json::json!({
        "kind": "struct", "name": ["uint8", "array"], "direction": "both", "openness": "reject",
        "fields": [{"wire": "d", "ident": ["d"], "ty": {"kind": "string"}, "presence": "required"}]
    });
    raw["types"]["Holder"] = serde_json::json!({
        "kind": "struct", "name": ["holder"], "direction": "output", "openness": "reject",
        "fields": [
            {"wire": "list", "ident": ["list"], "ty": {"kind": "array", "item": {"kind": "string"}}, "presence": "required"},
            {"wire": "dict", "ident": ["dict"], "ty": {"kind": "map", "key": {"kind": "string"}, "value": {"kind": "int", "bits": 16, "signed": false, "wire": "number"}}, "presence": "required"},
            {"wire": "blob", "ident": ["blob"], "ty": {"kind": "bytes", "encoding": "base64"}, "presence": "required"}
        ]
    });
    raw["operations"][0]["responses"][0]["body"] = serde_json::json!({"ref": "Holder"});
    raw["operations"][0]["provides"] = serde_json::Value::Null;
    let document: Document = serde_json::from_value(raw).unwrap();
    let files = files(&document);
    let types = types_of(&files);
    for reserved in ["Array", "Record", "Extract", "Uint8Array"] {
        assert!(
            !types.contains(&format!("export type {reserved} =")),
            "{reserved} kept the name the emitter prints it beside:\n{types}"
        );
    }
    for suffixed in ["Array2", "Record2", "Extract2", "Uint8Array2"] {
        assert!(
            types.contains(&format!("export type {suffixed} =")),
            "{suffixed} is missing:\n{types}"
        );
    }

    assert!(types.contains("\"blob\": Uint8Array;"), "{types}");
    type_checks(
        &files,
        r#"
import { createClient } from "./client.js";

const client = createClient({ baseUrl: "https://api.example.test" });
const value = await client.auth.login({ body: { email: "person@example.test" } });
const bytes: Uint8Array = value.blob;
const list: string[] = value.list;
const dict: Record<string, number> = value.dict;
// @ts-expect-error the bytes field is not the user's Uint8Array struct
const wrong: { d: string } = value.blob;
console.log(bytes.length, list, dict, wrong);
"#,
    );
}

#[test]
fn an_enum_struct_payload_is_open_like_the_runtime_treats_it() {
    let mut raw = fixture();
    raw["types"]["LoginError"]["variants"][1]["payload"]["fields"] = serde_json::json!([
        {"wire": "retry_after", "ident": ["retry", "after"], "ty": {"kind": "int", "bits": 32, "signed": false, "wire": "number"}, "presence": "required"},
        {"wire": "note", "ident": ["note"], "ty": {"kind": "string"}, "presence": "optional"}
    ]);
    let document: Document = serde_json::from_value(raw).unwrap();
    let files = files(&document);
    let types = types_of(&files);
    assert!(
        types.contains("\"note\"?: string; \"retry_after\": number; [key: string]: unknown }"),
        "{types}"
    );
    type_checks(
        &files,
        r#"
import type { LoginError } from "./types.js";

const locked: LoginError = { code: "locked", retry_after: 1, undeclared: "preserved" };
// @ts-expect-error a declared required payload field is still required
const missing: LoginError = { code: "locked", undeclared: "preserved" };
console.log(locked, missing);
"#,
    );
}

#[test]
fn a_narrowed_untagged_variant_carries_only_its_own_payload() {
    let mut raw = fixture();
    raw["types"]["Choice"] = serde_json::json!({
        "kind": "enum", "name": ["choice"], "direction": "output", "repr": {"kind": "untagged"},
        "variants": [
            {"wire": "label", "ident": ["label"], "payload": {"kind": "newtype", "ty": {"kind": "string"}}},
            {"wire": "count", "ident": ["count"], "payload": {"kind": "newtype", "ty": {"kind": "int", "bits": 16, "signed": false, "wire": "number"}}}
        ]
    });
    raw["operations"][0]["errors"][0]["body"] =
        serde_json::json!({"ref": "Choice", "variant": "label"});
    let document: Document = serde_json::from_value(raw).unwrap();
    let files = files(&document);
    assert!(
        client_of(&files)
            .contains("{ kind: \"http\"; status: number; body: string; headers: Headers }"),
        "{}",
        client_of(&files)
    );
    type_checks(
        &files,
        r#"
import { createClient, type AuthLoginError } from "./client.js";

const client = createClient({ baseUrl: "https://api.example.test" });
const failure: unknown = await client
  .auth.login({ body: { email: "person@example.test" } })
  .catch((cause: unknown) => cause);
if (typeof failure === "object" && failure !== null && "error" in failure) {
  const error = (failure as { error: AuthLoginError }).error;
  if (error.kind === "http") {
    const label: string = error.body;
    // @ts-expect-error the runtime only ever tries the variant the reference names
    const other: number = error.body;
    console.log(label, other);
  }
}
"#,
    );
}
