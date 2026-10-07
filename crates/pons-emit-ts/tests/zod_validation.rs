use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use pons_emit_ts::{
    DateMode, EmitMode, ErrorStrategy, FieldCase, GeneratedFile, Int64, Newtypes, Runtime,
    TsOptions, Validation, emit_with_options,
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

fn zod(document: &Document, options: TsOptions) -> Vec<GeneratedFile> {
    emitted(
        document,
        TsOptions {
            validation: Validation::Zod,
            errors: ErrorStrategy::Throw,
            ..options
        },
    )
}

fn zod_options() -> TsOptions {
    TsOptions {
        mode: EmitMode::Lossy,
        ..TsOptions::default()
    }
}

fn workspace_zod() -> PathBuf {
    let mut directory = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    loop {
        let candidate = directory.join("node_modules").join("zod");
        if candidate.join("package.json").is_file() {
            return candidate;
        }
        if !directory.pop() {
            panic!(
                "no zod beside the workspace to install into a generated client, looked in {:?} and \
                 every directory above it; install it with `bun add --dev zod` or \
                 `npm install --save-dev zod`",
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

fn install_zod(directory: &Path) {
    let scope = directory.join("node_modules");
    fs::create_dir_all(&scope).unwrap();
    link(&workspace_zod(), &scope.join("zod"));
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

fn run_bun(files: &[GeneratedFile], harness: &str) {
    run_bun_with(files, harness, Runtime::Vendor)
}

fn run_bun_with(files: &[GeneratedFile], harness: &str, runtime: Runtime) {
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
    if runtime == Runtime::Package {
        install_runtime_package(directory.path());
    }
    install_zod(directory.path());
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

fn kitchen_sink() -> Document {
    let mut raw = fixture();
    raw["types"] = json!({
        "Scalars": {
            "kind": "struct", "name": ["scalars"], "direction": "both", "openness": "reject",
            "fields": [
                {"wire": "small", "ident": ["small"], "ty": {"kind": "int", "bits": 8, "signed": true, "wire": "number", "minimum": -5, "maximum": 5, "multiple_of": 5}, "presence": "required"},
                {"wire": "wide", "ident": ["wide"], "ty": {"kind": "int", "bits": 64, "signed": true, "wire": "number"}, "presence": "required"},
                {"wire": "text", "ident": ["text"], "ty": {"kind": "int", "bits": 32, "signed": false, "wire": "string"}, "presence": "required"},
                {"wire": "single", "ident": ["single"], "ty": {"kind": "float", "bits": 32}, "presence": "required"},
                {"wire": "double", "ident": ["double"], "ty": {"kind": "float", "bits": 64}, "presence": "required"},
                {"wire": "money", "ident": ["money"], "ty": {"kind": "decimal", "wire": "number"}, "presence": "required"},
                {"wire": "exact", "ident": ["exact"], "ty": {"kind": "decimal", "wire": "string"}, "presence": "required"},
                {"wire": "uuid", "ident": ["uuid"], "ty": {"kind": "string", "format": "uuid"}, "presence": "required"},
                {"wire": "day", "ident": ["day"], "ty": {"kind": "string", "format": "date"}, "presence": "required"},
                {"wire": "clock", "ident": ["clock"], "ty": {"kind": "string", "format": "time"}, "presence": "required"},
                {"wire": "span", "ident": ["span"], "ty": {"kind": "string", "format": "duration"}, "presence": "required"},
                {"wire": "link", "ident": ["link"], "ty": {"kind": "string", "format": "uri"}, "presence": "required"},
                {"wire": "who", "ident": ["who"], "ty": {"kind": "string", "format": "email"}, "presence": "required"},
                {"wire": "v4", "ident": ["v4"], "ty": {"kind": "string", "format": "ipv4"}, "presence": "required"},
                {"wire": "v6", "ident": ["v6"], "ty": {"kind": "string", "format": "ipv6"}, "presence": "required"},
                {"wire": "moment", "ident": ["moment"], "ty": {"kind": "string", "format": "date-time"}, "presence": "required"},
                {"wire": "sized", "ident": ["sized"], "ty": {"kind": "string", "min_length": 2, "max_length": 4}, "presence": "required"},
                {"wire": "shaped", "ident": ["shaped"], "ty": {"kind": "string", "pattern": "^[a-z]+$"}, "presence": "required"},
                {"wire": "blob", "ident": ["blob"], "ty": {"kind": "bytes", "encoding": "base64"}, "presence": "required"},
                {"wire": "nothing", "ident": ["nothing"], "ty": {"kind": "null"}, "presence": "required"},
                {"wire": "whatever", "ident": ["whatever"], "ty": {"kind": "any"}, "presence": "required"},
                {"wire": "secret", "ident": ["secret"], "ty": {"kind": "opaque", "codec": "c", "wire": "json"}, "presence": "required"},
                {"wire": "maybe", "ident": ["maybe"], "ty": {"kind": "nullable", "inner": {"kind": "int", "bits": 32, "signed": false, "wire": "number"}}, "presence": "required"},
                {"wire": "many", "ident": ["many"], "ty": {"kind": "array", "item": {"kind": "int", "bits": 32, "signed": false, "wire": "number"}, "unique": true}, "presence": "required"},
                {"wire": "table", "ident": ["table"], "ty": {"kind": "map", "key": {"kind": "string"}, "value": {"kind": "int", "bits": 32, "signed": false, "wire": "number"}}, "presence": "required"},
                {"wire": "pair", "ident": ["pair"], "ty": {"kind": "tuple", "items": [{"kind": "string"}, {"kind": "bool"}]}, "presence": "required"},
                {"wire": "maybe_missing", "ident": ["maybe", "missing"], "ty": {"kind": "string"}, "presence": "optional"}
            ]
        },
        "Choices": {
            "kind": "enum", "name": ["choices"], "direction": "both", "repr": {"kind": "string"},
            "variants": [
                {"wire": "one", "ident": ["one"], "payload": {"kind": "unit"}},
                {"wire": "two", "ident": ["two"], "payload": {"kind": "unit"}}
            ]
        },
        "Spreads": {
            "kind": "enum", "name": ["spreads"], "direction": "both", "repr": {"kind": "external"},
            "variants": [
                {"wire": "bare", "ident": ["bare"], "payload": {"kind": "unit"}},
                {"wire": "held", "ident": ["held"], "payload": {"kind": "newtype", "ty": {"kind": "int", "bits": 32, "signed": false, "wire": "number"}}}
            ]
        },
        "Marked": {
            "kind": "enum", "name": ["marked"], "direction": "both", "repr": {"kind": "internal", "tag": "kind"},
            "variants": [
                {"wire": "none", "ident": ["none"], "payload": {"kind": "unit"}},
                {"wire": "some", "ident": ["some"], "payload": {"kind": "struct", "fields": [
                    {"wire": "at", "ident": ["at"], "ty": {"kind": "int", "bits": 32, "signed": false, "wire": "number"}, "presence": "required"}
                ]}}
            ]
        },
        "Beside": {
            "kind": "enum", "name": ["beside"], "direction": "both", "repr": {"kind": "adjacent", "tag": "kind", "content": "payload"},
            "variants": [
                {"wire": "none", "ident": ["none"], "payload": {"kind": "unit"}},
                {"wire": "some", "ident": ["some"], "payload": {"kind": "newtype", "ty": {"kind": "bool"}}}
            ]
        },
        "Shapes": {
            "kind": "enum", "name": ["shapes"], "direction": "both", "repr": {"kind": "untagged"},
            "variants": [
                {"wire": "nothing", "ident": ["nothing"], "payload": {"kind": "unit"}},
                {"wire": "count", "ident": ["count"], "payload": {"kind": "newtype", "ty": {"kind": "int", "bits": 32, "signed": false, "wire": "number"}}},
                {"wire": "named", "ident": ["named"], "payload": {"kind": "struct", "fields": [
                    {"wire": "name", "ident": ["name"], "ty": {"kind": "string"}, "presence": "required"}
                ]}},
                {"wire": "full", "ident": ["full"], "payload": {"kind": "struct", "fields": [
                    {"wire": "name", "ident": ["name"], "ty": {"kind": "string"}, "presence": "required"},
                    {"wire": "extra", "ident": ["extra"], "ty": {"kind": "string"}, "presence": "required"}
                ]}}
            ]
        },
        "Loose": {
            "kind": "struct", "name": ["loose"], "direction": "both", "openness": "preserve_opaque",
            "fields": [{"wire": "a", "ident": ["a"], "ty": {"kind": "string"}, "presence": "required"}]
        },
        "Typed": {
            "kind": "struct", "name": ["typed"], "direction": "both",
            "openness": {"preserve_typed": {"value": {"kind": "int", "bits": 32, "signed": false, "wire": "number"}}},
            "fields": [{"wire": "a", "ident": ["a"], "ty": {"kind": "int", "bits": 32, "signed": false, "wire": "number"}, "presence": "required"}]
        },
        "Sealed": {
            "kind": "struct", "name": ["sealed"], "direction": "both", "openness": "reject",
            "fields": [{"wire": "a", "ident": ["a"], "ty": {"kind": "string"}, "presence": "required"}]
        },
        "Part": {
            "kind": "struct", "name": ["part"], "direction": "both", "openness": "reject",
            "fields": [{"wire": "b", "ident": ["b"], "ty": {"kind": "bool"}, "presence": "required"}]
        },
        "Whole": {
            "kind": "struct", "name": ["whole"], "direction": "both", "openness": "reject",
            "fields": [{"wire": "a", "ident": ["a"], "ty": {"kind": "string"}, "presence": "required"}],
            "flattened": [{"ref": "Part"}]
        },
        "Alias": {"kind": "newtype", "name": ["alias"], "direction": "both", "inner": {"ref": "Deeper"}},
        "Deeper": {"kind": "newtype", "name": ["deeper"], "direction": "both", "inner": {"kind": "int", "bits": 32, "signed": false, "wire": "number"}},
        "Reading": {
            "kind": "struct", "name": ["reading"], "direction": "input", "openness": "reject",
            "fields": [
                {"wire": "in", "ident": ["in"], "ty": {"kind": "string"}, "presence": "required"},
                {"wire": "secret", "ident": ["secret"], "ty": {"kind": "string"}, "presence": "required", "access": "write_only"}
            ]
        },
        "Written": {
            "kind": "struct", "name": ["written"], "direction": "output", "openness": "reject",
            "fields": [
                {"wire": "out", "ident": ["out"], "ty": {"kind": "string"}, "presence": "required"},
                {"wire": "server", "ident": ["server"], "ty": {"kind": "string"}, "presence": "required", "access": "read_only"}
            ]
        },
        "Tagged": {
            "kind": "struct", "name": ["tagged"], "direction": "output", "openness": "reject",
            "fields": [{"wire": "id", "ident": ["id"], "ty": {"kind": "string"}, "presence": "required", "access": "read_only"}]
        },
        "Gathered": {
            "kind": "struct", "name": ["gathered"], "direction": "both", "openness": "reject",
            "fields": [{"wire": "own", "ident": ["own"], "ty": {"kind": "string"}, "presence": "required"}],
            "flattened": [{"ref": "Tagged"}]
        },
        "Tree": {
            "kind": "struct", "name": ["tree"], "direction": "both", "openness": "reject",
            "fields": [
                {"wire": "value", "ident": ["value"], "ty": {"kind": "int", "bits": 32, "signed": false, "wire": "number"}, "presence": "required"},
                {"wire": "next", "ident": ["next"], "ty": {"kind": "nullable", "inner": {"ref": "Tree"}}, "presence": "required"}
            ]
        },
        "Opaque": {"kind": "newtype", "name": ["opaque"], "direction": "both", "inner": {"kind": "opaque", "codec": "c", "wire": "json"}},
        "SealedOpaque": {
            "kind": "struct", "name": ["sealed", "opaque"], "direction": "both", "openness": "reject",
            "fields": [{"wire": "a", "ident": ["a"], "ty": {"kind": "string"}, "presence": "required"}],
            "flattened": [{"ref": "Opaque"}]
        }
    });
    raw["operations"] = json!([{
        "id": [["kitchen"], ["sink"]],
        "method": "GET",
        "path": [{"lit": "kitchen"}],
        "auth": {"alternatives": [], "optional": false},
        "semantics": {"safe": true, "idempotent": true},
        "responses": [{"status": 200, "content": "json", "body": {"ref": "Scalars"}}],
        "errors": [{"status": 400, "name": ["kitchen"], "content": "json", "body": {"ref": "Shapes", "variant": "count"}}]
    }]);
    serde_json::from_value(raw).unwrap()
}

#[test]
fn every_construct_the_contract_declares_reaches_a_schema() {
    let document = kitchen_sink();
    let files = zod(&document, zod_options());
    let schemas = file(&files, "schemas.ts");
    let members: BTreeMap<&str, &str> = [
        (
            "integer widths and their constraints",
            "integer(z.int(), 8, true, \"-5\", \"5\", \"5\")",
        ),
        (
            "a wide integer on the number wire",
            "integer(z.number().int(), 64, true)",
        ),
        (
            "an integer on the string wire",
            "integer(z.string().regex(CANONICAL_INTEGER), 32, false)",
        ),
        ("a binary32", "binary32()"),
        ("a binary64", "\"double\": z.number()"),
        ("a decimal on the number wire", "\"money\": z.number()"),
        (
            "a decimal on the string wire",
            "\"exact\": z.string().regex(DECIMAL)",
        ),
        ("a uuid", "\"uuid\": z.uuid()"),
        ("a day", "\"day\": z.iso.date()"),
        ("a clock", "\"clock\": clock()"),
        ("a duration", "\"span\": z.iso.duration()"),
        ("a uri", "\"link\": z.url()"),
        ("an email", "\"who\": z.email()"),
        ("an ipv4", "\"v4\": z.ipv4()"),
        ("an ipv6", "\"v6\": z.ipv6()"),
        ("an instant", "\"moment\": instant()"),
        ("a minimum length", "[...value].length >= 2"),
        ("a maximum length", "[...value].length <= 4"),
        ("a pattern", "z.string().regex(new RegExp(\"^[a-z]+$\"))"),
        ("bytes", "\"blob\": z.instanceof(Uint8Array)"),
        ("null", "\"nothing\": z.null()"),
        ("any", "\"whatever\": required(z.any())"),
        ("opaque", "\"secret\": required(z.unknown())"),
        (
            "nullable",
            "\"maybe\": integer(z.int(), 32, false).nullable()",
        ),
        (
            "an optional field",
            "\"maybe_missing\": z.string().optional()",
        ),
        (
            "a unique array",
            "\"many\": uniqueItems(integer(z.int(), 32, false))",
        ),
        (
            "a map",
            "\"table\": z.record(z.string(), integer(z.int(), 32, false))",
        ),
        ("a tuple", "\"pair\": z.tuple([z.string(), z.boolean()])"),
    ]
    .into_iter()
    .collect();
    for (construct, expected) in members {
        assert!(
            schemas.contains(expected),
            "{construct} reaches a schema: {schemas}"
        );
    }
    for (construct, expected) in [
        (
            "a string enum",
            "export const ChoicesOneSchema = z.literal(\"one\");",
        ),
        (
            "an external enum",
            "export const SpreadsBareSchema = z.union([z.literal(\"bare\"), oneKey(\"bare\", z.unknown())]);",
        ),
        (
            "an external payload",
            "export const SpreadsHeldSchema = oneKey(\"held\", integer(z.int(), 32, false));",
        ),
        (
            "an internal enum",
            "export const MarkedSomeSchema = z.object({\"kind\": z.literal(\"some\"), \"at\": integer(z.int(), 32, false)});",
        ),
        (
            "an adjacent enum",
            "export const BesideSomeSchema = z.object({\"kind\": z.literal(\"some\"), \"payload\": z.boolean()});",
        ),
        (
            "an untagged enum",
            "export const ShapesSchema = z.union([ShapesCountSchema, ShapesFullSchema, ShapesNamedSchema, ShapesNothingSchema]);",
        ),
        (
            "an untagged unit",
            "export const ShapesNothingSchema = z.null();",
        ),
        (
            "a newtype chain",
            "export const AliasSchema = z.lazy(() => DeeperSchema);",
        ),
        (
            "a flattened member",
            "export const WholeSchema = z.object({\"a\": z.string(), \"b\": z.boolean()}).strict();",
        ),
        (
            "an opaque flattened member",
            "export const SealedOpaqueSchema = z.unknown();",
        ),
        (
            "rejected openness",
            "export const SealedSchema = z.object({\"a\": z.string()}).strict();",
        ),
        (
            "typed openness",
            "export const TypedSchema = z.object({\"a\": integer(z.int(), 32, false)}).catchall(integer(z.int(), 32, false));",
        ),
        (
            "opaque openness",
            "export const LooseSchema = z.object({\"a\": z.string()});",
        ),
        ("a recursive type", "z.lazy(() => TreeSchema)"),
        (
            "a narrowed variant",
            "validate: mismatch(ShapesCountSchema)",
        ),
    ] {
        assert!(
            schemas.contains(expected) || file(&files, "operations/kitchen.ts").contains(expected),
            "{construct} reaches a schema: {schemas}"
        );
    }
    let operations = file(&files, "operations/kitchen.ts");
    assert!(
        operations.contains("import { mismatch,")
            && operations.contains("} from \"../schemas.js\";"),
        "the operation imports what it calls: {operations}"
    );
    assert!(
        operations.contains("validate: mismatch(ScalarsSchema)"),
        "a declared response carries the check: {operations}"
    );
    assert!(
        operations.contains("validate: mismatch(ShapesCountSchema)"),
        "a narrowed declared error carries the narrowed check: {operations}"
    );
    for name in ["ScalarsSchema", "ShapesCountSchema"] {
        assert!(
            schemas.contains(&format!("export const {name} =")),
            "{name} is a schema the client imports: {schemas}"
        );
    }
}

#[test]
fn a_schema_admits_what_the_wire_validator_admits_and_refuses_the_rest() {
    let document = kitchen_sink();
    let files = zod(&document, zod_options());
    let harness = r#"
import {
  AliasSchema, BesideSchema, ChoicesSchema, GatheredSchema, LooseSchema, MarkedSchema, ScalarsSchema,
  SealedSchema, ShapesSchema, SpreadsSchema, TreeSchema, TypedSchema, WholeSchema, mismatch,
} from "./schemas.ts";

const admits = (label: string, check: (value: unknown) => object | null, value: unknown) => {
  const found = check(value);
  if (found !== null) throw new Error(`${label} was refused: ${JSON.stringify(found)}`);
};
const refuses = (label: string, check: (value: unknown) => object | null, value: unknown) => {
  if (check(value) === null) throw new Error(`${label} was admitted`);
};

admits("a string enum", mismatch(ChoicesSchema), "one");
refuses("an unknown string enum", mismatch(ChoicesSchema), "three");
refuses("a string enum over a number", mismatch(ChoicesSchema), 1);

admits("a bare external variant", mismatch(SpreadsSchema), "bare");
admits("a bare external variant as an object", mismatch(SpreadsSchema), { bare: null });
admits("a bare external variant carrying anything", mismatch(SpreadsSchema), { bare: 7 });
admits("an external payload", mismatch(SpreadsSchema), { held: 3 });
refuses("two external variants", mismatch(SpreadsSchema), { bare: null, held: 3 });
refuses("an unknown external variant", mismatch(SpreadsSchema), { ghost: null });
refuses("an external payload of the wrong type", mismatch(SpreadsSchema), { held: "3" });

admits("a unit internal variant", mismatch(MarkedSchema), { kind: "none" });
admits("an internal unit variant carrying an extra key", mismatch(MarkedSchema), { kind: "none", at: 1 });
admits("an internal payload", mismatch(MarkedSchema), { kind: "some", at: 1 });
refuses("an internal payload without its field", mismatch(MarkedSchema), { kind: "some" });
refuses("an unknown internal variant", mismatch(MarkedSchema), { kind: "ghost" });
refuses("an internal variant without its tag", mismatch(MarkedSchema), { at: 1 });
refuses("an internal payload out of range", mismatch(MarkedSchema), { kind: "some", at: 2 ** 33 });

admits("a unit adjacent variant", mismatch(BesideSchema), { kind: "none" });
admits("an adjacent unit variant carrying an extra key", mismatch(BesideSchema), { kind: "none", payload: 1 });
admits("an adjacent payload", mismatch(BesideSchema), { kind: "some", payload: true });
refuses("an adjacent payload that is missing", mismatch(BesideSchema), { kind: "some" });
refuses("an adjacent payload of the wrong type", mismatch(BesideSchema), { kind: "some", payload: 1 });
refuses("an unknown adjacent variant", mismatch(BesideSchema), { kind: "ghost", payload: true });

admits("an untagged unit", mismatch(ShapesSchema), null);
admits("an untagged newtype", mismatch(ShapesSchema), 3);
admits("an untagged struct", mismatch(ShapesSchema), { name: "a" });
admits("a value two untagged variants fit", mismatch(ShapesSchema), { name: "a", extra: "b" });
refuses("a value no untagged variant fits", mismatch(ShapesSchema), "a");
refuses("an untagged struct without its field", mismatch(ShapesSchema), {});

admits("a newtype chain", mismatch(AliasSchema), 3);
refuses("a newtype chain out of range", mismatch(AliasSchema), 2 ** 33);
refuses("a newtype chain over the wrong type", mismatch(AliasSchema), "3");

admits("a recursive value", mismatch(TreeSchema), { value: 1, next: { value: 2, next: null } });
refuses("a recursive value missing a field", mismatch(TreeSchema), { value: 1, next: {} });
refuses("a recursive value with an unknown key", mismatch(TreeSchema), { value: 1, next: null, other: 1 });

admits("a flattened member", mismatch(WholeSchema), { a: "x", b: true });
refuses("a flattened member that is missing", mismatch(WholeSchema), { a: "x" });
refuses("a flattened member of the wrong type", mismatch(WholeSchema), { a: "x", b: 1 });

admits("a read_only member of a flattened output type", mismatch(GatheredSchema), { own: "x", id: "y" });
refuses("a flattened output member that is missing", mismatch(GatheredSchema), { own: "x" });

admits("rejected openness", mismatch(SealedSchema), { a: "x" });
refuses("an unknown property", mismatch(SealedSchema), { a: "x", b: 1 });
admits("opaque openness", mismatch(LooseSchema), { a: "x", b: { anything: true } });
admits("typed openness", mismatch(TypedSchema), { a: 1, b: 2 });
refuses("a preserved property of the wrong type", mismatch(TypedSchema), { a: 1, b: "2" });
refuses("a preserved property the field does not allow", mismatch(TypedSchema), { a: "1" });

const scalars = {
  blob: new Uint8Array([65]), clock: "12:00:00Z", day: "2024-02-29", double: 1.5,
  exact: "1.50", link: "https://e.test/a", many: [1, 2], maybe: null, moment: "2024-01-01T00:00:00Z",
  money: 1.25, nothing: null, pair: ["a", true], secret: { any: 1 }, shaped: "abc", single: 1.5,
  sized: "abc", small: 5, span: "P1D", table: { a: 1 }, text: "42", uuid: "6ba7b810-9dad-11d1-80b4-00c04fd430c8",
  v4: "1.2.3.4", v6: "::1", whatever: 1, who: "a@b.test", wide: 5,
};
admits("every scalar", mismatch(ScalarsSchema), scalars);
const changed = (change: Record<string, unknown>) => ({ ...scalars, ...change });
for (const [label, change] of [
  ["an integer below its minimum", { small: -10 }],
  ["an integer above its maximum", { small: 10 }],
  ["an integer off its multiple", { small: 4 }],
  ["an integer above its width", { wide: 2 ** 64 }],
  ["a non-canonical integer string", { text: "007" }],
  ["an integer string that is not one", { text: "1.0" }],
  ["a number outside binary32", { single: 1e39 }],
  ["a decimal that is not one", { exact: "1.2.3" }],
  ["a day that is not one", { day: "2024-02-30" }],
  ["a clock with no offset", { clock: "12:00:00" }],
  ["a uuid that is not one", { uuid: "not-a-uuid" }],
  ["a uri that is not one", { link: "not a uri" }],
  ["an email that is not one", { who: "nope" }],
  ["an ipv4 that is not one", { v4: "999.1.1.1" }],
  ["an ipv6 that is not one", { v6: "::x" }],
  ["an instant with no offset", { moment: "2024-01-01T00:00:00" }],
  ["a month the calendar does not have", { moment: "2024-13-01T00:00:00Z" }],
  ["a string that is too short", { sized: "a" }],
  ["a string that is too long", { sized: "abcde" }],
  ["a string the pattern refuses", { shaped: "ABC" }],
  ["bytes that are not bytes", { blob: [65] }],
  ["a null that is not null", { nothing: 0 }],
  ["a nullable that is neither", { maybe: "x" }],
  ["an array with a repeated item", { many: [1, 1] }],
  ["an array item of the wrong type", { many: [1, "2"] }],
  ["a map value of the wrong type", { table: { a: "1" } }],
  ["a tuple of the wrong length", { pair: ["a"] }],
  ["a tuple item of the wrong type", { pair: ["a", 1] }],
  ["a required field that is absent", { sized: undefined }],
  ["a required opaque that is absent", { secret: undefined }],
  ["an unknown property", { ghost: 1 }],
] as Array<[string, Record<string, unknown>]>) {
  refuses(label, mismatch(ScalarsSchema), changed(change));
}
const sized = (value: string) => changed({ sized: value });
admits("a string of two astral code points", mismatch(ScalarsSchema), sized("😀😀"));
refuses("a string of one astral code point", mismatch(ScalarsSchema), sized("😀"));
"#;
    run_bun(&files, harness);
}

#[test]
fn the_carrier_options_reach_the_schema_they_change() {
    let mut raw = fixture();
    raw["types"]["Wide"] = json!({
        "kind": "struct", "name": ["wide"], "direction": "output", "openness": "reject",
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
        "responses": [{"status": 200, "content": "json", "body": {"ref": "Wide"}}],
        "errors": []
    }]);
    let document: Document = serde_json::from_value(raw).unwrap();
    let schemas = |options: TsOptions| file(&zod(&document, options), "schemas.ts").to_owned();
    let one_sided = {
        let mut raw = fixture();
        raw["types"]["Wide"] = json!({
            "kind": "struct", "name": ["wide"], "direction": "output", "openness": "reject",
            "fields": [
                {"wire": "ceiling", "ident": ["ceiling"], "ty": {"kind": "int", "bits": 16, "signed": false, "wire": "number", "maximum": 100}, "presence": "required"},
                {"wire": "step", "ident": ["step"], "ty": {"kind": "int", "bits": 16, "signed": false, "wire": "number", "multiple_of": 7}, "presence": "required"},
                {"wire": "floor", "ident": ["floor"], "ty": {"kind": "int", "bits": 16, "signed": false, "wire": "number", "minimum": 3}, "presence": "required"}
            ]
        });
        serde_json::from_value::<Document>(raw).unwrap()
    };
    let partial = file(&zod(&one_sided, TsOptions::default()), "schemas.ts").to_owned();
    for (construct, expected) in [
        (
            "a maximum on its own",
            "\"ceiling\": integer(z.int(), 16, false, undefined, \"100\")",
        ),
        (
            "a multiple on its own",
            "\"step\": integer(z.int(), 16, false, undefined, undefined, \"7\")",
        ),
        (
            "a minimum on its own",
            "\"floor\": integer(z.int(), 16, false, \"3\")",
        ),
    ] {
        assert!(
            partial.contains(expected),
            "{construct} is not dropped for want of the constraint before it: {partial}"
        );
    }

    let strict = schemas(TsOptions::default());
    assert!(
        strict.contains("\"id\": integer(z.bigint(), 64, true)"),
        "a wide integer is a bigint in strict mode: {strict}"
    );
    assert!(strict.contains("\"moment\": instant()"), "{strict}");

    let text = schemas(TsOptions {
        int64: Int64::String,
        ..TsOptions::default()
    });
    assert!(
        text.contains("\"id\": integer(z.string().regex(CANONICAL_SIGNED_INTEGER), 64, true)"),
        "int64 = string carries it as canonical text: {text}"
    );

    let dates = schemas(TsOptions {
        date_mode: DateMode::Date,
        ..TsOptions::default()
    });
    assert!(
        dates.contains("\"moment\": z.date()"),
        "date_mode = date reads an instant as a Date: {dates}"
    );

    let camel = schemas(TsOptions {
        field_case: FieldCase::Camel,
        ..TsOptions::default()
    });
    assert!(
        camel.contains("\"moment\": instant()"),
        "a camel key does not change a type: {camel}"
    );

    let branded = schemas(TsOptions {
        newtypes: Newtypes::Branded,
        ..TsOptions::default()
    });
    assert!(
        branded.contains("\"id\": integer(z.bigint(), 64, true)"),
        "a brand is erased on the wire, so the schema is the same: {branded}"
    );

    let lossy = schemas(TsOptions {
        mode: EmitMode::Lossy,
        int64: Int64::Number,
        ..TsOptions::default()
    });
    assert!(
        lossy.contains("\"id\": integer(z.number().int(), 64, true)"),
        "a number carrier is an integer the schema still bounds: {lossy}"
    );
}

#[test]
fn the_direction_split_reaches_the_schema_of_each_side() {
    let document = kitchen_sink();
    let files = zod(&document, zod_options());
    let schemas = file(&files, "schemas.ts");
    assert!(
        schemas.contains("export const ReadingSchema = z.object({\"in\": z.string(), \"secret\": z.string()}).strict();"),
        "an input type keeps the write_only field the caller sends: {schemas}"
    );
    assert!(
        schemas.contains("export const WrittenSchema = z.object({\"out\": z.string(), \"server\": z.string()}).strict();"),
        "an output type keeps the read_only field the server sends: {schemas}"
    );
    assert!(
        !schemas.contains("export const ReadingSchema = z.object({\"in\": z.string()})"),
        "{schemas}"
    );
    assert!(
        !schemas.contains("export const WrittenSchema = z.object({\"out\": z.string()})"),
        "{schemas}"
    );
    assert!(
        schemas.contains("export const GatheredSchema = z.object({\"own\": z.string(), \"id\": z.string()}).strict();"),
        "a member flattened from an output type answers to that type's direction: {schemas}"
    );
}

#[test]
fn the_four_byte_encodings_reach_one_decoded_schema_and_keep_their_spelling() {
    let mut raw = fixture();
    raw["types"]["User"]["fields"] = json!([
        {"wire": "array", "ident": ["array"], "ty": {"kind": "bytes", "encoding": "array"}, "presence": "required"},
        {"wire": "base64", "ident": ["base64"], "ty": {"kind": "bytes", "encoding": "base64"}, "presence": "required"},
        {"wire": "base64url", "ident": ["base64url"], "ty": {"kind": "bytes", "encoding": "base64url"}, "presence": "required"},
        {"wire": "hex", "ident": ["hex"], "ty": {"kind": "bytes", "encoding": "hex"}, "presence": "required"}
    ]);
    let document: Document = serde_json::from_value(raw).unwrap();
    let files = zod(&document, TsOptions::default());
    let schemas = file(&files, "schemas.ts");
    for encoding in ["array", "base64", "base64url", "hex"] {
        assert!(
            schemas.contains(&format!("\"{encoding}\": z.instanceof(Uint8Array)")),
            "{encoding} decodes to the bytes a schema checks: {schemas}"
        );
    }
    let harness = r#"
import { createClient } from "./client.ts";

const answering = (body: string) => createClient({
  baseUrl: "http://example.test",
  fetch: async () => new Response(body, { status: 200, headers: { "content-type": "application/json" } }),
});
const login = async (body: unknown) => {
  try {
    return { value: await answering(JSON.stringify(body)).auth.login({ body: { email: "person@example.test" } }) };
  } catch (failure) {
    return { failure };
  }
};
const body = { array: [65], base64: "QQ==", base64url: "QQ", hex: "41" };
const decoded: any = await login(body);
if (decoded.failure !== undefined) throw new Error(`four encodings were refused: ${JSON.stringify(decoded.failure)}`);
for (const encoding of ["array", "base64", "base64url", "hex"]) {
  const bytes = decoded.value[encoding];
  if (!(bytes instanceof Uint8Array) || bytes.length !== 1 || bytes[0] !== 65) {
    throw new Error(`${encoding} did not decode to its byte: ${bytes}`);
  }
}
for (const [label, broken] of [
  ["an array that is not bytes", { ...body, array: ["65"] }],
  ["base64 that is not base64", { ...body, base64: "!!!" }],
  ["base64url that is not base64url", { ...body, base64url: "???" }],
  ["hex that is not hex", { ...body, hex: "zz" }],
] as Array<[string, Record<string, unknown>]>) {
  const refused = await login(broken);
  if (refused.failure?.kind !== "protocol") {
    throw new Error(`${label} was admitted: ${JSON.stringify(refused)}`);
  }
  if (refused.failure.message.includes("the response does not match the contract")) {
    throw new Error(`${label} is the wire validator's to refuse: ${refused.failure.message}`);
  }
}
"#;
    run_bun(&files, harness);
}

#[test]
fn a_streamed_response_carries_no_check() {
    let document: Document = serde_json::from_value(fixture()).unwrap();
    let files = zod(&document, TsOptions::default());
    let events = file(&files, "operations/events.ts");
    assert!(
        !events.contains("mismatch") && !events.contains("validate"),
        "a stream is read item by item as it arrives, so there is no body to check: {events}"
    );
    let auth = file(&files, "operations/auth.ts");
    assert!(
        auth.contains("validate: mismatch(UserSchema)"),
        "the operation beside it does carry one: {auth}"
    );
}

#[test]
fn a_response_that_breaks_its_schema_is_the_failure_a_broken_body_already_is() {
    let document: Document = serde_json::from_value(fixture()).unwrap();
    {
        let files = zod(&document, TsOptions::default());
        let harness = r#"
import { createClient } from "./client.ts";

const answering = (body: string, status = 200) => createClient({
  baseUrl: "http://example.test",
  fetch: async () => new Response(body, { status, headers: { "content-type": "application/json" } }),
});
const login = async (client: any) => {
  try {
    return { value: await client.auth.login({ body: { email: "person@example.test" } }) };
  } catch (failure) {
    return { failure };
  }
};

const broken = await login(answering(JSON.stringify({ id: 1 })));
const malformed = await login(answering(JSON.stringify({})));
const ok = await login(answering(JSON.stringify({ id: "1" })));
if (ok.failure !== undefined) throw new Error(`a good body was refused: ${JSON.stringify(ok.failure)}`);
if (ok.value.id !== "1") throw new Error("the good body did not come back");

const shape = (answer: any) => answer.failure?.kind ?? "ok";
if (shape(broken) !== "protocol") throw new Error(`a schema breach is not a protocol failure: ${JSON.stringify(broken)}`);
if (shape(malformed) !== "protocol") throw new Error(`a malformed body is not a protocol failure: ${JSON.stringify(malformed)}`);
if (typeof (broken.failure ?? malformed.failure).message !== "string") {
  throw new Error(`a protocol failure names what it saw: ${JSON.stringify(broken)}`);
}
if (broken.failure.message === malformed.failure.message) {
  throw new Error(`the two failures say the same thing: ${broken.failure.message}`);
}
if (!malformed.failure.message.includes("missing required field id")) {
  throw new Error(`the wire validator answers first: ${malformed.failure.message}`);
}
if (broken.failure.message.includes("the response does not match the contract")) {
  throw new Error(`the wire validator already refused this body: ${broken.failure.message}`);
}

const declared = await login(answering(JSON.stringify({ code: "invalid" }), 418));
if (declared.failure.kind !== "http" || declared.failure.status !== 418) {
  throw new Error(`a declared error status is still an http failure: ${JSON.stringify(declared)}`);
}
if (declared.failure.body.code !== "invalid") {
  throw new Error(`the declared error body is checked too: ${JSON.stringify(declared)}`);
}
const unexpected = await login(answering(JSON.stringify({ id: "1" }), 500));
if (unexpected.failure.kind !== "unexpected_http" || unexpected.failure.status !== 500) {
  throw new Error(`a status no list declares is an unexpected_http: ${JSON.stringify(unexpected)}`);
}
"#;
        run_bun(&files, harness);
    }
    for errors in [ErrorStrategy::Neverthrow, ErrorStrategy::Throw] {
        let files = emitted(
            &document,
            TsOptions {
                validation: Validation::Zod,
                errors,
                ..TsOptions::default()
            },
        );
        let errors_file = file(&files, "errors.ts");
        let declared = errors_file
            .split("export type ClientFailure = ")
            .nth(1)
            .and_then(|rest| rest.split('\n').next())
            .unwrap();
        for kind in [
            "network",
            "timeout",
            "aborted",
            "auth_adapter",
            "auth_source",
            "opaque_codec",
            "interceptor",
            "protocol",
            "unexpected_http",
            "auth",
        ] {
            assert!(
                declared.contains(&format!("kind: \"{kind}\"")),
                "{kind} is still in the closed union under {errors:?}: {declared}"
            );
        }
        assert!(
            !declared.contains("schema"),
            "a schema breach joins the union as a protocol failure, not as a new kind: {declared}"
        );
    }
}

#[test]
fn the_schema_checks_the_formats_the_wire_validator_leaves_open() {
    let mut raw = fixture();
    raw["types"]["User"]["fields"] = json!([
        {"wire": "id", "ident": ["id"], "ty": {"kind": "string", "format": "uuid"}, "presence": "required"}
    ]);
    let document: Document = serde_json::from_value(raw).unwrap();
    let files = zod(&document, TsOptions::default());
    let harness = r#"
import { createClient } from "./client.ts";

const answering = (body: string) => createClient({
  baseUrl: "http://example.test",
  fetch: async () => new Response(body, { status: 200, headers: { "content-type": "application/json" } }),
});
const good = "6ba7b810-9dad-11d1-80b4-00c04fd430c8";
const accepted: any = await answering(JSON.stringify({ id: good })).auth.login({ body: { email: "person@example.test" } });
if (accepted.id !== good) throw new Error(`a uuid the contract allows was refused: ${JSON.stringify(accepted)}`);

let refused: any;
try {
  await answering(JSON.stringify({ id: "not-a-uuid" })).auth.login({ body: { email: "person@example.test" } });
} catch (cause) {
  refused = cause;
}
if (refused?.kind !== "protocol") throw new Error(`the schema did not answer: ${JSON.stringify(refused)}`);
if (!refused.message.includes("the response does not match the contract")) {
  throw new Error(`the failure names the schema: ${refused.message}`);
}
if (!refused.message.includes("id")) throw new Error(`the failure names the path: ${refused.message}`);
"#;
    run_bun(&files, harness);
}

#[test]
fn the_wire_validator_still_answers_a_request_it_refuses() {
    let mut raw = fixture();
    raw["types"]["LoginRequest"] = json!({
        "kind": "struct", "name": ["login", "request"], "direction": "input", "openness": "reject",
        "fields": [
            {"wire": "email", "ident": ["email"], "ty": {"kind": "string"}, "presence": "required"},
            {"wire": "id", "ident": ["id"], "ty": {"kind": "string"}, "presence": "required", "access": "read_only", "default": "filled"}
        ]
    });
    let document: Document = serde_json::from_value(raw).unwrap();
    let files = zod(&document, TsOptions::default());
    let harness = r#"
import { createClient } from "./client.ts";

const client = createClient({
  baseUrl: "http://example.test",
  fetch: async () => { throw new Error("the request should not be sent"); },
});
let failure: any;
try {
  await client.auth.login({ body: { email: "person@example.test", id: "1" } } as any);
} catch (cause) {
  failure = cause;
}
if (failure?.kind !== "protocol") throw new Error(`expected a protocol failure: ${JSON.stringify(failure)}`);
if (!failure.message.includes("is read_only and cannot cross the encode direction")) {
  throw new Error(`the wire validator answers the request: ${failure.message}`);
}
"#;
    run_bun(&files, harness);
}

#[test]
fn an_untagged_enum_takes_the_first_variant_that_fits() {
    let mut raw = fixture();
    raw["types"]["Shapes"] = json!({
        "kind": "enum", "name": ["shapes"], "direction": "output", "repr": {"kind": "untagged"},
        "variants": [
            {"wire": "count", "ident": ["count"], "payload": {"kind": "newtype", "ty": {"kind": "int", "bits": 32, "signed": false, "wire": "number"}}},
            {"wire": "named", "ident": ["named"], "payload": {"kind": "struct", "fields": [
                {"wire": "name", "ident": ["name"], "ty": {"kind": "string"}, "presence": "required"}
            ]}},
            {"wire": "full", "ident": ["full"], "payload": {"kind": "struct", "fields": [
                {"wire": "name", "ident": ["name"], "ty": {"kind": "string"}, "presence": "required"},
                {"wire": "extra", "ident": ["extra"], "ty": {"kind": "string"}, "presence": "required"}
            ]}},
            {"wire": "nothing", "ident": ["nothing"], "payload": {"kind": "unit"}}
        ]
    });
    raw["operations"][0]["responses"][0]["body"] = json!({"ref": "Shapes", "variant": "count"});
    let document: Document = serde_json::from_value(raw).unwrap();
    let files = zod(
        &document,
        TsOptions {
            mode: EmitMode::Lossy,
            ..TsOptions::default()
        },
    );
    let schemas = file(&files, "schemas.ts");
    assert!(
        schemas.contains("export const ShapesSchema = z.union([ShapesCountSchema, ShapesFullSchema, ShapesNamedSchema, ShapesNothingSchema]);"),
        "the union keeps the order the canonical document declares: {schemas}"
    );
    let harness = r#"
import { createClient } from "./client.ts";
import { mismatch, ShapesCountSchema, ShapesSchema } from "./schemas.ts";

const check = mismatch(ShapesSchema);
const admits = (label: string, value: unknown) => {
  const found = check(value);
  if (found !== null) throw new Error(`${label} was refused: ${JSON.stringify(found)}`);
};
const refuses = (label: string, value: unknown) => {
  if (check(value) === null) throw new Error(`${label} was admitted`);
};
admits("a count", 3);
admits("a struct", { name: "a" });
admits("a value two variants fit", { name: "a", extra: "b" });
admits("a unit", null);
refuses("a struct without its field", {});
refuses("a value no variant fits", "a");
refuses("a count out of range", 2 ** 33);
if (mismatch(ShapesCountSchema)(2 ** 33) === null) throw new Error("the narrowed schema bounds the count");

const client = createClient({
  baseUrl: "http://example.test",
  fetch: async () => new Response("3", { status: 200, headers: { "content-type": "application/json" } }),
});
const value: any = await client.auth.login({ body: { email: "person@example.test" } });
if (value !== 3) throw new Error(`the narrowed response is checked: ${JSON.stringify(value)}`);
"#;
    run_bun(&files, harness);
}

#[test]
fn a_string_enum_with_a_payload_is_refused_before_a_schema_is_written() {
    let mut raw = fixture();
    raw["types"]["Choices"] = json!({
        "kind": "enum", "name": ["choices"], "direction": "output", "repr": {"kind": "string"},
        "variants": [
            {"wire": "one", "ident": ["one"], "payload": {"kind": "unit"}},
            {"wire": "held", "ident": ["held"], "payload": {"kind": "newtype", "ty": {"kind": "int", "bits": 32, "signed": false, "wire": "number"}}}
        ]
    });
    raw["operations"] = json!([{
        "id": [["choices"], ["read"]],
        "method": "GET",
        "path": [{"lit": "choices"}],
        "auth": {"alternatives": [], "optional": false},
        "semantics": {"safe": true, "idempotent": true},
        "responses": [{"status": 200, "content": "json", "body": {"ref": "Choices"}}],
        "errors": []
    }]);
    let document: Document = serde_json::from_value(raw).unwrap();
    let refusal = emit_with_options(
        &document,
        TsOptions {
            validation: Validation::Zod,
            ..TsOptions::default()
        },
    )
    .unwrap_err();
    assert!(
        refusal.0.contains("Choices") && refusal.0.contains("unit payload"),
        "the refusal names the type and the rule: {}",
        refusal.0
    );
}

#[test]
fn the_packaged_runtime_carries_the_hook_the_option_uses() {
    let mut raw = fixture();
    raw["types"]["User"]["fields"] = json!([
        {"wire": "id", "ident": ["id"], "ty": {"kind": "string", "format": "uuid"}, "presence": "required"}
    ]);
    let document: Document = serde_json::from_value(raw).unwrap();
    let files = zod(
        &document,
        TsOptions {
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
    let harness = r#"
import { createClient } from "./client.ts";

const answering = (body: string) => createClient({
  baseUrl: "http://example.test",
  fetch: async () => new Response(body, { status: 200, headers: { "content-type": "application/json" } }),
});
const good = "6ba7b810-9dad-11d1-80b4-00c04fd430c8";
let accepted: any;
try {
  accepted = await answering(JSON.stringify({ id: good })).auth.login({ body: { email: "person@example.test" } });
} catch (cause) {
  throw new Error(`a good body was refused: ${JSON.stringify(cause)}`);
}
if (accepted.id !== good) throw new Error(`a good body did not come back: ${JSON.stringify(accepted)}`);
let refused: any;
try {
  await answering(JSON.stringify({ id: "not-a-uuid" })).auth.login({ body: { email: "person@example.test" } });
} catch (cause) {
  refused = cause;
}
if (refused?.kind !== "protocol") throw new Error(`the packaged runtime did not answer: ${JSON.stringify(refused)}`);
"#;
    run_bun_with(&files, harness, Runtime::Package);
}
