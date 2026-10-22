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

const COMPILER_OPTIONS: &str = r#"{
    "target": "es2022",
    "module": "esnext",
    "moduleResolution": "bundler",
    "lib": ["es2022", "dom", "dom.iterable"],
    "types": [],
    "strict": true,
    "exactOptionalPropertyTypes": true,
    "noUncheckedIndexedAccess": true,
    "noUnusedLocals": true,
    "noUnusedParameters": true,
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

fn documented() -> Value {
    let mut raw = fixture();
    raw["types"]["User"]["doc"] = serde_json::json!({
        "summary": "A person",
        "description": "Ends the comment early */ and then\ncarries on"
    });
    raw["types"]["User"]["fields"][1]["doc"] =
        serde_json::json!({"summary": "How they would like to be called."});
    raw["types"]["LoginRequest"]["doc"] =
        serde_json::json!({"summary": "Credentials", "deprecated": true});
    raw["types"]["LoginError"]["variants"][0]["doc"] = serde_json::json!({
        "summary": "The credentials were refused.",
        "deprecated": true
    });
    raw["types"]["LoginError"]["variants"][1]["payload"]["fields"][0]["doc"] =
        serde_json::json!({"summary": "How long to wait."});
    raw["operations"][0]["doc"] = serde_json::json!({
        "summary": "Exchange a password for a session.",
        "description": "The session token arrives in the body."
    });
    raw["operations"][0]["errors"][0]["doc"] =
        serde_json::json!({"summary": "The credentials did not match."});
    raw["common_errors"] = serde_json::json!([{
        "status": "5xx",
        "name": ["server"],
        "content": "text",
        "body": {"kind": "string"},
        "doc": {"summary": "The server failed."}
    }]);
    raw
}

fn files(document: &Document) -> Vec<GeneratedFile> {
    emit_with_strategy(document, EmitMode::Strict, ErrorStrategy::Throw)
        .expect("the TypeScript emitter must accept the document")
        .files
}

fn files_with(document: &Document, options: pons_emit_ts::TsOptions) -> Vec<GeneratedFile> {
    pons_emit_ts::emit_with_options(
        document,
        pons_emit_ts::TsOptions {
            errors: ErrorStrategy::Throw,
            ..options
        },
    )
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

fn operations_of<'a>(files: &'a [GeneratedFile], member: &str) -> &'a str {
    &files
        .iter()
        .find(|file| file.path == format!("operations/{member}.ts"))
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
    type_checks_with(files, probe, false)
}

fn type_checks_with(files: &[GeneratedFile], probe: &str, packaged: bool) {
    type_checks_with_packages(files, probe, packaged, &[])
}

fn type_checks_beside(files: &[GeneratedFile], probe: &str, packaged: bool, zod: bool) {
    let packages = if zod { vec!["zod"] } else { Vec::new() };
    type_checks_with_packages(files, probe, packaged, &packages);
}

