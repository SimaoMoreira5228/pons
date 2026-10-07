use std::{fs, path::Path, process::Command};

use pons_emit_ts::{
    ErrorStrategy, GeneratedFile, Runtime, TsOptions, Validation, emit_with_options,
};
use pons_ir::Document;

fn document() -> Document {
    serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap()
}

fn files(document: &Document) -> Vec<GeneratedFile> {
    emit_with_options(
        document,
        TsOptions {
            runtime: Runtime::Package,
            validation: Validation::Builtin,
            errors: ErrorStrategy::Throw,
            ..TsOptions::default()
        },
    )
    .expect("the TypeScript emitter must accept the document")
    .files
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
        "export const validateLoginRequest: Validator = (value) => validateContract(value, {\"ref\":\"LoginRequest\"});",
        "export const validateLoginErrorInvalid: Validator = (value) => validateContract(value, {\"ref\":\"LoginError\",\"variant\":\"invalid\"});",
    ] {
        assert!(
            validators.contains(expected),
            "{expected} is missing: {validators}"
        );
    }
    assert!(
        files.iter().all(|file| file.path != "schemas.ts"),
        "the builtin backend writes no library schema"
    );

    let harness = r#"
import { validateLoginErrorInvalid, validateLoginRequest, validateUser } from "./validators.ts";

const admits = (label: string, check: (value: unknown) => unknown, value: unknown) => {
  const issue = check(value);
  if (issue !== undefined) throw new Error(`${label} was refused: ${String(issue)}`);
};
const refuses = (label: string, check: (value: unknown) => unknown, value: unknown) => {
  if (check(value) === undefined) throw new Error(`${label} was admitted`);
};

admits("a user", validateUser, { id: "1", nickname: null });
admits("a user without a nickname", validateUser, { id: "1" });
refuses("a user whose id is not canonical text", validateUser, { id: 1 });
refuses("a user without an id", validateUser, {});

admits("a login request", validateLoginRequest, { email: "person@example.test" });
refuses("an email that is not one", validateLoginRequest, { email: "person" });

admits("an invalid-credentials error", validateLoginErrorInvalid, { code: "invalid" });
refuses("a locked error under the invalid variant", validateLoginErrorInvalid, {
  code: "locked",
  retry_after: 1,
});
"#;
    run_bun(&files, harness);
}
