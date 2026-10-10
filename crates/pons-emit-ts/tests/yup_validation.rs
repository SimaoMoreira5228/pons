use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use pons_emit_ts::{
    EmitMode, GeneratedFile, TsOptions, ValidationBackend, ValidationOptions, emit_with_options,
};
use pons_ir::Document;
use serde_json::{Value, json};

fn fixture() -> Value {
    serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap()
}

fn file<'a>(files: &'a [GeneratedFile], path: &str) -> &'a str {
    &files
        .iter()
        .find(|file| file.path == path)
        .unwrap_or_else(|| panic!("the emitter wrote no {path}"))
        .content
}

fn workspace_yup() -> PathBuf {
    let mut directory = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    loop {
        let candidate = directory.join("node_modules").join("yup");
        if candidate.join("package.json").is_file() {
            return candidate;
        }
        if !directory.pop() {
            panic!(
                "no yup beside the workspace to install into a generated client, looked in {:?} and \
                 every directory above it; install it with `bun add --dev yup`",
                env!("CARGO_MANIFEST_DIR")
            );
        }
    }
}

fn link(source: &Path, destination: &Path) {
    #[cfg(unix)]
    std::os::unix::fs::symlink(source, destination).unwrap();
    #[cfg(windows)]
    std::os::windows::fs::symlink_dir(source, destination).unwrap();
}