fn type_checks_with_packages(
    files: &[GeneratedFile],
    probe: &str,
    packaged: bool,
    packages: &[&str],
) {
    let compiler = tsc_command();
    let checked = checked_files(files);
    let directory = tempfile::tempdir().unwrap();
    for file in files {
        let destination = directory.path().join(&file.path);
        fs::create_dir_all(destination.parent().unwrap()).unwrap();
        fs::write(destination, &file.content).unwrap();
    }
    fs::write(directory.path().join(PROBE), probe).unwrap();
    fs::write(directory.path().join(PROJECT), tsconfig(&checked)).unwrap();
    let mut expected = checked.clone();
    if packaged {
        expected.extend(install_runtime_package(directory.path()));
    }
    for package in packages {
        install_package(directory.path(), package);
    }
    let mut written = written_sources(directory.path());
    written.sort();
    expected.sort();
    assert_eq!(
        written, expected,
        "the type check must cover every emitted file and the runtime beside them, listed in \
         {PROJECT}"
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

fn install_package(output: &Path, name: &str) {
    let mut directory = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let package = loop {
        let candidate = directory.join("node_modules").join(name);
        if candidate.join("package.json").is_file() {
            break candidate;
        }
        if !directory.pop() {
            panic!(
                "no {name} beside the workspace to type check the emitted schemas against, looked \
                 in {:?} and every directory above it; install it with `bun add --dev {name}` or \
                 `npm install --save-dev {name}`",
                env!("CARGO_MANIFEST_DIR")
            );
        }
    };
    let scope = output.join("node_modules");
    fs::create_dir_all(&scope).unwrap();
    let link = scope.join(name);
    #[cfg(unix)]
    std::os::unix::fs::symlink(&package, &link).unwrap();
    #[cfg(windows)]
    std::os::windows::fs::symlink_dir(&package, &link).unwrap();
}

fn install_runtime_package(output: &Path) -> Vec<String> {
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../runtime/ts");
    let installed = output.join("runtime");
    fs::create_dir_all(&installed).unwrap();
    let mut written = Vec::new();
    for entry in fs::read_dir(&source).unwrap() {
        let entry = entry.unwrap();
        let name = entry.file_name();
        if name.to_string_lossy().ends_with(".ts") {
            fs::copy(entry.path(), installed.join(&name)).unwrap();
            written.push(format!("runtime/{}", name.to_string_lossy()));
        }
    }
    fs::copy(source.join("package.json"), installed.join("package.json")).unwrap();
    let scope = output.join("node_modules").join("@pons");
    fs::create_dir_all(&scope).unwrap();
    let link = scope.join("runtime");
    #[cfg(unix)]
    std::os::unix::fs::symlink(&installed, &link).unwrap();
    #[cfg(windows)]
    std::os::windows::fs::symlink_dir(&installed, &link).unwrap();
    written
}

fn written_sources(root: &Path) -> Vec<String> {
    let mut written = Vec::new();
    let mut directories = vec![root.to_path_buf()];
    while let Some(directory) = directories.pop() {
        for entry in fs::read_dir(&directory).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                if path.file_name().is_some_and(|name| name == "node_modules") {
                    continue;
                }
                directories.push(path);
            } else if path.extension().is_some_and(|extension| extension == "ts") {
                written.push(
                    path.strip_prefix(root)
                        .unwrap()
                        .to_string_lossy()
                        .replace('\\', "/"),
                );
            }
        }
    }
    written
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
        path: "runtime/extra.ts".into(),
        content: "export const extra = 1;\n".into(),
    };
    let grown = checked_files(&[files, vec![extra]].concat());
    assert!(grown.contains(&"runtime/extra.ts".to_string()), "{grown:?}");
}

#[test]
fn the_check_covers_the_vendored_runtime_sources() {
    let document: Document = serde_json::from_value(fixture()).unwrap();
    let files = files(&document);
    let checked = checked_files(&files);
    for module in [
        "runtime.ts",
        "runtime/index.ts",
        "runtime/wire.ts",
        "runtime/codec.ts",
        "runtime/validate.ts",
        "runtime/stream.ts",
        "runtime/credentials.ts",
        "runtime/transport.ts",
        "runtime/websocket.ts",
    ] {
        assert!(
            checked.contains(&module.to_string()),
            "{module} escapes the type check: {checked:?}"
        );
    }
    let config: Value = serde_json::from_str(&tsconfig(&checked)).unwrap();
    assert_eq!(config["files"], serde_json::to_value(&checked).unwrap());
}

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
fn a_client_that_checks_its_responses_against_zod_compiles_beside_zod() {
    let document: Document = serde_json::from_value(fixture()).unwrap();
    for packaged in [false, true] {
        let files = files_with(
            &document,
            pons_emit_ts::TsOptions {
                validation: pons_emit_ts::ValidationOptions::new(
                    pons_emit_ts::ValidationBackend::Zod,
                ),
                runtime: if packaged {
                    pons_emit_ts::Runtime::Package
                } else {
                    pons_emit_ts::Runtime::Vendor
                },
                ..pons_emit_ts::TsOptions::default()
            },
        );
        assert!(
            files.iter().any(|file| file.path == "validators.ts"),
            "the option writes the validators: {:?}",
            files.iter().map(|file| &file.path).collect::<Vec<_>>()
        );
        type_checks_beside(
            &files,
            r#"
import { createClient } from "./client.js";
import { validateLoginErrorInvalid, validateUser } from "./validators.js";

export const client = createClient({ baseUrl: "https://api.example.test" });
export const loggedIn = await client.auth.login({ body: { email: "person@example.test" } });
export const id = loggedIn.id;
export const user = validateUser({ id: "1" });
export const invalid = validateLoginErrorInvalid({ code: "invalid" });
// @ts-expect-error a validator takes one argument and returns a verdict
export const wrong = validateUser({ id: "1" }, { id: "1" });
"#,
            packaged,
            true,
        );
    }
}

