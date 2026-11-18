mod support;

use std::{fs, path::Path, process::Command};

use pons_emit_ts::{
    DateMode, EmitMode, ErrorStrategy, GeneratedFile, Int64, TsOptions, ValidationBackend,
    ValidationOptions, emit_with_options,
};
use pons_ir::Document;
use serde_json::{Value, json};

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

fn fixture() -> Value {
    serde_json::from_str(include_str!("../../../spec/fixtures/login.json")).unwrap()
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

fn arktype(document: &Document, options: TsOptions) -> Vec<GeneratedFile> {
    emitted(
        document,
        TsOptions {
            validation: ValidationOptions::new(ValidationBackend::Arktype),
            errors: ErrorStrategy::Throw,
            ..options
        },
    )
}

fn link(source: &Path, destination: &Path) {
    #[cfg(unix)]
    std::os::unix::fs::symlink(source, destination).unwrap();
    #[cfg(windows)]
    std::os::windows::fs::symlink_dir(source, destination).unwrap();
}

fn install_arktype(directory: &Path) {
    let scope = directory.join("node_modules");
    fs::create_dir_all(&scope).unwrap();
    link(&support::package("arktype"), &scope.join("arktype"));
}

fn write_files(files: &[GeneratedFile], directory: &Path) {
    for file in files {
        let destination = directory.join(&file.path);
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(destination, &file.content).unwrap();
    }
}

fn run_bun(files: &[GeneratedFile], harness: &str) {
    let version = Command::new("bun")
        .arg("--version")
        .output()
        .expect("Bun is required for generated TypeScript runtime tests");
    assert!(version.status.success());
    let directory = tempfile::tempdir().unwrap();
    write_files(files, directory.path());
    install_arktype(directory.path());
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

fn type_check(files: &[GeneratedFile]) {
    let compiler = support::type_check::tsc_command();
    let version = Command::new(&compiler.program)
        .args(&compiler.arguments)
        .arg("--version")
        .output()
        .expect("the TypeScript compiler is required for generated schema type checks");
    assert!(version.status.success());
    let directory = tempfile::tempdir().unwrap();
    write_files(files, directory.path());
    install_arktype(directory.path());
    fs::write(
        directory.path().join("tsconfig.json"),
        format!(r#"{{"compilerOptions": {COMPILER_OPTIONS}, "include": ["**/*.ts"]}}"#),
    )
    .unwrap();
    let output = Command::new(&compiler.program)
        .args(&compiler.arguments)
        .args(["--project", "tsconfig.json"])
        .current_dir(directory.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "tsc refused the emitted validators\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn the_login_fixture_gets_one_scope_and_a_validator_for_every_subject() {
    let document: Document = serde_json::from_value(fixture()).unwrap();
    let files = arktype(&document, TsOptions::default());
    let validators = file(&files, "validators.ts");
    for expected in [
        "import { scope, type as ark } from \"arktype\";",
        "const $ = scope({",
        "LoginError: \"LoginErrorInvalid | LoginErrorLocked\"",
        "validateInline1: \"string\"",
        "export const validateUser: Validator = mismatch($.User);",
        "export const validateLoginError: Validator = mismatch($.LoginError);",
        "export const validateLoginErrorInvalid: Validator = mismatch($.LoginErrorInvalid);",
        "export const validateLoginErrorLocked: Validator = mismatch($.LoginErrorLocked);",
    ] {
        assert!(
            validators.contains(expected),
            "missing {expected}: {validators}"
        );
    }
    assert!(
        !files.iter().any(|file| file.path == "schemas.ts"),
        "an arktype module is one file, not a schema file beside it"
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
fn an_inline_body_gets_one_validator_the_operation_imports() {
    let mut raw = fixture();
    raw["operations"][0]["responses"][0]["body"] = json!({"kind": "string", "format": "email"});
    let document: Document = serde_json::from_value(raw).unwrap();
    let files = arktype(&document, TsOptions::default());
    let validators = file(&files, "validators.ts");
    assert!(
        validators.contains("validateInline1: ark(EMAIL),"),
        "{validators}"
    );
    assert!(
        validators
            .contains("export const validateInline1: Validator = mismatch($.validateInline1);"),
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
}

#[test]
fn the_scope_admits_the_wire_values_the_contract_admits_and_refuses_the_rest() {
    let document: Document = serde_json::from_value(fixture()).unwrap();
    let files = arktype(&document, TsOptions::default());
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

admits("a user", validateUser, { id: "123", nickname: null });
admits("a user without the optional field", validateUser, { id: "123" });
admits("a user with another preserved property", validateUser, { id: "123", extra: true });
refuses("a user with a number where the contract says text", validateUser, { id: 123 });
refuses("a user that is not an object", validateUser, null);

admits("an invalid unit variant", validateLoginError, { code: "invalid" });
admits("a unit variant with an extra key", validateLoginError, { code: "invalid", extra: 1 });
admits("a locked payload", validateLoginError, { code: "locked", retry_after: 5 });
refuses("an unknown variant", validateLoginError, { code: "ghost" });
refuses("a locked payload without its field", validateLoginError, { code: "locked" });
refuses("a negative retry", validateLoginError, { code: "locked", retry_after: -1 });
refuses("the other narrowed variant", validateLoginErrorInvalid, { code: "locked", retry_after: 1 });
admits("the narrowed variant", validateLoginErrorLocked, { code: "locked", retry_after: 1 });

const issue = validateLoginError({ code: "locked", retry_after: "5" });
if (issue === null) throw new Error("a string retry was admitted");
if (JSON.stringify(issue.path) !== JSON.stringify(["retry_after"])) {
  throw new Error(`the path is not the failing field: ${JSON.stringify(issue.path)}`);
}
if (typeof issue.message !== "string" || issue.message.length === 0) {
  throw new Error("the issue names what it saw");
}
"#;
    run_bun(&files, harness);
}

#[test]
fn a_recursive_alias_and_a_native_format_keep_the_scope_and_map_the_path() {
    let mut raw = fixture();
    raw["types"] = json!({
        "Branch": {
            "kind": "struct", "name": ["branch"], "direction": "both", "openness": "reject",
            "fields": [
                {"wire": "value", "ident": ["value"], "ty": {"kind": "int", "bits": 32, "signed": false, "wire": "number"}, "presence": "required"},
                {"wire": "child", "ident": ["child"], "ty": {"kind": "nullable", "inner": {"ref": "Branch"}}, "presence": "required"}
            ]
        },
        "Contact": {
            "kind": "struct", "name": ["contact"], "direction": "both", "openness": "reject",
            "fields": [
                {"wire": "email", "ident": ["email"], "ty": {"kind": "string", "format": "email"}, "presence": "required"},
                {"wire": "at", "ident": ["at"], "ty": {"kind": "string", "format": "date-time"}, "presence": "required"}
            ]
        },
        "Bag": {
            "kind": "struct", "name": ["bag"], "direction": "both", "openness": "reject",
            "fields": [{"wire": "names", "ident": ["names"], "ty": {"kind": "array", "item": {"kind": "string"}}, "presence": "required"}]
        }
    });
    raw["operations"] = json!([{
        "id": [["branch"], ["read"]],
        "method": "GET",
        "path": [{"lit": "branch"}],
        "auth": {"alternatives": [], "optional": false},
        "semantics": {"safe": true, "idempotent": true},
        "responses": [{"status": 200, "content": "json", "body": {"ref": "Branch"}}],
        "errors": []
    }]);
    let document: Document = serde_json::from_value(raw).unwrap();
    let files = arktype(&document, TsOptions::default());
    let validators = file(&files, "validators.ts");
    assert!(
        validators.contains("\"child\": [\"Branch\", \"|\", \"null\"]"),
        "recursion is a scope alias, not a JavaScript name read before the scope exists: {validators}"
    );
    assert!(
        validators.contains("\"email\": ark(EMAIL)"),
        "a format is the contract regex, not a library keyword: {validators}"
    );
    assert!(validators.contains("\"at\": instant()"), "{validators}");
    type_check(&files);
    let harness = r#"
import { validateBag, validateBranch, validateContact } from "./validators.ts";

const admits = (label: string, check: (value: unknown) => unknown, value: unknown) => {
  const found = check(value);
  if (found !== null) throw new Error(`${label} was refused: ${JSON.stringify(found)}`);
};
const refuses = (label: string, check: (value: unknown) => unknown, value: unknown) => {
  if (check(value) === null) throw new Error(`${label} was admitted`);
};
const paths = (label: string, check: (value: unknown) => any, value: unknown) => {
  const found = check(value);
  if (found === null) throw new Error(`${label} was admitted`);
  return found.path as Array<string | number>;
};

admits("a nested branch", validateBranch, { value: 1, child: { value: 2, child: null } });
refuses("a branch without its child", validateBranch, { value: 1 });
refuses("a branch with a bad nested value", validateBranch, { value: 1, child: { value: "x", child: null } });
const branchPath = paths("a bad branch", validateBranch, { value: 1, child: { value: "x", child: null } });
if (branchPath[0] !== "child") throw new Error(`the alias lost the path: ${JSON.stringify(branchPath)}`);

admits("a contact", validateContact, { email: "a@b.test", at: "2024-02-29T00:00:00Z" });
refuses("an email the regex refuses", validateContact, { email: "nope", at: "2024-02-29T00:00:00Z" });
refuses("a calendar day that does not exist", validateContact, { email: "a@b.test", at: "2024-02-30T00:00:00Z" });
const emailPath = paths("a bad email", validateContact, { email: "nope", at: "2024-02-29T00:00:00Z" });
if (JSON.stringify(emailPath) !== JSON.stringify(["email"])) {
  throw new Error(`the format path is not the field: ${JSON.stringify(emailPath)}`);
}

admits("a bag", validateBag, { names: ["a", "b"] });
refuses("a bag item of the wrong type", validateBag, { names: ["a", 1] });
const itemPath = paths("a bad item", validateBag, { names: ["a", 1] });
if (JSON.stringify(itemPath) !== JSON.stringify(["names", 1])) {
  throw new Error(`an array index is not a number in the path: ${JSON.stringify(itemPath)}`);
}
"#;
    run_bun(&files, harness);
}

#[test]
fn a_typed_catchall_is_lossy_when_the_index_cannot_be_exact() {
    let mut raw = fixture();
    raw["types"] = json!({
        "StrictTyped": {
            "kind": "struct", "name": ["strict", "typed"], "direction": "both",
            "openness": {"preserve_typed": {"value": {"kind": "string"}}},
            "fields": [
                {"wire": "a", "ident": ["a"], "ty": {"kind": "string"}, "presence": "required"},
                {"wire": "b", "ident": ["b"], "ty": {"kind": "string"}, "presence": "required"}
            ]
        },
        "OptionalTyped": {
            "kind": "struct", "name": ["optional", "typed"], "direction": "both",
            "openness": {"preserve_typed": {"value": {"kind": "string"}}},
            "fields": [
                {"wire": "a", "ident": ["a"], "ty": {"kind": "string"}, "presence": "required"},
                {"wire": "b", "ident": ["b"], "ty": {"kind": "string"}, "presence": "optional"}
            ]
        },
        "Keyed": {
            "kind": "struct", "name": ["keyed"], "direction": "both", "openness": "reject",
            "fields": [{"wire": "table", "ident": ["table"], "ty": {"kind": "map", "key": {"kind": "string", "format": "uuid"}, "value": {"kind": "string"}}, "presence": "required"}]
        }
    });
    raw["operations"] = json!([{
        "id": [["typed"], ["read"]],
        "method": "GET",
        "path": [{"lit": "typed"}],
        "auth": {"alternatives": [], "optional": false},
        "semantics": {"safe": true, "idempotent": true},
        "responses": [{"status": 200, "content": "json", "body": {"ref": "StrictTyped"}}],
        "errors": []
    }]);
    let document: Document = serde_json::from_value(raw).unwrap();
    let output = emit_with_options(
        &document,
        TsOptions {
            validation: ValidationOptions::new(ValidationBackend::Arktype),
            errors: ErrorStrategy::Throw,
            mode: EmitMode::Lossy,
            ..TsOptions::default()
        },
    )
    .unwrap();
    assert!(
        output
            .validation_coverage
            .exact
            .contains("types.StrictTyped")
    );
    assert!(
        output
            .validation_coverage
            .lossy
            .contains("types.OptionalTyped")
    );
    assert!(output.validation_coverage.lossy.contains("types.Keyed"));
    assert!(output.validation_coverage.unsupported.is_empty());

    let validators = file(&output.files, "validators.ts");
    assert!(
        validators.contains(
            "StrictTyped: [{ \"a\": \"string\", \"b\": \"string\", \"[string]\": \"string\" }, \"&\", plainRecord]"
        ),
        "an all-required catchall of the field type is exact: {validators}"
    );
    assert!(
        validators.contains(
            "OptionalTyped: [{ \"a\": \"string\", \"b?\": [\"string\", \"|\", \"undefined\"] }, \"&\", plainRecord]"
        ),
        "an optional field leaves the index unenforceable, so the catchall is dropped rather than tightened: {validators}"
    );
    assert!(
        validators.contains(
            "Keyed: [{ \"+\": \"reject\", \"table\": [{ \"[string]\": \"string\" }, \"&\", plainRecord] }, \"&\", plainRecord]"
        ),
        "a formatted record key is checked as a plain string key and reported: {validators}"
    );

    let refusal = emit_with_options(
        &document,
        TsOptions {
            validation: ValidationOptions::new(ValidationBackend::Arktype),
            errors: ErrorStrategy::Throw,
            mode: EmitMode::Strict,
            ..TsOptions::default()
        },
    )
    .unwrap_err();
    for subject in ["types.OptionalTyped", "types.Keyed"] {
        assert!(
            refusal.0.contains(&format!(
                "the arktype validation backend cannot check {subject} exactly"
            )),
            "{}",
            refusal.0
        );
    }

    let harness = r#"
import { validateKeyed, validateOptionalTyped, validateStrictTyped } from "./validators.ts";

const admits = (label: string, check: (value: unknown) => unknown, value: unknown) => {
  const found = check(value);
  if (found !== null) throw new Error(`${label} was refused: ${JSON.stringify(found)}`);
};
const refuses = (label: string, check: (value: unknown) => unknown, value: unknown) => {
  if (check(value) === null) throw new Error(`${label} was admitted`);
};

admits("the exact catchall", validateStrictTyped, { a: "x", b: "y", extra: "z" });
refuses("a catchall value of the wrong type", validateStrictTyped, { a: "x", b: "y", extra: 1 });
refuses("a declared field of the wrong type", validateStrictTyped, { a: 1, b: "y" });

admits("the widened catchall", validateOptionalTyped, { a: "x", b: "y", extra: 1 });
refuses("a declared field of the wrong type", validateOptionalTyped, { a: 1 });

admits("a record with a key the contract would refuse", validateKeyed, {
  table: { "not-a-uuid": "x" },
});
refuses("a record value of the wrong type", validateKeyed, { table: { "not-a-uuid": 1 } });
"#;
    run_bun(&output.files, harness);
}

#[test]
fn an_invalid_date_and_a_hostile_value_are_issues_not_throws() {
    let mut raw = fixture();
    raw["types"] = json!({
        "Stamped": {
            "kind": "struct", "name": ["stamped"], "direction": "output", "openness": "reject",
            "fields": [{
                "wire": "at",
                "ident": ["at"],
                "ty": {"kind": "string", "format": "date-time"},
                "presence": "required"
            }]
        }
    });
    raw["operations"] = json!([{
        "id": [["stamped"], ["read"]],
        "method": "GET",
        "path": [{"lit": "stamped"}],
        "auth": {"alternatives": [], "optional": false},
        "semantics": {"safe": true, "idempotent": true},
        "responses": [{"status": 200, "content": "json", "body": {"ref": "Stamped"}}],
        "errors": []
    }]);
    let document: Document = serde_json::from_value(raw).unwrap();
    let files = arktype(
        &document,
        TsOptions {
            date_mode: DateMode::Date,
            ..TsOptions::default()
        },
    );
    let harness = r#"
import { validateStamped } from "./validators.ts";

const admits = (label: string, value: unknown) => {
  const found = validateStamped(value);
  if (found !== null) throw new Error(`${label} was refused: ${JSON.stringify(found)}`);
};
const refuses = (label: string, value: unknown) => {
  const found = validateStamped(value);
  if (found === null) throw new Error(`${label} was admitted`);
  return found;
};

admits("a real date", { at: new Date("2024-01-01T00:00:00Z") });
const invalid = refuses("an invalid date", { at: new Date(Number.NaN) });
if (!Array.isArray(invalid.path)) throw new Error(`the issue has no path: ${JSON.stringify(invalid)}`);

const hostile = new Proxy({}, {
  get(target, property) {
    if (property === "at") throw new Error("the proxy refused the read");
    return Reflect.get(target, property);
  },
  ownKeys() {
    throw new Error("the proxy refused the keys");
  },
});
const trapped = refuses("a hostile proxy", hostile);
if (typeof trapped.message !== "string" || trapped.message.length === 0) {
  throw new Error(`the issue does not name the throw: ${JSON.stringify(trapped)}`);
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
    let files = arktype(&document, TsOptions::default());
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
fn a_constrained_native_integer_and_a_strict_bigint_compile_and_check() {
    let mut raw = fixture();
    raw["types"]["User"]["fields"] = json!([
        {
            "wire": "small",
            "ident": ["small"],
            "ty": {
                "kind": "int",
                "bits": 8,
                "signed": true,
                "wire": "number",
                "minimum": -5,
                "maximum": 5,
                "multiple_of": 5
            },
            "presence": "required"
        },
        {
            "wire": "wide",
            "ident": ["wide"],
            "ty": {"kind": "int", "bits": 64, "signed": true, "wire": "number"},
            "presence": "required"
        }
    ]);
    let document: Document = serde_json::from_value(raw).unwrap();
    let files = arktype(
        &document,
        TsOptions {
            int64: Int64::Bigint,
            ..TsOptions::default()
        },
    );
    let harness = r#"
import { validateUser } from "./validators.ts";

const admits = (label: string, check: (value: unknown) => unknown, value: unknown) => {
  const found = check(value);
  if (found !== null) throw new Error(`${label} was refused: ${JSON.stringify(found)}`);
};
const refuses = (label: string, check: (value: unknown) => unknown, value: unknown) => {
  if (check(value) === null) throw new Error(`${label} was admitted`);
};

admits("a constrained integer and a bigint", validateUser, { small: 5, wide: 1n });
refuses("an integer outside its multiple", validateUser, { small: 2, wide: 1n });
refuses("a number under the bigint carrier", validateUser, { small: 5, wide: 1 });
refuses("an integer outside its range", validateUser, { small: 10, wide: 1n });
"#;
    run_bun(&files, harness);
}

#[test]
fn a_fractional_multiple_is_widened_instead_of_dropped() {
    let mut raw = fixture();
    raw["types"]["User"]["fields"] = json!([
        {
            "wire": "ratio",
            "ident": ["ratio"],
            "ty": {
                "kind": "float",
                "bits": 64,
                "minimum": 0,
                "multiple_of": 0.5
            },
            "presence": "required"
        }
    ]);
    let document: Document = serde_json::from_value(raw).unwrap();
    let output = emit_with_options(
        &document,
        TsOptions {
            mode: EmitMode::Lossy,
            validation: ValidationOptions::new(ValidationBackend::Arktype),
            ..TsOptions::default()
        },
    )
    .unwrap();
    assert!(
        output.validation_coverage.lossy.contains("types.User"),
        "{:?}",
        output.validation_coverage
    );
    let validators = file(&output.files, "validators.ts");
    assert!(validators.contains("number >= 0"), "{validators}");
    assert!(!validators.contains("% 0.5"), "{validators}");
}
