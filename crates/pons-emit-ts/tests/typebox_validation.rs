use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use pons_emit_ts::{
    DateMode, EmitMode, ErrorStrategy, GeneratedFile, Int64, Runtime, TsOptions, ValidationBackend,
    ValidationOptions, emit_with_options,
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

fn typebox(document: &Document, options: TsOptions) -> Vec<GeneratedFile> {
    emitted(
        document,
        TsOptions {
            validation: ValidationOptions::new(ValidationBackend::Typebox),
            errors: ErrorStrategy::Throw,
            ..options
        },
    )
}

fn workspace_typebox() -> PathBuf {
    let mut directory = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    loop {
        let candidate = directory.join("node_modules").join("typebox");
        if candidate.join("package.json").is_file() {
            return candidate;
        }
        if !directory.pop() {
            panic!(
                "no typebox beside the workspace to install into a generated client, looked in \
                 {:?} and every directory above it; install it with `bun add --dev typebox` or \
                 `npm install --save-dev typebox`",
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

fn install_typebox(directory: &Path) {
    let scope = directory.join("node_modules");
    fs::create_dir_all(&scope).unwrap();
    link(&workspace_typebox(), &scope.join("typebox"));
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
    install_typebox(directory.path());
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
fn the_module_declares_the_types_and_the_operations_import_their_validators() {
    let document: Document = serde_json::from_value(fixture()).unwrap();
    let files = typebox(&document, TsOptions::default());
    let validators = file(&files, "validators.ts");
    assert!(
        validators.starts_with(
            "import Type from \"typebox\";\nimport Value from \"typebox/value\";\nimport type { ValidationIssue, Validator } from \"./runtime.js\";\nexport type { ValidationIssue, Validator } from \"./runtime.js\";\n\n"
        ),
        "{validators}"
    );
    for expected in [
        "const M = Type.Module({",
        "LoginErrorInvalid: Type.Object({\"code\": Type.Literal(\"invalid\")})",
        "LoginErrorLocked: Type.Object({\"code\": Type.Literal(\"locked\"), \"retry_after\": integer(Type.Integer(), 32, false)})",
        "LoginError: Type.Union([Type.Ref(\"LoginErrorInvalid\"), Type.Ref(\"LoginErrorLocked\")])",
        "User: Type.Object({\"id\": integer(Type.String({ pattern: \"^(0|[1-9][0-9]*)$\" }), 64, false), \"nickname\": Type.Optional(Type.Union([Type.String(), Type.Null()]))})",
        "export const validateLoginErrorInvalid: Validator = mismatch(M.LoginErrorInvalid);",
        "export const validateLoginErrorLocked: Validator = mismatch(M.LoginErrorLocked);",
        "export const validateLoginError: Validator = mismatch(M.LoginError);",
        "export const validateUser: Validator = mismatch(M.User);",
    ] {
        assert!(
            validators.contains(expected),
            "{expected} is missing: {validators}"
        );
    }
    assert!(
        !files.iter().any(|file| file.path == "schemas.ts"),
        "TypeBox keeps its module private to validators.ts"
    );
    assert!(
        file(&files, "index.ts").contains("export * from \"./validators.js\";"),
        "{}",
        file(&files, "index.ts")
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

    let harness = r#"
import { validateLoginError, validateUser } from "./validators.ts";

const admits = (label: string, check: (value: unknown) => unknown, value: unknown) => {
  const found = check(value);
  if (found !== null) throw new Error(`${label} was refused: ${JSON.stringify(found)}`);
};
const refuses = (label: string, check: (value: unknown) => unknown, value: unknown) => {
  if (check(value) === null) throw new Error(`${label} was admitted`);
};

admits("a user", validateUser, { id: "1" });
admits("a user with a nickname", validateUser, { id: "1", nickname: "a" });
admits("a user with a null nickname", validateUser, { id: "1", nickname: null });
admits("a user with an opaque extra", validateUser, { id: "1", extra: { anything: true } });
refuses("a numeric carrier", validateUser, { id: 1 });
refuses("a non-canonical integer", validateUser, { id: "01" });
refuses("a user without its id", validateUser, {});

admits("a unit variant", validateLoginError, { code: "invalid" });
admits("a payload variant", validateLoginError, { code: "locked", retry_after: 1 });
admits("an extra key on a unit variant", validateLoginError, { code: "invalid", extra: true });
refuses("an unknown variant", validateLoginError, { code: "ghost" });
refuses("a variant without its payload", validateLoginError, { code: "locked" });
refuses("a negative retry", validateLoginError, { code: "locked", retry_after: -1 });
"#;
    run_bun(&files, harness);
}

#[test]
fn an_inline_body_gets_one_private_validator_the_operation_imports() {
    let mut raw = fixture();
    raw["operations"][0]["responses"][0]["body"] = json!({"kind": "string", "format": "email"});
    let document: Document = serde_json::from_value(raw).unwrap();
    let files = typebox(&document, TsOptions::default());
    let validators = file(&files, "validators.ts");
    assert!(
        validators.contains(
            "export const validateInline1: Validator = mismatch(format(Type.String(), \"email\"));"
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

if (validateInline1("person@example.test") !== null) throw new Error("a valid email was refused");
const refused = validateInline1("nope");
if (refused === null) throw new Error("a broken email was admitted");
if (refused.path.length !== 0) throw new Error(`a scalar path is empty: ${JSON.stringify(refused.path)}`);
"#;
    run_bun(&files, harness);
}

#[test]
fn a_recursive_type_is_a_ref_inside_the_module_and_the_pointer_keeps_its_shape() {
    let mut raw = fixture();
    raw["types"] = json!({
        "Tree": {
            "kind": "struct", "name": ["tree"], "direction": "output", "openness": "reject",
            "fields": [
                {"wire": "value", "ident": ["value"], "ty": {"kind": "int", "bits": 32, "signed": false, "wire": "number"}, "presence": "required"},
                {"wire": "next", "ident": ["next"], "ty": {"kind": "nullable", "inner": {"ref": "Tree"}}, "presence": "required"},
                {"wire": "label", "ident": ["label"], "ty": {"kind": "string", "format": "email"}, "presence": "required"}
            ]
        }
    });
    raw["operations"] = json!([{
        "id": [["tree"], ["read"]],
        "method": "GET",
        "path": [{"lit": "tree"}],
        "auth": {"alternatives": [], "optional": false},
        "semantics": {"safe": true, "idempotent": true},
        "responses": [{"status": 200, "content": "json", "body": {"ref": "Tree"}}],
        "errors": []
    }]);
    let document: Document = serde_json::from_value(raw).unwrap();
    let files = typebox(&document, TsOptions::default());
    let validators = file(&files, "validators.ts");
    assert!(
        validators.contains(
            "Tree: Type.Object({\"label\": format(Type.String(), \"email\"), \"next\": Type.Union([Type.Ref(\"Tree\"), Type.Null()]), \"value\": integer(Type.Integer(), 32, false)}, { additionalProperties: false })"
        ),
        "{validators}"
    );
    assert!(
        validators.contains("export const validateTree: Validator = mismatch(M.Tree);"),
        "{validators}"
    );
    let harness = r#"
import { validateTree } from "./validators.ts";

const valid = { value: 1, label: "a@b.test", next: { value: 2, label: "c@d.test", next: null } };
if (validateTree(valid) !== null) throw new Error(`a recursive value was refused: ${JSON.stringify(validateTree(valid))}`);
const refused = validateTree({ value: 1, label: "a@b.test", next: { value: 2, label: "nope", next: null } });
if (refused === null) throw new Error("a nested format breach was admitted");
if (JSON.stringify(refused.path) !== JSON.stringify(["next", "label"])) {
  throw new Error(`the pointer did not keep its segments: ${JSON.stringify(refused.path)}`);
}
const missing = validateTree({ value: 1, label: "a@b.test" });
if (missing === null || JSON.stringify(missing.path) !== JSON.stringify([])) {
  throw new Error(`a missing property is an object issue: ${JSON.stringify(missing)}`);
}
"#;
    run_bun(&files, harness);
}

#[test]
fn the_import_is_the_package_root_and_the_value_subpath_is_derived() {
    let document: Document = serde_json::from_value(fixture()).unwrap();
    let custom = emitted(
        &document,
        TsOptions {
            validation: ValidationOptions {
                backend: ValidationBackend::Typebox,
                import_from: Some("@fork/typebox".into()),
            },
            errors: ErrorStrategy::Throw,
            ..TsOptions::default()
        },
    );
    let validators = file(&custom, "validators.ts");
    assert!(
        validators.starts_with(
            "import Type from \"@fork/typebox\";\nimport Value from \"@fork/typebox/value\";\n"
        ),
        "{validators}"
    );

    let refusal = emit_with_options(
        &document,
        TsOptions {
            validation: ValidationOptions {
                backend: ValidationBackend::Typebox,
                import_from: Some("typebox/value".into()),
            },
            ..TsOptions::default()
        },
    )
    .unwrap_err();
    assert!(
        refusal.0.contains("package root") && refusal.0.contains("typebox/value"),
        "the refusal names the root rule and the subpath: {}",
        refusal.0
    );
}

#[test]
fn the_carrier_options_reach_the_schema_they_change() {
    let mut raw = fixture();
    raw["types"]["User"] = json!({
        "kind": "struct", "name": ["user"], "direction": "output", "openness": "reject",
        "fields": [
            {"wire": "id", "ident": ["id"], "ty": {"kind": "int", "bits": 64, "signed": true, "wire": "number"}, "presence": "required"},
            {"wire": "moment", "ident": ["moment"], "ty": {"kind": "string", "format": "date-time"}, "presence": "required"}
        ]
    });
    raw["operations"] = json!([{
        "id": [["wide"], ["read"]],
        "method": "GET",
        "path": [{"lit": "wide"}],
        "auth": {"alternatives": [], "optional": false},
        "semantics": {"safe": true, "idempotent": true},
        "responses": [{"status": 200, "content": "json", "body": {"ref": "User"}}],
        "errors": []
    }]);
    let document: Document = serde_json::from_value(raw).unwrap();
    let schemas =
        |options: TsOptions| file(&typebox(&document, options), "validators.ts").to_owned();

    let strict = schemas(TsOptions::default());
    assert!(
        strict.contains("\"id\": integer(Type.BigInt(), 64, true)"),
        "a wide integer is a bigint in strict mode: {strict}"
    );
    assert!(
        strict.contains("\"moment\": format(Type.String(), \"date-time\")"),
        "{strict}"
    );

    let text = schemas(TsOptions {
        int64: Int64::String,
        ..TsOptions::default()
    });
    assert!(
        text.contains(
            "\"id\": integer(Type.String({ pattern: \"^-?(0|[1-9][0-9]*)$\" }), 64, true)"
        ),
        "int64 = string carries it as canonical text: {text}"
    );

    let dates = schemas(TsOptions {
        date_mode: DateMode::Date,
        ..TsOptions::default()
    });
    assert!(
        dates.contains("\"moment\": dateCarrier()"),
        "date_mode = date reads an instant as a Date: {dates}"
    );
}

#[test]
fn a_map_with_a_key_that_is_not_plain_text_is_lossy() {
    let mut raw = fixture();
    raw["types"] = json!({
        "Keyed": {
            "kind": "struct", "name": ["keyed"], "direction": "output", "openness": "reject",
            "fields": [{
                "wire": "table",
                "ident": ["table"],
                "ty": {"kind": "map", "key": {"kind": "string", "format": "uuid"}, "value": {"kind": "string"}},
                "presence": "required"
            }]
        }
    });
    raw["operations"] = json!([{
        "id": [["keyed"], ["read"]],
        "method": "GET",
        "path": [{"lit": "keyed"}],
        "auth": {"alternatives": [], "optional": false},
        "semantics": {"safe": true, "idempotent": true},
        "responses": [{"status": 200, "content": "json", "body": {"ref": "Keyed"}}],
        "errors": []
    }]);
    let document: Document = serde_json::from_value(raw).unwrap();
    let refusal = emit_with_options(
        &document,
        TsOptions {
            validation: ValidationOptions::new(ValidationBackend::Typebox),
            errors: ErrorStrategy::Throw,
            ..TsOptions::default()
        },
    )
    .unwrap_err();
    for expected in ["typebox", "types.Keyed", "lossy"] {
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
            validation: ValidationOptions::new(ValidationBackend::Typebox),
            errors: ErrorStrategy::Throw,
            ..TsOptions::default()
        },
    )
    .unwrap();
    for subject in ["types.Keyed", "operations.keyed.read"] {
        assert!(
            output.validation_coverage.lossy.contains(subject),
            "{subject} is lossy: {:?}",
            output.validation_coverage
        );
    }
    let validators = file(&output.files, "validators.ts");
    assert!(
        validators.contains("\"table\": Type.Record(Type.String(), Type.String())"),
        "the record widens its key rather than dropping the check: {validators}"
    );
    let harness = r#"
import { validateKeyed } from "./validators.ts";

const admits = (label: string, value: unknown) => {
  const found = validateKeyed(value);
  if (found !== null) throw new Error(`${label} was refused: ${JSON.stringify(found)}`);
};
const refuses = (label: string, value: unknown) => {
  if (validateKeyed(value) === null) throw new Error(`${label} was admitted`);
};

admits("a key the contract would refuse", { table: { "not-a-uuid": "x" } });
refuses("a value of the wrong type", { table: { "not-a-uuid": 1 } });
"#;
    run_bun(&output.files, harness);
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
    let files = typebox(&document, TsOptions::default());
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
fn the_packaged_runtime_carries_the_hook_the_option_uses() {
    let document: Document = serde_json::from_value(fixture()).unwrap();
    let files = emitted(
        &document,
        TsOptions {
            validation: ValidationOptions::new(ValidationBackend::Typebox),
            errors: ErrorStrategy::Throw,
            runtime: Runtime::Package,
            ..TsOptions::default()
        },
    );
    assert!(
        !files
            .iter()
            .any(|file| file.path == "runtime.ts" || file.path.starts_with("runtime/")),
        "the package supplies the runtime, so the emitter writes none of it"
    );
    assert!(
        file(&files, "client.ts").contains("\"@pons/runtime\""),
        "the package is the runtime the client imports"
    );
    let validators = file(&files, "validators.ts");
    assert!(
        validators.contains(
            "import type { ValidationIssue, Validator } from \"@pons/runtime\";\nexport type { ValidationIssue, Validator } from \"@pons/runtime\";"
        ),
        "{validators}"
    );
}