#[test]
fn a_client_that_checks_its_responses_against_valibot_compiles_beside_valibot() {
    let document: Document = serde_json::from_value(fixture()).unwrap();
    for packaged in [false, true] {
        let files = files_with(
            &document,
            pons_emit_ts::TsOptions {
                validation: pons_emit_ts::ValidationOptions::new(
                    pons_emit_ts::ValidationBackend::Valibot,
                ),
                runtime: if packaged {
                    pons_emit_ts::Runtime::Package
                } else {
                    pons_emit_ts::Runtime::Vendor
                },
                ..pons_emit_ts::TsOptions::default()
            },
        );
        type_checks_with_packages(
            &files,
            r#"
import { validateLoginErrorInvalid, validateUser } from "./validators.js";

export const user = validateUser({ id: "1" });
export const invalid = validateLoginErrorInvalid({ code: "invalid" });
// @ts-expect-error a validator takes one argument and returns a verdict
export const wrong = validateUser({ id: "1" }, { id: "1" });
"#,
            packaged,
            &["valibot"],
        );
    }
}

#[test]
fn a_client_that_checks_its_responses_against_superstruct_compiles_beside_superstruct() {
    let document: Document = serde_json::from_value(fixture()).unwrap();
    for packaged in [false, true] {
        let files = files_with(
            &document,
            pons_emit_ts::TsOptions {
                validation: pons_emit_ts::ValidationOptions::new(
                    pons_emit_ts::ValidationBackend::Superstruct,
                ),
                runtime: if packaged {
                    pons_emit_ts::Runtime::Package
                } else {
                    pons_emit_ts::Runtime::Vendor
                },
                ..pons_emit_ts::TsOptions::default()
            },
        );
        type_checks_with_packages(
            &files,
            r#"
import { validateLoginErrorInvalid, validateUser } from "./validators.js";

export const user = validateUser({ id: "1" });
export const invalid = validateLoginErrorInvalid({ code: "invalid" });
// @ts-expect-error a validator takes one argument and returns a verdict
export const wrong = validateUser({ id: "1" }, { id: "1" });
"#,
            packaged,
            &["superstruct"],
        );
    }
}

#[test]
fn a_client_that_checks_its_responses_against_typebox_compiles_beside_typebox() {
    let document: Document = serde_json::from_value(fixture()).unwrap();
    for packaged in [false, true] {
        let files = files_with(
            &document,
            pons_emit_ts::TsOptions {
                validation: pons_emit_ts::ValidationOptions::new(
                    pons_emit_ts::ValidationBackend::Typebox,
                ),
                runtime: if packaged {
                    pons_emit_ts::Runtime::Package
                } else {
                    pons_emit_ts::Runtime::Vendor
                },
                ..pons_emit_ts::TsOptions::default()
            },
        );
        type_checks_with_packages(
            &files,
            r#"
import { validateLoginErrorInvalid, validateUser } from "./validators.js";

export const user = validateUser({ id: "1" });
export const invalid = validateLoginErrorInvalid({ code: "invalid" });
// @ts-expect-error a validator takes one argument and returns a verdict
export const wrong = validateUser({ id: "1" }, { id: "1" });
"#,
            packaged,
            &["typebox"],
        );
    }
}

#[test]
fn a_client_that_checks_its_responses_against_yup_compiles_beside_yup() {
    let document: Document = serde_json::from_value(fixture()).unwrap();
    for packaged in [false, true] {
        let files = files_with(
            &document,
            pons_emit_ts::TsOptions {
                validation: pons_emit_ts::ValidationOptions::new(
                    pons_emit_ts::ValidationBackend::Yup,
                ),
                mode: pons_emit_ts::EmitMode::Lossy,
                runtime: if packaged {
                    pons_emit_ts::Runtime::Package
                } else {
                    pons_emit_ts::Runtime::Vendor
                },
                ..pons_emit_ts::TsOptions::default()
            },
        );
        type_checks_with_packages(
            &files,
            r#"
import { validateLoginErrorInvalid, validateUser } from "./validators.js";

export const user = validateUser({ id: "1" });
export const invalid = validateLoginErrorInvalid({ code: "invalid" });
// @ts-expect-error a validator takes one argument and returns a verdict
export const wrong = validateUser({ id: "1" }, { id: "1" });
"#,
            packaged,
            &["yup"],
        );
    }
}

