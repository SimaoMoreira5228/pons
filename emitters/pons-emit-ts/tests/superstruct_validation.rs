mod support;

use std::{fs, path::Path, process::Command};

use pons_emit_ts::{
    EmitMode, ErrorStrategy, GeneratedFile, TsOptions, ValidationBackend, ValidationOptions,
    emit_with_options,
};
use pons_ir::Document;
use serde_json::{Value, json};

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

fn link(source: &Path, destination: &Path) {
    #[cfg(unix)]
    std::os::unix::fs::symlink(source, destination).unwrap();
    #[cfg(windows)]
    std::os::windows::fs::symlink_dir(source, destination).unwrap();
}

fn install_superstruct(directory: &Path) {
    let scope = directory.join("node_modules");
    fs::create_dir_all(&scope).unwrap();
    link(&support::package("superstruct"), &scope.join("superstruct"));
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
    let files = superstruct(&document, TsOptions::default());
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

fn response_document(types: Value, response: &str) -> Document {
    let mut raw = fixture();
    raw["types"] = types;
    raw["operations"] = json!([{
        "id": [["shape"], ["read"]],
        "method": "GET",
        "path": [{"lit": "shape"}],
        "auth": {"alternatives": [], "optional": false},
        "semantics": {"safe": true, "idempotent": true},
        "responses": [{"status": 200, "content": "json", "body": {"ref": response}}],
        "errors": []
    }]);
    serde_json::from_value(raw).unwrap()
}

#[test]
fn a_tuple_rejects_a_value_that_is_too_long_or_too_short() {
    let document = response_document(
        json!({
            "Pair": {
                "kind": "struct", "name": ["pair"], "direction": "output", "openness": "reject",
                "fields": [{
                    "wire": "pair",
                    "ident": ["pair"],
                    "ty": {"kind": "tuple", "items": [{"kind": "string"}, {"kind": "bool"}]},
                    "presence": "required"
                }]
            }
        }),
        "Pair",
    );
    let files = superstruct(&document, TsOptions::default());
    let validators = file(&files, "validators.ts");
    assert!(
        validators.contains(
            "refine(tuple([string(), boolean()]), \"length\", (value) => value.length === 2 || \"tuple has the wrong length\")"
        ),
        "the tuple keeps its exact length: {validators}"
    );
    let harness = r#"
import { validatePair } from "./validators.ts";

const admits = (label: string, value: unknown) => {
  if (validatePair(value) !== null) throw new Error(`${label} was refused`);
};
const refuses = (label: string, value: unknown) => {
  if (validatePair(value) === null) throw new Error(`${label} was admitted`);
};

admits("an exact tuple", { pair: ["x", true] });
refuses("a short tuple", { pair: ["x"] });
refuses("a long tuple", { pair: ["x", true, 1] });
"#;
    run_bun(&files, harness);
}

#[test]
fn an_external_enum_accepts_one_key_and_no_others() {
    let document = response_document(
        json!({
            "Only": {
                "kind": "enum", "name": ["only"], "direction": "output",
                "repr": {"kind": "external"},
                "variants": [{
                    "wire": "held",
                    "ident": ["held"],
                    "payload": {"kind": "struct", "fields": [{
                        "wire": "at",
                        "ident": ["at"],
                        "ty": {"kind": "int", "bits": 32, "signed": false, "wire": "number"},
                        "presence": "required"
                    }]}
                }]
            }
        }),
        "Only",
    );
    let files = superstruct(&document, TsOptions::default());
    let validators = file(&files, "validators.ts");
    assert!(
        validators.contains("const oneKey = (key: string, payload: Struct<any, any>) => object({ [key]: required(payload) });"),
        "the one-key wrapper is strict: {validators}"
    );
    let harness = r#"
import { validateOnly, validateOnlyHeld } from "./validators.ts";

const admits = (label: string, check: (value: unknown) => unknown, value: unknown) => {
  const found = check(value);
  if (found !== null) throw new Error(`${label} was refused: ${JSON.stringify(found)}`);
};
const refuses = (label: string, check: (value: unknown) => unknown, value: unknown) => {
  if (check(value) === null) throw new Error(`${label} was admitted`);
};

admits("the only external variant", validateOnly, { held: { at: 1 } });
admits("the narrowed external variant", validateOnlyHeld, { held: { at: 1 } });
refuses("an extra external key", validateOnly, { held: { at: 1 }, other: null });
refuses("two external keys", validateOnly, { held: { at: 1 }, bare: null });
refuses("an external payload of the wrong type", validateOnly, { held: { at: "1" } });
"#;
    run_bun(&files, harness);
}

#[test]
fn a_struct_error_path_names_the_nested_member_it_failed_on() {
    let document = response_document(
        json!({
            "Outer": {
                "kind": "struct", "name": ["outer"], "direction": "output", "openness": "reject",
                "fields": [{"wire": "inner", "ident": ["inner"], "ty": {"ref": "Inner"}, "presence": "required"}]
            },
            "Inner": {
                "kind": "struct", "name": ["inner"], "direction": "output", "openness": "reject",
                "fields": [{"wire": "items", "ident": ["items"], "ty": {"kind": "array", "item": {"ref": "Item"}}, "presence": "required"}]
            },
            "Item": {
                "kind": "struct", "name": ["item"], "direction": "output", "openness": "reject",
                "fields": [{"wire": "name", "ident": ["name"], "ty": {"kind": "string"}, "presence": "required"}]
            }
        }),
        "Outer",
    );
    let files = superstruct(&document, TsOptions::default());
    let harness = r#"
import { validateOuter } from "./validators.ts";

const found = validateOuter({ inner: { items: [{ name: 1 }] } });
if (found === null) throw new Error("a wrong nested member was admitted");
const path = (found as { path: unknown }).path;
if (JSON.stringify(path) !== JSON.stringify(["inner", "items", 0, "name"])) {
  throw new Error(`the issue path is not the nested member: ${JSON.stringify(path)}`);
}
"#;
    run_bun(&files, harness);
}

#[test]
fn a_required_field_whose_schema_accepts_undefined_is_still_required() {
    let document = response_document(
        json!({
            "Required": {
                "kind": "struct", "name": ["required"], "direction": "output", "openness": "reject",
                "fields": [
                    {"wire": "value", "ident": ["value"], "ty": {"kind": "any"}, "presence": "required"},
                    {"wire": "secret", "ident": ["secret"], "ty": {"kind": "opaque", "codec": "c", "wire": "json"}, "presence": "required"}
                ]
            }
        }),
        "Required",
    );
    let files = superstruct(&document, TsOptions::default());
    let validators = file(&files, "validators.ts");
    assert!(
        validators.contains("\"value\": required(any())")
            && validators.contains("\"secret\": required(unknown())"),
        "a required any or opaque value is non-optional: {validators}"
    );
    let harness = r#"
import { validateRequired } from "./validators.ts";

const admits = (label: string, value: unknown) => {
  if (validateRequired(value) !== null) throw new Error(`${label} was refused`);
};
const refuses = (label: string, value: unknown) => {
  if (validateRequired(value) === null) throw new Error(`${label} was admitted`);
};

admits("both required values", { value: 1, secret: { any: "json" } });
refuses("a missing any", { secret: {} });
refuses("a missing opaque", { value: 1 });
refuses("an undefined any", { value: undefined, secret: {} });
refuses("an undefined opaque", { value: 1, secret: undefined });
"#;
    run_bun(&files, harness);
}
