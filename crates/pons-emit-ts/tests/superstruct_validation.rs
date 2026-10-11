use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use pons_emit_ts::{
    EmitMode, ErrorStrategy, GeneratedFile, TsOptions, ValidationBackend, ValidationOptions,
    emit_with_options,
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

fn superstruct(document: &Document, options: TsOptions) -> Vec<GeneratedFile> {
    emitted(
        document,
        TsOptions {
            validation: ValidationOptions::new(ValidationBackend::Superstruct),
            errors: ErrorStrategy::Throw,
            ..options
        },
    )
}

fn workspace_superstruct() -> PathBuf {
    let mut directory = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    loop {
        let candidate = directory.join("node_modules").join("superstruct");
        if candidate.join("package.json").is_file() {
            return candidate;
        }
        if !directory.pop() {
            panic!(
                "no superstruct beside the workspace to install into a generated client, looked in \
                 {:?} and every directory above it; install it with `bun add --dev superstruct` or \
                 `npm install --save-dev superstruct`",
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

fn install_superstruct(directory: &Path) {
    let scope = directory.join("node_modules");
    fs::create_dir_all(&scope).unwrap();
    link(&workspace_superstruct(), &scope.join("superstruct"));
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
    install_superstruct(directory.path());
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

#[test]
fn the_superstruct_backend_writes_private_schemas_and_public_validators() {
    let document: Document = serde_json::from_value(fixture()).unwrap();
    let files = superstruct(&document, TsOptions::default());
    let validators = file(&files, "validators.ts");
    assert!(
        validators.starts_with("import {") && validators.contains("} from \"superstruct\";"),
        "the file imports the library it calls: {validators}"
    );
    assert!(
        validators.contains("function mismatch("),
        "the validator adapter is private: {validators}"
    );
    for expected in [
        "const UserSchema = openType(",
        "const LoginErrorInvalidSchema = openType({\"code\": literal(\"invalid\")});",
        "const LoginErrorLockedSchema = openType(",
        "const LoginErrorSchema = union([LoginErrorInvalidSchema, LoginErrorLockedSchema]);",
        "export const validateUser: Validator = mismatch(UserSchema);",
        "export const validateLoginError: Validator = mismatch(LoginErrorSchema);",
        "export const validateLoginErrorInvalid: Validator = mismatch(LoginErrorInvalidSchema);",
        "export const validateLoginErrorLocked: Validator = mismatch(LoginErrorLockedSchema);",
    ] {
        assert!(
            validators.contains(expected),
            "{expected} is missing: {validators}"
        );
    }
    for name in ["UserSchema", "LoginErrorSchema"] {
        assert!(
            !validators.contains(&format!("export const {name} =")),
            "{name} is not part of the public surface: {validators}"
        );
    }
    assert!(
        !files.iter().any(|file| file.path == "schemas.ts"),
        "the schema values stay private to validators.ts"
    );
    let auth = file(&files, "operations/auth.ts");
    assert!(
        auth.contains(
            "import { validateLoginErrorInvalid, validateUser } from \"../validators.js\";"
        ),
        "{auth}"
    );
    assert!(auth.contains("validate: validateUser"), "{auth}");
    assert!(
        auth.contains("validate: validateLoginErrorInvalid"),
        "{auth}"
    );
}

#[test]
fn a_schema_admits_a_valid_user_and_error_and_refuses_the_wrong_carrier_or_variant() {
    let document: Document = serde_json::from_value(fixture()).unwrap();
    let files = superstruct(&document, TsOptions::default());
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

admits("a user", validateUser, { id: "1", nickname: null });
admits("a user without the optional nickname", validateUser, { id: "1" });
refuses("a numeric id beside the string carrier", validateUser, { id: 1 });
refuses("a non-canonical integer string", validateUser, { id: "007" });
refuses("a nickname that is not a string", validateUser, { id: "1", nickname: 1 });

admits("the invalid variant", validateLoginError, { code: "invalid" });
admits("the locked variant", validateLoginError, { code: "locked", retry_after: 1 });
admits("an internal unit variant carrying an extra key", validateLoginError, {
  code: "invalid",
  detail: "preserved",
});
refuses("an unknown variant", validateLoginError, { code: "ghost" });
refuses("the locked variant without its field", validateLoginError, { code: "locked" });
refuses("the locked variant with a string retry_after", validateLoginError, {
  code: "locked",
  retry_after: "1",
});
refuses("a variant without its tag", validateLoginError, { retry_after: 1 });

admits("the narrowed variant", validateLoginErrorInvalid, { code: "invalid" });
refuses("the narrowed variant against another variant", validateLoginErrorInvalid, {
  code: "locked",
  retry_after: 1,
});
refuses("the locked narrowed variant against the unit variant", validateLoginErrorLocked, {
  code: "invalid",
});
"#;
    run_bun(&files, harness);
}

#[test]
fn an_inline_body_gets_one_private_validator() {
    let mut raw = fixture();
    raw["operations"][0]["responses"][0]["body"] = json!({"kind": "string", "format": "email"});
    let document: Document = serde_json::from_value(raw).unwrap();
    let files = superstruct(&document, TsOptions::default());
    let validators = file(&files, "validators.ts");
    assert!(
        validators.contains(
            "export const validateInline1: Validator = mismatch(formatStruct(\"email\"));"
        ),
        "{validators}"
    );
    let auth = file(&files, "operations/auth.ts");
    assert!(
        auth.contains(
            "import { validateInline1, validateLoginErrorInvalid } from \"../validators.js\";"
        ),
        "{auth}"
    );
    assert!(auth.contains("validate: validateInline1"), "{auth}");
    let harness = r#"
import { validateInline1 } from "./validators.ts";

if (validateInline1("person@example.test") !== null) throw new Error("an email was refused");
if (validateInline1("not-an-email") === null) throw new Error("a non-email was admitted");
"#;
    run_bun(&files, harness);
}

#[test]
fn a_preserve_typed_object_is_lossy_rather_than_exact() {
    let mut raw = fixture();
    raw["types"]["Typed"] = json!({
        "kind": "struct", "name": ["typed"], "direction": "output",
        "openness": {"preserve_typed": {"value": {"kind": "int", "bits": 32, "signed": false, "wire": "number"}}},
        "fields": [
            {"wire": "a", "ident": ["a"], "ty": {"kind": "int", "bits": 32, "signed": false, "wire": "number"}, "presence": "required"}
        ]
    });
    raw["operations"][0]["responses"][0]["body"] = json!({"ref": "Typed"});
    raw["operations"][0]["provides"] = Value::Null;
    let document: Document = serde_json::from_value(raw).unwrap();
    let output = emit_with_options(
        &document,
        TsOptions {
            mode: EmitMode::Lossy,
            validation: ValidationOptions::new(ValidationBackend::Superstruct),
            errors: ErrorStrategy::Throw,
            ..TsOptions::default()
        },
    )
    .unwrap();
    assert!(
        output.validation_coverage.lossy.contains("types.Typed"),
        "a typed catchall has no superstruct equivalent: {:?}",
        output.validation_coverage
    );
    assert!(
        !output.validation_coverage.exact.contains("types.Typed"),
        "a typed catchall is not exact: {:?}",
        output.validation_coverage
    );
    let validators = file(&output.files, "validators.ts");
    assert!(
        validators.contains("const TypedSchema = openType("),
        "the emitted check is the open form it can express: {validators}"
    );
}
