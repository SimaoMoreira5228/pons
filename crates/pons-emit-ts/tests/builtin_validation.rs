use std::{fs, path::Path, process::Command};

use pons_emit_ts::{
    DateMode, ErrorStrategy, GeneratedFile, Int64, Runtime, TsOptions, Validation,
    emit_with_options,
};
use pons_ir::Document;
use serde_json::{Value, json};

fn document() -> Document {
    serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap()
}

fn files_with(document: &Document, options: TsOptions) -> Vec<GeneratedFile> {
    emit_with_options(document, options)
        .expect("the TypeScript emitter must accept the document")
        .files
}

fn files(document: &Document) -> Vec<GeneratedFile> {
    files_with(
        document,
        TsOptions {
            runtime: Runtime::Package,
            validation: Validation::Builtin,
            errors: ErrorStrategy::Throw,
            ..TsOptions::default()
        },
    )
}

fn install_runtime_package(directory: &Path) {
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../runtime/ts");
    let installed = directory.join("runtime");
    fs::create_dir_all(&installed).unwrap();
    for entry in fs::read_dir(source).unwrap() {
        let entry = entry.unwrap();
        let name = entry.file_name();
        if name.to_string_lossy().ends_with(".ts") || name == "package.json" {
            fs::copy(entry.path(), installed.join(&name)).unwrap();
        }
    }
    let scope = directory.join("node_modules").join("@pons");
    fs::create_dir_all(&scope).unwrap();
    link(&installed, &scope.join("runtime"));
}

fn link(source: &Path, destination: &Path) {
    #[cfg(unix)]
    std::os::unix::fs::symlink(source, destination).unwrap();
    #[cfg(windows)]
    std::os::windows::fs::symlink_dir(source, destination).unwrap();
}