#[test]
fn a_validator_over_every_format_compiles_beside_zod() {
    let mut raw = fixture();
    raw["types"]["User"] = serde_json::json!({
        "kind": "struct", "name": ["user"], "direction": "output", "openness": "reject",
        "fields": [
            {"wire": "moment", "ident": ["moment"], "ty": {"kind": "string", "format": "date-time"}, "presence": "required"},
            {"wire": "day", "ident": ["day"], "ty": {"kind": "string", "format": "date"}, "presence": "required"},
            {"wire": "clock", "ident": ["clock"], "ty": {"kind": "string", "format": "time"}, "presence": "required"},
            {"wire": "span", "ident": ["span"], "ty": {"kind": "string", "format": "duration"}, "presence": "required"},
            {"wire": "uuid", "ident": ["uuid"], "ty": {"kind": "string", "format": "uuid"}, "presence": "required"},
            {"wire": "link", "ident": ["link"], "ty": {"kind": "string", "format": "uri"}, "presence": "required"},
            {"wire": "who", "ident": ["who"], "ty": {"kind": "string", "format": "email"}, "presence": "required"},
            {"wire": "v4", "ident": ["v4"], "ty": {"kind": "string", "format": "ipv4"}, "presence": "required"},
            {"wire": "v6", "ident": ["v6"], "ty": {"kind": "string", "format": "ipv6"}, "presence": "required"}
        ]
    });
    raw["operations"][0]["provides"] = serde_json::Value::Null;
    let document: Document = serde_json::from_value(raw).unwrap();
    let files = files_with(
        &document,
        pons_emit_ts::TsOptions {
            validation: pons_emit_ts::ValidationOptions::new(pons_emit_ts::ValidationBackend::Zod),
            ..pons_emit_ts::TsOptions::default()
        },
    );
    type_checks_beside(
        &files,
        r#"
import { validateUser } from "./validators.js";

export const issue = validateUser({
  moment: "2024-01-01T00:00:00Z",
  day: "2024-01-01",
  clock: "00:00:00Z",
  span: "P1D",
  uuid: "6ba7b810-9dad-11d1-80b4-00c04fd430c8",
  link: "https://example.test",
  who: "a@b.co",
  v4: "1.2.3.4",
  v6: "::1",
});
"#,
        false,
        true,
    );
}

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
        operations_of(&files, "auth")
            .contains("{ kind: \"http\"; status: number; body: string; headers: Headers }"),
        "{}",
        operations_of(&files, "auth")
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

#[test]
fn a_documented_contract_compiles_under_the_strictest_flags() {
    let document: Document = serde_json::from_value(documented()).unwrap();
    let files = files(&document);
    for source in [
        types_of(&files),
        client_of(&files),
        operations_of(&files, "auth"),
        operations_of(&files, "events"),
    ] {
        assert!(
            source.contains("/**"),
            "the document is documented:\n{source}"
        );
        assert_eq!(
            source.matches("/**").count(),
            source.matches("*/").count(),
            "a summary that ends a comment early leaves every comment closed:\n{source}"
        );
        assert!(
            !source.contains("\n */"),
            "a comment ends on the line it starts:\n{source}"
        );
    }
    assert!(
        types_of(&files)
            .contains("/** A person Ends the comment early *\\/ and then carries on */"),
        "{}",
        types_of(&files)
    );
    type_checks(
        &files,
        r#"
import { createClient, type AuthLoginError } from "./client.js";

const client = createClient({ baseUrl: "https://api.example.test" });
const loggedIn = await client.auth.login({ body: { email: "person@example.test" } });
const failure: AuthLoginError | undefined = undefined;
console.log(loggedIn, failure);
"#,
    );
}

