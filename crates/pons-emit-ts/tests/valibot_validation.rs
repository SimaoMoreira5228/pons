use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use pons_emit_ts::{
    ErrorStrategy, GeneratedFile, TsOptions, ValidationBackend, ValidationOptions,
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

fn valibot(document: &Document) -> Vec<GeneratedFile> {
    emitted(
        document,
        TsOptions {
            validation: ValidationOptions::new(ValidationBackend::Valibot),
            errors: ErrorStrategy::Throw,
            ..TsOptions::default()
        },
    )
}

fn workspace_valibot() -> PathBuf {
    let mut directory = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    loop {
        let candidate = directory.join("node_modules").join("valibot");
        if candidate.join("package.json").is_file() {
            return candidate;
        }
        if !directory.pop() {
            panic!(
                "no valibot beside the workspace to install into a generated client, looked in \
                 {:?} and every directory above it; install it with `bun add --dev valibot`",
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

fn install_valibot(directory: &Path) {
    let scope = directory.join("node_modules");
    fs::create_dir_all(&scope).unwrap();
    link(&workspace_valibot(), &scope.join("valibot"));
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
    install_valibot(directory.path());
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
fn the_valibot_backend_emits_private_schemas_and_checks_a_login_response() {
    let document: Document = serde_json::from_value(fixture()).unwrap();
    let files = valibot(&document);
    let validators = file(&files, "validators.ts");
    assert!(
        validators.starts_with("import * as v from \"valibot\";\n"),
        "the generated file imports Valibot: {validators}"
    );
    for expected in [
        "import type { ValidationIssue, Validator } from \"./runtime.js\";",
        "const LoginErrorInvalidSchema = v.looseObject({\"code\": v.literal(\"invalid\")});",
        "const LoginErrorLockedSchema = v.looseObject({\"code\": v.literal(\"locked\"), \"retry_after\": integer(v.pipe(v.number(), v.integer()), 32, false)});",
        "const LoginErrorSchema = v.variant(\"code\", [LoginErrorInvalidSchema, LoginErrorLockedSchema]);",
        "const UserSchema = v.looseObject({\"id\": integer(v.pipe(v.string(), v.regex(CANONICAL_INTEGER)), 64, false), \"nickname\": v.optional(v.nullable(v.string()))});",
        "export const validateLoginErrorInvalid: Validator = mismatch(LoginErrorInvalidSchema);",
        "export const validateLoginErrorLocked: Validator = mismatch(LoginErrorLockedSchema);",
        "export const validateLoginError: Validator = mismatch(LoginErrorSchema);",
        "export const validateUser: Validator = mismatch(UserSchema);",
    ] {
        assert!(
            validators.contains(expected),
            "{expected} is missing: {validators}"
        );
    }
    assert!(
        !validators.contains("export const UserSchema")
            && !validators.contains("export const LoginErrorSchema"),
        "the schemas stay private: {validators}"
    );
    assert!(
        files.iter().all(|file| file.path != "schemas.ts"),
        "valibot writes no separate schema module"
    );
    let harness = r#"
import {
  validateLoginError, validateLoginErrorInvalid, validateUser,
} from "./validators.ts";

const admits = (label: string, check: (value: unknown) => unknown, value: unknown) => {
  const found = check(value);
  if (found !== null) throw new Error(`${label} was refused: ${JSON.stringify(found)}`);
};
const refuses = (label: string, check: (value: unknown) => unknown, value: unknown) => {
  const found = check(value);
  if (found === null) throw new Error(`${label} was admitted`);
  return found as { path: readonly (string | number)[] };
};

admits("a user", validateUser, { id: "1", nickname: null });
admits("a user without the optional nickname", validateUser, { id: "1" });
const carrier = refuses("a user whose wide integer is a number", validateUser, { id: 1 });
if (!Array.isArray(carrier.path) || carrier.path[0] !== "id") {
  throw new Error(`the wrong carrier does not name its path: ${JSON.stringify(carrier)}`);
}
refuses("a user whose wide integer is not canonical", validateUser, { id: "007" });
refuses("a user whose nickname is a number", validateUser, { id: "1", nickname: 1 });

admits("the invalid variant", validateLoginError, { code: "invalid" });
admits("the invalid variant carrying an extra key", validateLoginError, { code: "invalid", detail: "x" });
admits("the locked variant", validateLoginError, { code: "locked", retry_after: 1 });
const variant = refuses("an unknown variant", validateLoginError, { code: "ghost" });
if (!Array.isArray(variant.path) || variant.path[0] !== "code") {
  throw new Error(`the wrong variant does not name its path: ${JSON.stringify(variant)}`);
}
refuses("a locked variant without its field", validateLoginError, { code: "locked" });
refuses("a locked variant with the wrong field type", validateLoginError, { code: "locked", retry_after: "1" });
refuses("a variant without its tag", validateLoginError, { retry_after: 1 });

if (validateLoginErrorInvalid({ code: "invalid" }) !== null) {
  throw new Error("the narrowed validator refuses its own variant");
}
if (validateLoginErrorInvalid({ code: "locked", retry_after: 1 }) === null) {
  throw new Error("the narrowed validator admits another variant");
}
"#;
    run_bun(&files, harness);
}

#[test]
fn a_string_length_is_counted_in_code_points() {
    let mut raw = fixture();
    raw["types"] = json!({
        "Text": {
            "kind": "struct", "name": ["text"], "direction": "output", "openness": "reject",
            "fields": [{
                "wire": "label",
                "ident": ["label"],
                "ty": {"kind": "string", "min_length": 2, "max_length": 4},
                "presence": "required"
            }]
        }
    });
    raw["operations"] = json!([{
        "id": [["text"], ["read"]],
        "method": "GET",
        "path": [{"lit": "text"}],
        "auth": {"alternatives": [], "optional": false},
        "semantics": {"safe": true, "idempotent": true},
        "responses": [{"status": 200, "content": "json", "body": {"ref": "Text"}}],
        "errors": []
    }]);
    let document: Document = serde_json::from_value(raw).unwrap();
    let files = valibot(&document);
    let harness = r#"
import { validateText } from "./validators.ts";

const admits = (label: string, value: unknown) => {
  const found = validateText(value);
  if (found !== null) throw new Error(`${label} was refused: ${JSON.stringify(found)}`);
};
const refuses = (label: string, value: unknown) => {
  if (validateText(value) === null) throw new Error(`${label} was admitted`);
};

admits("two astral code points", { label: "😀😀" });
admits("three astral code points", { label: "😀😀😀" });
refuses("one astral code point", { label: "😀" });
refuses("five astral code points", { label: "😀😀😀😀😀" });
"#;
    run_bun(&files, harness);
}

#[test]
fn an_inline_body_gets_one_validator_the_operation_imports() {
    let mut raw = fixture();
    raw["operations"][0]["responses"][0]["body"] = json!({"kind": "string", "format": "email"});
    let document: Document = serde_json::from_value(raw).unwrap();
    let files = valibot(&document);
    let validators = file(&files, "validators.ts");
    assert!(
        validators.contains(
            "export const validateInline1: Validator = mismatch(v.pipe(v.string(), v.regex(EMAIL, \"expected an email\")));"
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

if (validateInline1("person@example.test") !== null) {
  throw new Error("a valid email was refused");
}
const found = validateInline1("not an email");
if (found === null) throw new Error("an invalid email was admitted");
if (!Array.isArray(found.path)) throw new Error(`the issue path is not an array: ${JSON.stringify(found)}`);
"#;
    run_bun(&files, harness);
}