fn run_bun(files: &[GeneratedFile], harness: &str) {
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
    install_runtime_package(directory.path());
    let harness_path = directory.path().join("check.ts");
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

fn validators_of(files: &[GeneratedFile]) -> &str {
    &files
        .iter()
        .find(|file| file.path == "validators.ts")
        .expect("the builtin backend writes validators.ts")
        .content
}

#[test]
fn a_builtin_contract_checks_a_decoded_value_against_the_runtime_walker() {
    let document = document();
    let files = files(&document);
    let validators = validators_of(&files);
    for expected in [
        "export const validateUser: Validator = (value) => validateContract(value, {\"ref\":\"User\"});",
        "export const validateLoginErrorInvalid: Validator = (value) => validateContract(value, {\"ref\":\"LoginError\",\"variant\":\"invalid\"});",
    ] {
        assert!(
            validators.contains(expected),
            "{expected} is missing: {validators}"
        );
    }
    assert!(
        !validators.contains("validateLoginRequest"),
        "an input-only type needs no response validator: {validators}"
    );
    assert!(
        files.iter().all(|file| file.path != "schemas.ts"),
        "the builtin backend writes no library schema"
    );
    assert!(
        files.iter().any(|file| file.path == "validators.ts"),
        "the builtin backend writes validators"
    );

    let harness = r#"
import { validateLoginErrorInvalid, validateUser } from "./validators.ts";

const admits = (label: string, check: (value: unknown) => unknown, value: unknown) => {
  const issue = check(value);
  if (issue !== null) throw new Error(`${label} was refused: ${String(issue)}`);
};
const refuses = (label: string, check: (value: unknown) => unknown, value: unknown) => {
  if (check(value) === null) throw new Error(`${label} was admitted`);
};

admits("a user", validateUser, { id: "1", nickname: null });
admits("a user without a nickname", validateUser, { id: "1" });
refuses("a user whose id is not canonical text", validateUser, { id: 1 });
refuses("a user without an id", validateUser, {});

admits("an invalid-credentials error", validateLoginErrorInvalid, { code: "invalid" });
refuses("a locked error under the invalid variant", validateLoginErrorInvalid, {
  code: "locked",
  retry_after: 1,
});
"#;
    run_bun(&files, harness);
}

#[test]
fn a_builtin_contract_checks_the_decoded_carriers() {
    let mut raw: Value =
        serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
    raw["types"]["User"]["fields"][0]["ty"] =
        json!({"kind": "int", "bits": 64, "signed": false, "wire": "number"});
    raw["types"]["User"]["fields"]
        .as_array_mut()
        .unwrap()
        .push(json!({
            "wire": "created_at",
            "ident": ["created", "at"],
            "ty": {"kind": "string", "format": "date-time"},
            "presence": "required"
        }));
    let document: Document = serde_json::from_value(raw).unwrap();
    let files = files_with(
        &document,
        TsOptions {
            runtime: Runtime::Package,
            validation: Validation::Builtin,
            errors: ErrorStrategy::Throw,
            int64: Int64::Bigint,
            date_mode: DateMode::Date,
            ..TsOptions::default()
        },
    );
    let harness = r#"
import { validateLoginErrorLocked, validateUser } from "./validators.ts";

const admits = (label: string, check: (value: unknown) => unknown, value: unknown) => {
  const issue = check(value);
  if (issue !== null) throw new Error(`${label} was refused: ${String(issue)}`);
};
const refuses = (label: string, check: (value: unknown) => unknown, value: unknown) => {
  if (check(value) === null) throw new Error(`${label} was admitted`);
};

const created = new Date("2024-01-02T03:04:05.678Z");
admits("a decoded wide integer and date", validateUser, { id: 1n, created_at: created });
refuses("a wide integer as text", validateUser, { id: "1", created_at: created });
refuses("a wide integer as a plain number", validateUser, { id: 1, created_at: created });
refuses("a date-time string under the date carrier", validateUser, {
  id: 1n,
  created_at: "2024-01-02T03:04:05.678Z",
});
admits("a small integer as a plain number", validateLoginErrorLocked, {
  code: "locked",
  retry_after: 1,
});
refuses("a small integer as a bigint", validateLoginErrorLocked, {
  code: "locked",
  retry_after: 1n,
});
"#;
    run_bun(&files, harness);
}

#[test]
fn a_flattened_opaque_carrier_owns_the_value_it_decodes() {
    let mut raw: Value =
        serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
    raw["types"]["Carried"] = json!({
        "kind": "struct",
        "name": ["carried"],
        "direction": "output",
        "openness": "reject",
        "fields": [
            {
                "wire": "own",
                "ident": ["own"],
                "ty": {"kind": "string"},
                "presence": "required"
            }
        ],
        "flattened": [{"kind": "opaque", "codec": "flatten.codec", "wire": "json"}]
    });
    let document: Document = serde_json::from_value(raw).unwrap();
    let files = files(&document);
    let harness = r#"
import { validateCarried } from "./validators.ts";

const admits = (label: string, check: (value: unknown) => unknown, value: unknown) => {
  const issue = check(value);
  if (issue !== null) throw new Error(`${label} was refused: ${String(issue)}`);
};
const refuses = (label: string, check: (value: unknown) => unknown, value: unknown) => {
  if (check(value) === null) throw new Error(`${label} was admitted`);
};

class Carrier {
  own = "carried";
}

admits("an opaque carrier string", validateCarried, "carried");
admits("an opaque carrier instance", validateCarried, new Carrier());
admits("a plain record with the declared field", validateCarried, { own: "carried" });
admits("a plain record with a key the carrier owns", validateCarried, { own: "carried", extra: 1 });
refuses("a plain record without the declared field", validateCarried, { extra: 1 });
refuses("a plain record with a field of the wrong type", validateCarried, { own: 1 });
"#;
    run_bun(&files, harness);
}

#[test]
fn a_namespace_keeps_every_response_validator_in_one_deterministic_import() {
    let mut raw: Value =
        serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
    let mut refresh = raw["operations"][0].clone();
    let refresh = refresh.as_object_mut().unwrap();
    refresh.remove("body");
    refresh.remove("provides");
    refresh.insert("id".into(), json!([["auth"], ["refresh"]]));
    refresh.insert("method".into(), json!("GET"));
    refresh.insert("path".into(), json!([{"lit": "auth"}, {"lit": "refresh"}]));
    refresh.insert(
        "responses".into(),
        json!([{"status": 200, "content": "json", "body": {"ref": "LoginError"}}]),
    );
    refresh.remove("errors");
    raw["operations"]
        .as_array_mut()
        .unwrap()
        .push(Value::Object(refresh.clone()));
    let document: Document = serde_json::from_value(raw).unwrap();
    let emitted = files(&document);
    let auth = emitted
        .iter()
        .find(|file| file.path == "operations/auth.ts")
        .expect("both operations share the auth namespace");
    let expected = "import { validateLoginError, validateLoginErrorInvalid, validateUser } from \"../validators.js\";";
    assert_eq!(
        auth.content.matches(expected).count(),
        1,
        "{}",
        auth.content
    );
    assert!(
        auth.content.contains("validate: validateUser"),
        "{}",
        auth.content
    );
    assert!(
        auth.content.contains("validate: validateLoginError"),
        "{}",
        auth.content
    );
    assert_eq!(
        emitted,
        files(&document),
        "the import order must be deterministic"
    );
}

#[test]
fn a_bytes_response_descriptor_carries_a_validator() {
    let mut raw: Value =
        serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
    raw["operations"][0]["responses"][0]["content"] = json!("bytes");
    raw["operations"][0]["responses"][0]["body"] = json!({"kind": "bytes", "encoding": "base64"});
    let document: Document = serde_json::from_value(raw).unwrap();
    let files = files(&document);
    let validators = validators_of(&files);
    assert!(
        validators.contains(
            "export const validateInline1: Validator = (value) => validateContract(value, {\"encoding\":\"base64\",\"kind\":\"bytes\"});"
        ),
        "{validators}"
    );
    let auth = files
        .iter()
        .find(|file| file.path == "operations/auth.ts")
        .expect("the login operation is emitted");
    assert!(
        auth.content.contains("content: \"bytes\", schema: {\"encoding\":\"base64\",\"kind\":\"bytes\"}, validate: validateInline1"),
        "{}",
        auth.content
    );
}