fn install_yup(directory: &Path) {
    let scope = directory.join("node_modules");
    fs::create_dir_all(&scope).unwrap();
    link(&workspace_yup(), &scope.join("yup"));
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
    install_yup(directory.path());
    let harness_path = directory.path().join("schema.ts");
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

fn wallet() -> Document {
    let mut raw = fixture();
    raw["types"] = json!({
        "Wallet": {
            "kind": "struct",
            "name": ["wallet"],
            "direction": "output",
            "openness": "reject",
            "fields": [
                {
                    "wire": "balance",
                    "ident": ["balance"],
                    "ty": {"kind": "int", "bits": 32, "signed": false, "wire": "number"},
                    "presence": "required"
                },
                {
                    "wire": "label",
                    "ident": ["label"],
                    "ty": {"kind": "string", "min_length": 2},
                    "presence": "optional"
                }
            ]
        }
    });
    raw["operations"] = json!([{
        "id": [["wallet"], ["read"]],
        "method": "GET",
        "path": [{"lit": "wallet"}],
        "auth": {"alternatives": [], "optional": false},
        "semantics": {"safe": true, "idempotent": true},
        "responses": [{"status": 200, "content": "json", "body": {"ref": "Wallet"}}],
        "errors": []
    }]);
    serde_json::from_value(raw).unwrap()
}

fn counts() -> Document {
    let mut raw = fixture();
    raw["types"] = json!({
        "Counts": {
            "kind": "struct",
            "name": ["counts"],
            "direction": "output",
            "openness": "reject",
            "fields": [{
                "wire": "values",
                "ident": ["values"],
                "ty": {
                    "kind": "map",
                    "key": {"kind": "string"},
                    "value": {"kind": "int", "bits": 32, "signed": false, "wire": "number"}
                },
                "presence": "required"
            }]
        }
    });
    raw["operations"] = json!([{
        "id": [["counts"], ["read"]],
        "method": "GET",
        "path": [{"lit": "counts"}],
        "auth": {"alternatives": [], "optional": false},
        "semantics": {"safe": true, "idempotent": true},
        "responses": [{"status": 200, "content": "json", "body": {"ref": "Counts"}}],
        "errors": []
    }]);
    serde_json::from_value(raw).unwrap()
}

#[test]
fn a_lossy_internal_enum_is_reported_while_the_bodies_it_can_check_stay_checked() {
    let document: Document = serde_json::from_value(fixture()).unwrap();
    let output = emit_with_options(
        &document,
        TsOptions {
            mode: EmitMode::Lossy,
            validation: ValidationOptions::new(ValidationBackend::Yup),
            ..TsOptions::default()
        },
    )
    .unwrap();
    assert!(
        !output.files.iter().any(|file| file.path == "schemas.ts"),
        "the public surface is validators.ts, not exported schemas"
    );
    let validators = file(&output.files, "validators.ts");
    for expected in [
        "const UserSchema = yup.object(",
        "export const validateUser: Validator = mismatch(UserSchema);",
        "export const validateLoginError: Validator = mismatch(LoginErrorSchema);",
        "export const validateLoginErrorInvalid: Validator = mismatch(LoginErrorInvalidSchema);",
        "export const validateLoginErrorLocked: Validator = mismatch(LoginErrorLockedSchema);",
    ] {
        assert!(
            validators.contains(expected),
            "{expected} is missing:\n{validators}"
        );
    }
    assert!(
        output
            .validation_coverage
            .lossy
            .contains("types.LoginError"),
        "the internal enum lost its discrimination: {:?}",
        output.validation_coverage
    );
    assert!(
        output.validation_coverage.exact.contains("types.User"),
        "the user struct is exact: {:?}",
        output.validation_coverage
    );
    let harness = r#"
import {
  validateLoginError, validateLoginErrorInvalid, validateLoginErrorLocked, validateUser,
} from "./validators.ts";

const admits = (label: string, check: (value: unknown) => unknown, value: unknown) => {
  const found = check(value);
  if (found !== null) throw new Error(`${label} was refused: ${JSON.stringify(found)}`);
};
const refuses = (label: string, check: (value: unknown) => unknown, value: unknown) => {
  if (check(value) === null) throw new Error(`${label} was admitted`);
};

admits("a user", validateUser, { id: "42" });
admits("an absent nickname", validateUser, { id: "42" });
admits("a null nickname", validateUser, { id: "42", nickname: null });
refuses("a numeric carrier", validateUser, { id: 42 });
refuses("a non-canonical integer string", validateUser, { id: "007" });
refuses("a missing id", validateUser, {});
refuses("a nickname of the wrong type", validateUser, { id: "42", nickname: 7 });
refuses("an undefined body", validateUser, undefined);
const issue = validateUser({ id: 42 });
if (issue === null || !Array.isArray(issue.path) || issue.path[0] !== "id") {
  throw new Error(`the issue does not name the field: ${JSON.stringify(issue)}`);
}

admits("the lossy enum accepts any tag", validateLoginError, { code: "anything" });
refuses("the lossy enum still needs a string tag", validateLoginError, { code: 1 });
admits("the narrowed invalid variant", validateLoginErrorInvalid, { code: "invalid" });
refuses("a narrowed variant with another tag", validateLoginErrorInvalid, { code: "locked" });
refuses("a narrowed variant with no tag", validateLoginErrorInvalid, {});
admits("the narrowed locked variant", validateLoginErrorLocked, { code: "locked", retry_after: 5 });
refuses("the locked payload of the wrong type", validateLoginErrorLocked, { code: "locked", retry_after: "5" });
"#;
    run_bun(&output.files, harness);
}

#[test]
fn a_struct_without_a_union_or_a_map_is_exact() {
    let document = wallet();
    let output = emit_with_options(
        &document,
        TsOptions {
            validation: ValidationOptions::new(ValidationBackend::Yup),
            ..TsOptions::default()
        },
    )
    .unwrap();
    assert!(
        output.validation_coverage.lossy.is_empty(),
        "a plain struct is exact: {:?}",
        output.validation_coverage
    );
    assert!(
        output.validation_coverage.unsupported.is_empty(),
        "{:?}",
        output.validation_coverage
    );
    for subject in ["types.Wallet", "operations.wallet.read"] {
        assert!(
            output.validation_coverage.exact.contains(subject),
            "{subject} is exact: {:?}",
            output.validation_coverage
        );
    }
    let validators = file(&output.files, "validators.ts");
    assert!(validators.contains(".noUnknown(true)"), "{validators}");
    let harness = r#"
import { validateWallet } from "./validators.ts";

const admits = (label: string, value: unknown) => {
  const found = validateWallet(value);
  if (found !== null) throw new Error(`${label} was refused: ${JSON.stringify(found)}`);
};
const refuses = (label: string, value: unknown) => {
  if (validateWallet(value) === null) throw new Error(`${label} was admitted`);
};

admits("a wallet", { balance: 1 });
admits("a wallet with its label", { balance: 1, label: "ab" });
refuses("a numeric carrier on the wire", { balance: "1" });
refuses("a fractional balance", { balance: 1.5 });
refuses("a balance above its width", { balance: 2 ** 32 });
refuses("a label shorter than the contract", { balance: 1, label: "a" });
refuses("an unknown property", { balance: 1, extra: true });
"#;
    run_bun(&output.files, harness);
}

#[test]
fn a_map_is_lossy_so_strict_mode_refuses_it_and_lossy_mode_names_it() {
    let document = counts();
    let refusal = emit_with_options(
        &document,
        TsOptions {
            validation: ValidationOptions::new(ValidationBackend::Yup),
            ..TsOptions::default()
        },
    )
    .unwrap_err();
    for expected in ["yup", "types.Counts", "lossy"] {
        assert!(
            refusal.0.contains(expected),
            "the refusal names {expected}: {}",
            refusal.0
        );
    }
    let output = emit_with_options(
        &document,
        TsOptions {
            mode: EmitMode::Lossy,
            validation: ValidationOptions::new(ValidationBackend::Yup),
            ..TsOptions::default()
        },
    )
    .unwrap();
    assert!(
        output.validation_coverage.lossy.contains("types.Counts"),
        "the map is reported: {:?}",
        output.validation_coverage
    );
    let validators = file(&output.files, "validators.ts");
    assert!(validators.contains("yup.object()"), "{validators}");
    let harness = r#"
import { validateCounts } from "./validators.ts";

const admits = (label: string, value: unknown) => {
  const found = validateCounts(value);
  if (found !== null) throw new Error(`${label} was refused: ${JSON.stringify(found)}`);
};
const refuses = (label: string, value: unknown) => {
  if (validateCounts(value) === null) throw new Error(`${label} was admitted`);
};

admits("a map", { values: { a: 1 } });
refuses("a map that is not an object", { values: 1 });
"#;
    run_bun(&output.files, harness);
}