fn tagged() -> Document {
    let mut raw = fixture();
    raw["types"]["Tag"] = serde_json::json!({
        "kind": "newtype", "name": ["tag"], "direction": "both", "inner": {"kind": "string"}
    });
    raw["types"]["Session"] = serde_json::json!({
        "kind": "struct", "name": ["session"], "direction": "both", "openness": "reject",
        "fields": [
            {"wire": "tag", "ident": ["tag"], "ty": {"ref": "Tag"}, "presence": "required"}
        ]
    });
    raw["types"]["LoginRequest"] = serde_json::json!({
        "kind": "struct", "name": ["login", "request"], "direction": "input", "openness": "reject",
        "fields": [
            {"wire": "tag", "ident": ["tag"], "ty": {"ref": "Tag"}, "presence": "required"}
        ]
    });
    raw["operations"][0]["responses"][0]["body"] = serde_json::json!({"ref": "Session"});
    raw["operations"][0]["provides"] = serde_json::Value::Null;
    serde_json::from_value(raw).unwrap()
}

#[test]
fn a_branded_newtype_is_nominal_where_an_alias_is_not() {
    let document = tagged();
    let branded_files = files_with(
        &document,
        pons_emit_ts::TsOptions {
            newtypes: pons_emit_ts::Newtypes::Branded,
            ..pons_emit_ts::TsOptions::default()
        },
    );
    assert!(
        types_of(&branded_files)
            .contains("export type Tag = string & { readonly [brandTag]: never };"),
        "{}",
        types_of(&branded_files)
    );
    type_checks(
        &branded_files,
        r#"
import { createClient } from "./client.js";
import type { Tag } from "./types.js";

const client = createClient({ baseUrl: "https://api.example.test" });
const session = await client.auth.login({ body: { tag: "abc" as Tag } });
export const branded: Tag = session.tag;
const plain: string = session.tag;
// @ts-expect-error a plain string is not a branded newtype
export const forged: Tag = "abc";
// @ts-expect-error the brand does not leak onto the alias either
export const swapped: string & { readonly [brandTag]: never } = branded;
console.log(branded, plain, forged, swapped);
"#,
    );
    let aliased = files(&document);
    type_checks(
        &aliased,
        r#"
import { createClient } from "./client.js";
import type { Tag } from "./types.js";

const client = createClient({ baseUrl: "https://api.example.test" });
const session = await client.auth.login({ body: { tag: "abc" as Tag } });
export const plain: string = session.tag;
export const alias: Tag = session.tag;
console.log(plain, alias);
"#,
    );
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

fn camel_document() -> Document {
    let fields = serde_json::json!([
        {"wire": "user_id", "ident": ["user", "id"], "ty": {"kind": "string"}, "presence": "required"},
        {"wire": "retry_after", "ident": ["retry", "after"], "ty": {"kind": "int", "bits": 32, "signed": false, "wire": "number"}, "presence": "required"}
    ]);
    let mut raw = fixture();
    raw["types"]["User"] = serde_json::json!({
        "kind": "struct", "name": ["user"], "direction": "output", "openness": "reject", "fields": fields
    });
    raw["types"]["LoginRequest"] = serde_json::json!({
        "kind": "struct", "name": ["login", "request"], "direction": "input", "openness": "reject", "fields": fields
    });
    raw["operations"][0]["provides"] = serde_json::Value::Null;
    serde_json::from_value(raw).unwrap()
}

#[test]
fn a_camel_member_is_the_only_name_a_caller_may_write() {
    let document = camel_document();
    let camel = files_with(
        &document,
        pons_emit_ts::TsOptions {
            field_case: pons_emit_ts::FieldCase::Camel,
            ..pons_emit_ts::TsOptions::default()
        },
    );
    type_checks(
        &camel,
        r#"
import { createClient } from "./client.js";
import type { User } from "./types.js";

const client = createClient({ baseUrl: "https://api.example.test" });
const value = await client.auth.login({ body: { userId: "u-1", retryAfter: 30 } });
const id: string = value.userId;
const after: number = value.retryAfter;
const user: User = { userId: "u-1", retryAfter: 30 };
// @ts-expect-error the wire name is not a member of the emitted type
const wire: User = { user_id: "u-1", retry_after: 30 };
// @ts-expect-error the caller cannot read a member the wire spells
const leaked: string = value.user_id;
console.log(id, after, user, wire, leaked);
"#,
    );
    let aliased = files(&document);
    type_checks(
        &aliased,
        r#"
import { createClient } from "./client.js";

const client = createClient({ baseUrl: "https://api.example.test" });
const value = await client.auth.login({ body: { user_id: "u-1", retry_after: 30 } });
// @ts-expect-error the wire case has no camel member
const camel: string = value.userId;
console.log(value.user_id, camel);
"#,
    );
}

fn wide_document() -> Document {
    let mut raw = fixture();
    raw["types"]["User"] = serde_json::json!({
        "kind": "struct", "name": ["user"], "direction": "output", "openness": "reject",
        "fields": [
            {"wire": "count", "ident": ["count"], "ty": {"kind": "int", "bits": 64, "signed": false, "wire": "number"}, "presence": "required"}
        ]
    });
    raw["types"]["LoginRequest"] = serde_json::json!({
        "kind": "struct", "name": ["login", "request"], "direction": "input", "openness": "reject",
        "fields": [
            {"wire": "count", "ident": ["count"], "ty": {"kind": "int", "bits": 64, "signed": false, "wire": "number"}, "presence": "required"}
        ]
    });
    raw["operations"][0]["provides"] = serde_json::Value::Null;
    serde_json::from_value(raw).unwrap()
}

#[test]
fn a_wide_integer_is_a_string_where_the_option_says_so() {
    let document = wide_document();
    let files = files_with(
        &document,
        pons_emit_ts::TsOptions {
            int64: pons_emit_ts::Int64::String,
            ..pons_emit_ts::TsOptions::default()
        },
    );
    type_checks(
        &files,
        r#"
import { createClient } from "./client.js";

const client = createClient({ baseUrl: "https://api.example.test" });
const value = await client.auth.login({ body: { count: "9007199254740993" } });
const exact: string = value.count;
const digits: bigint = BigInt(value.count);
// @ts-expect-error the option says the member is the string, not a bigint
const wrong: bigint = value.count;
// @ts-expect-error a number is not the string the option declares
await client.auth.login({ body: { count: 9007199254740993n } });
console.log(exact, digits, wrong);
"#,
    );
}

fn date_document() -> Document {
    let fields = serde_json::json!([
        {"wire": "day", "ident": ["day"], "ty": {"kind": "string", "format": "date"}, "presence": "required"},
        {"wire": "at", "ident": ["at"], "ty": {"kind": "string", "format": "date-time"}, "presence": "required"}
    ]);
    let mut raw = fixture();
    raw["types"]["User"] = serde_json::json!({
        "kind": "struct", "name": ["user"], "direction": "output", "openness": "reject", "fields": fields
    });
    raw["types"]["LoginRequest"] = serde_json::json!({
        "kind": "struct", "name": ["login", "request"], "direction": "input", "openness": "reject", "fields": fields
    });
    raw["operations"][0]["provides"] = serde_json::Value::Null;
    serde_json::from_value(raw).unwrap()
}

#[test]
fn a_day_is_a_branded_string_and_an_instant_is_a_date() {
    let document = date_document();
    let carried = files_with(
        &document,
        pons_emit_ts::TsOptions {
            date_mode: pons_emit_ts::DateMode::Date,
            ..pons_emit_ts::TsOptions::default()
        },
    );
    type_checks(
        &carried,
        r#"
import { createClient } from "./client.js";
import type { DateOnly } from "./types.js";

const client = createClient({ baseUrl: "https://api.example.test" });
const value = await client.auth.login({
  body: { day: "2024-01-31" as DateOnly, at: new Date("2024-01-31T12:00:00Z") },
});
const day: DateOnly = value.day;
const text: string = value.day;
const instant: Date = value.at;
const millis: number = value.at.getTime();
// @ts-expect-error a plain string is not a calendar day
const forged: DateOnly = "2024-01-31";
// @ts-expect-error a day is not an instant
const wrongInstant: Date = value.day;
// @ts-expect-error an instant is not a calendar day
const wrongDay: DateOnly = new Date();
// @ts-expect-error a date-time member is a Date, not a string
await client.auth.login({ body: { day: "2024-01-31" as DateOnly, at: "2024-01-31T12:00:00Z" } });
console.log(day, text, instant, millis, forged, wrongInstant, wrongDay);
"#,
    );
    let wire = files(&document);
    type_checks(
        &wire,
        r#"
import { createClient } from "./client.js";

const client = createClient({ baseUrl: "https://api.example.test" });
const value = await client.auth.login({ body: { day: "2024-01-31", at: "2024-01-31T12:00:00Z" } });
// @ts-expect-error the default options carry the wire string, not a Date
const instant: Date = value.at;
console.log(value.day, instant);
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
