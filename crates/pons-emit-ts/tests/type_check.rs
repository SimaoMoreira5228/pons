use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use pons_emit_ts::{EmitMode, ErrorStrategy, GeneratedFile, emit_with_strategy};
use pons_ir::Document;
use serde_json::Value;

const PROBE: &str = "probe.ts";
const PROJECT: &str = "tsconfig.json";

const COMPILER_OPTIONS: &str = r#"{
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
  }"#;

fn tsconfig(checked: &[String]) -> String {
    format!(
        r#"{{
  "compilerOptions": {COMPILER_OPTIONS},
  "files": [{}]
}}
"#,
        checked
            .iter()
            .map(|path| serde_json::to_string(path).unwrap())
            .collect::<Vec<_>>()
            .join(", ")
    )
}

fn checked_files(files: &[GeneratedFile]) -> Vec<String> {
    let mut checked = files
        .iter()
        .map(|file| file.path.clone())
        .collect::<Vec<_>>();
    assert!(
        !checked.iter().any(|path| path == PROBE),
        "the emitter must not write {PROBE}, which the harness owns"
    );
    checked.push(PROBE.into());
    checked
}

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

#[derive(Clone, Debug)]
struct Compiler {
    program: String,
    arguments: Vec<String>,
}

fn compiler_candidates() -> Vec<Compiler> {
    let workspace = workspace_typescript().map(|path| Compiler {
        program: path.display().to_string(),
        arguments: Vec::new(),
    });
    [
        workspace,
        Some(Compiler {
            program: "tsc".into(),
            arguments: Vec::new(),
        }),
        Some(Compiler {
            program: "bunx".into(),
            arguments: ["--package", "typescript", "tsc"]
                .map(String::from)
                .to_vec(),
        }),
        Some(Compiler {
            program: "npx".into(),
            arguments: ["-p", "typescript", "tsc"].map(String::from).to_vec(),
        }),
    ]
    .into_iter()
    .flatten()
    .collect()
}

fn workspace_bin_directories() -> Vec<PathBuf> {
    let mut directory = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut directories = Vec::new();
    loop {
        directories.push(directory.join("node_modules").join(".bin"));
        if !directory.pop() {
            return directories;
        }
    }
}

fn workspace_typescript() -> Option<PathBuf> {
    workspace_bin_directories()
        .into_iter()
        .flat_map(|bin| [bin.join("tsc"), bin.join("tsc.cmd")])
        .find(|candidate| candidate.is_file())
}

fn is_compiler_version(stdout: &str) -> bool {
    stdout
        .lines()
        .any(|line| line.trim_start().starts_with("Version "))
}

fn tsc_command() -> Compiler {
    let candidates = compiler_candidates();
    for candidate in &candidates {
        let Ok(output) = Command::new(&candidate.program)
            .args(&candidate.arguments)
            .arg("--version")
            .output()
        else {
            continue;
        };
        if output.status.success() && is_compiler_version(&String::from_utf8_lossy(&output.stdout))
        {
            return candidate.clone();
        }
    }
    panic!(
        "no TypeScript compiler for the generated TypeScript type tests, tried {candidates:?} and \
         every tsc and tsc.cmd in {directories:?}; the deprecated npm package named tsc prints a \
         banner instead of compiling, so install the compiler itself with \
         `npm install --save-dev typescript` or `bun add --dev typescript`",
        directories = workspace_bin_directories(),
    );
}

fn type_checks(files: &[GeneratedFile], probe: &str) {
    let compiler = tsc_command();
    let checked = checked_files(files);
    let directory = tempfile::tempdir().unwrap();
    for file in files {
        fs::write(directory.path().join(&file.path), &file.content).unwrap();
    }
    fs::write(directory.path().join(PROBE), probe).unwrap();
    fs::write(directory.path().join(PROJECT), tsconfig(&checked)).unwrap();
    let mut written = fs::read_dir(directory.path())
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "ts"))
        .map(|path| path.file_name().unwrap().to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    written.sort();
    assert_eq!(
        written,
        {
            let mut expected = checked.clone();
            expected.sort();
            expected
        },
        "the type check must cover every emitted file, listed in {PROJECT}"
    );
    let output = Command::new(&compiler.program)
        .args(&compiler.arguments)
        .args(["--project", PROJECT])
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
fn every_candidate_invokes_the_typescript_compiler_and_nothing_else() {
    let runners = compiler_candidates()
        .iter()
        .map(|candidate| (candidate.program.clone(), candidate.arguments.clone()))
        .collect::<Vec<_>>();
    let arguments = |program: &str| {
        runners
            .iter()
            .find(|(name, _)| name == program)
            .map(|(_, arguments)| arguments.clone())
    };
    let expected = |arguments: [&str; 3]| arguments.map(String::from).to_vec();
    assert_eq!(
        arguments("bunx"),
        Some(expected(["--package", "typescript", "tsc"])),
        "bunx tsc resolves the deprecated npm package named tsc"
    );
    assert_eq!(
        arguments("npx"),
        Some(expected(["-p", "typescript", "tsc"])),
        "npx tsc resolves the deprecated npm package named tsc"
    );
    assert_eq!(
        arguments("tsc"),
        Some(Vec::new()),
        "`tsc tsc --project` mixes a source file with --project, which the compiler refuses as TS5042"
    );
    for (program, arguments) in &runners {
        if Path::new(program)
            .file_name()
            .is_some_and(|name| name == "tsc" || name == "tsc.cmd")
        {
            assert!(
                arguments.is_empty(),
                "{program} is the compiler itself, so `tsc tsc --project` would be TS5042: {arguments:?}"
            );
        }
    }
}

#[test]
fn a_program_that_only_prints_a_banner_is_not_a_compiler() {
    assert!(is_compiler_version("Version 5.9.2\n"));
    assert!(is_compiler_version("  Version 7.0.2\r\n"));
    assert!(!is_compiler_version(""));
    assert!(!is_compiler_version(
        "This is not the tsc command you are looking for\n\nTo get access to the \
         TypeScript compiler, tsc, from the command line either:\n\n- Use `npm install \
         typescript`\n"
    ));
}

#[test]
fn the_check_covers_every_file_the_emitter_produced() {
    let document: Document = serde_json::from_value(fixture()).unwrap();
    let files = files(&document);
    let checked = checked_files(&files);
    assert!(!files.is_empty());
    for file in &files {
        assert!(
            checked.contains(&file.path),
            "{} escapes the type check",
            file.path
        );
    }
    assert_eq!(checked.last().map(String::as_str), Some(PROBE));
    let config: Value = serde_json::from_str(&tsconfig(&checked)).unwrap();
    assert_eq!(
        config["files"],
        serde_json::to_value(&checked).unwrap(),
        "{PROJECT} must list the emitted files, not a pinned list"
    );

    let extra = GeneratedFile {
        path: "runtime.ts".into(),
        content: "export const runtime = 1;\n".into(),
    };
    let grown = checked_files(&[files, vec![extra]].concat());
    assert!(grown.contains(&"runtime.ts".to_string()), "{grown:?}");
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
    let files = emit_with_strategy(&document, EmitMode::Lossy, ErrorStrategy::Throw)
        .expect("the TypeScript emitter must accept the document")
        .files;
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
