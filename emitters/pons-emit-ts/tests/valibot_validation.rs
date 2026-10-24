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

fn link(source: &Path, destination: &Path) {
    #[cfg(unix)]
    std::os::unix::fs::symlink(source, destination).unwrap();
    #[cfg(windows)]
    std::os::windows::fs::symlink_dir(source, destination).unwrap();
}

fn install_valibot(directory: &Path) {
    let scope = directory.join("node_modules");
    fs::create_dir_all(&scope).unwrap();
    link(&support::package("valibot"), &scope.join("valibot"));
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
        "const LoginErrorInvalidSchema = plainRecord(v.looseObject({\"code\": v.literal(\"invalid\")}));",
        "const LoginErrorLockedSchema = plainRecord(v.looseObject({\"code\": v.literal(\"locked\"), \"retry_after\": integer(v.pipe(v.number(), v.integer()), 32, false)}));",
        "const LoginErrorSchema = v.variant(\"code\", [LoginErrorInvalidSchema, LoginErrorLockedSchema]);",
        "const UserSchema = plainRecord(v.looseObject({\"id\": integer(v.pipe(v.string(), v.regex(CANONICAL_INTEGER)), 64, false), \"nickname\": v.optional(v.nullable(v.string()))}));",
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
    let files = valibot(&document);
    let validators = file(&files, "validators.ts");
    assert!(
        validators.contains("v.strictTuple([v.string(), v.boolean()])"),
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
fn objects_reject_arrays_dates_typed_arrays_and_class_instances() {
    let document = response_document(
        json!({
            "Loose": {
                "kind": "struct", "name": ["loose"], "direction": "output",
                "openness": "preserve_opaque", "fields": []
            },
            "Sealed": {
                "kind": "struct", "name": ["sealed"], "direction": "output",
                "openness": "reject", "fields": []
            }
        }),
        "Loose",
    );
    let files = valibot(&document);
    let validators = file(&files, "validators.ts");
    assert!(
        validators.contains("plainRecord(v.looseObject({}))")
            && validators.contains("plainRecord(v.strictObject({}))"),
        "both object policies are guarded: {validators}"
    );
    let harness = r#"
import { validateLoose, validateSealed } from "./validators.ts";

class Box { a = 1; }

for (const [name, check] of [["loose", validateLoose], ["sealed", validateSealed]] as const) {
  if (check({}) !== null) throw new Error(`${name} refused a plain object`);
  if (check([]) === null) throw new Error(`${name} admitted an array`);
  if (check(new Date()) === null) throw new Error(`${name} admitted a date`);
  if (check(new Uint8Array([1])) === null) throw new Error(`${name} admitted a typed array`);
  if (check(new Box()) === null) throw new Error(`${name} admitted a class instance`);
}
"#;
    run_bun(&files, harness);
}

#[test]
fn a_single_variant_external_enum_stays_a_union() {
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
    let files = valibot(&document);
    let validators = file(&files, "validators.ts");
    assert!(
        validators.contains("const OnlySchema = v.union([OnlyHeldSchema]);"),
        "an external enum is a union, not a variant: {validators}"
    );
    assert!(
        validators.contains(
            "const OnlyHeldSchema = oneKey(\"held\", plainRecord(v.looseObject({\"at\": integer(v.pipe(v.number(), v.integer()), 32, false)})));"
        ),
        "the external wrapper keeps one key: {validators}"
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
refuses("an external payload of the wrong type", validateOnly, { held: { at: "1" } });
refuses("an external array", validateOnly, []);
"#;
    run_bun(&files, harness);
}

#[test]
fn a_map_is_lossy_because_a_record_skips_reserved_keys() {
    let document = response_document(
        json!({
            "Table": {
                "kind": "struct", "name": ["table"], "direction": "output", "openness": "reject",
                "fields": [{
                    "wire": "entries",
                    "ident": ["entries"],
                    "ty": {"kind": "map", "key": {"kind": "string"}, "value": {"kind": "string"}},
                    "presence": "required"
                }]
            }
        }),
        "Table",
    );
    let error = emit_with_options(
        &document,
        TsOptions {
            validation: ValidationOptions::new(ValidationBackend::Valibot),
            errors: ErrorStrategy::Throw,
            ..TsOptions::default()
        },
    )
    .expect_err("strict mode must refuse a lossy map");
    assert!(
        error.to_string().contains("types.Table") && error.to_string().contains("valibot"),
        "the refusal names the backend and the subject: {error}"
    );
    let output = emit_with_options(
        &document,
        TsOptions {
            mode: EmitMode::Lossy,
            validation: ValidationOptions::new(ValidationBackend::Valibot),
            errors: ErrorStrategy::Throw,
            ..TsOptions::default()
        },
    )
    .unwrap();
    assert!(
        output.validation_coverage.lossy.contains("types.Table"),
        "a map is lossy: {:?}",
        output.validation_coverage
    );
    assert!(
        !output.validation_coverage.exact.contains("types.Table"),
        "a map is not exact: {:?}",
        output.validation_coverage
    );
    let validators = file(&output.files, "validators.ts");
    assert!(
        validators.contains("plainRecord(v.record(v.string(), v.string()))"),
        "the lossy form still checks the entries it can: {validators}"
    );
    let harness = r#"
import { validateTable } from "./validators.ts";

const admits = (label: string, value: unknown) => {
  const found = validateTable(value);
  if (found !== null) throw new Error(`${label} was refused: ${JSON.stringify(found)}`);
};
const refuses = (label: string, value: unknown) => {
  if (validateTable(value) === null) throw new Error(`${label} was admitted`);
};

admits("a map of strings", { entries: { a: "x", b: "y" } });
refuses("a map value of the wrong type", { entries: { a: 1 } });
refuses("an array in place of a map", { entries: [] });
"#;
    run_bun(&output.files, harness);
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
    let files = valibot(&document);
    let validators = file(&files, "validators.ts");
    assert!(
        validators.contains("\"value\": v.nonOptional(v.any())")
            && validators.contains("\"secret\": v.nonOptional(v.unknown())"),
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
