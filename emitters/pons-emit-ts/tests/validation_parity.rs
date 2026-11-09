mod support;

use std::{collections::BTreeMap, fs, path::Path, process::Command};

use pons_emit_ts::{
    DateMode, EmitMode, ErrorStrategy, GeneratedFile, Int64, Runtime, TsOptions, ValidationBackend,
    ValidationOptions, emit_with_options,
};
use pons_ir::Document;
use serde_json::json;

struct Case {
    label: &'static str,
    value: String,
    valid: bool,
}

fn document() -> Document {
    let raw = json!({
        "ir_version": "0.3",
        "info": {"title": "Parity", "version": "1"},
        "types": {
            "Parity": {
                "kind": "struct",
                "name": ["parity"],
                "direction": "output",
                "openness": "reject",
                "fields": [
                    {"wire": "flag", "ident": ["flag"], "ty": {"kind": "bool"}, "presence": "required"},
                    {"wire": "small", "ident": ["small"], "ty": {"kind": "int", "bits": 8, "signed": true, "wire": "number", "minimum": -5, "maximum": 5, "multiple_of": 5}, "presence": "required"},
                    {"wire": "count", "ident": ["count"], "ty": {"kind": "int", "bits": 32, "signed": true, "wire": "number"}, "presence": "required"},
                    {"wire": "wide", "ident": ["wide"], "ty": {"kind": "int", "bits": 64, "signed": true, "wire": "number"}, "presence": "required"},
                    {"wire": "ratio", "ident": ["ratio"], "ty": {"kind": "float", "bits": 64}, "presence": "required"},
                    {"wire": "money", "ident": ["money"], "ty": {"kind": "decimal", "wire": "string"}, "presence": "required"},
                    {"wire": "name", "ident": ["name"], "ty": {"kind": "string", "min_length": 2, "max_length": 4, "pattern": "^[a-z]+$"}, "presence": "required"},
                    {"wire": "id", "ident": ["id"], "ty": {"kind": "string", "format": "uuid"}, "presence": "required"},
                    {"wire": "day", "ident": ["day"], "ty": {"kind": "string", "format": "date"}, "presence": "required"},
                    {"wire": "clock", "ident": ["clock"], "ty": {"kind": "string", "format": "time"}, "presence": "required"},
                    {"wire": "span", "ident": ["span"], "ty": {"kind": "string", "format": "duration"}, "presence": "required"},
                    {"wire": "link", "ident": ["link"], "ty": {"kind": "string", "format": "uri"}, "presence": "required"},
                    {"wire": "who", "ident": ["who"], "ty": {"kind": "string", "format": "email"}, "presence": "required"},
                    {"wire": "v4", "ident": ["v4"], "ty": {"kind": "string", "format": "ipv4"}, "presence": "required"},
                    {"wire": "v6", "ident": ["v6"], "ty": {"kind": "string", "format": "ipv6"}, "presence": "required"},
                    {"wire": "moment", "ident": ["moment"], "ty": {"kind": "string", "format": "date-time"}, "presence": "required"},
                    {"wire": "blob", "ident": ["blob"], "ty": {"kind": "bytes", "encoding": "base64"}, "presence": "required"},
                    {"wire": "maybe", "ident": ["maybe"], "ty": {"kind": "nullable", "inner": {"kind": "string"}}, "presence": "required"},
                    {"wire": "unique", "ident": ["unique"], "ty": {"kind": "array", "item": {"kind": "string"}, "unique": true}, "presence": "required"},
                    {"wire": "pair", "ident": ["pair"], "ty": {"kind": "tuple", "items": [{"kind": "string"}, {"kind": "bool"}]}, "presence": "required"},
                    {"wire": "choice", "ident": ["choice"], "ty": {"ref": "Choice"}, "presence": "required"},
                    {"wire": "tree", "ident": ["tree"], "ty": {"ref": "Tree"}, "presence": "required"},
                    {"wire": "anything", "ident": ["anything"], "ty": {"kind": "any"}, "presence": "required"},
                    {"wire": "sealed", "ident": ["sealed"], "ty": {"kind": "opaque", "codec": "test.codec", "wire": "json"}, "presence": "required"}
                ],
                "flattened": [{"ref": "Part"}]
            },
            "Choice": {
                "kind": "enum",
                "name": ["choice"],
                "direction": "both",
                "repr": {"kind": "string"},
                "variants": [
                    {"wire": "alpha", "ident": ["alpha"], "payload": {"kind": "unit"}},
                    {"wire": "beta", "ident": ["beta"], "payload": {"kind": "unit"}}
                ]
            },
            "Part": {
                "kind": "struct",
                "name": ["part"],
                "direction": "both",
                "openness": "reject",
                "fields": [
                    {"wire": "part", "ident": ["part"], "ty": {"kind": "string"}, "presence": "required"}
                ]
            },
            "Tree": {
                "kind": "struct",
                "name": ["tree"],
                "direction": "both",
                "openness": "reject",
                "fields": [
                    {"wire": "value", "ident": ["value"], "ty": {"kind": "int", "bits": 32, "signed": true, "wire": "number"}, "presence": "required"},
                    {"wire": "next", "ident": ["next"], "ty": {"kind": "nullable", "inner": {"ref": "Tree"}}, "presence": "required"}
                ]
            }
        },
        "operations": [
            {
                "id": [["parity"], ["get"]],
                "method": "GET",
                "path": [{"lit": "parity"}],
                "auth": {"alternatives": [], "optional": false},
                "semantics": {"safe": true, "idempotent": true},
                "responses": [{"status": 200, "content": "json", "body": {"ref": "Parity"}}]
            }
        ]
    });
    serde_json::from_value(raw).unwrap()
}

fn base() -> BTreeMap<&'static str, String> {
    BTreeMap::from([
        ("flag", "true".into()),
        ("small", "-5".into()),
        ("count", "7".into()),
        ("wide", "9007199254740993n".into()),
        ("ratio", "1.5".into()),
        ("money", "\"12.34\"".into()),
        ("name", "\"abc\"".into()),
        ("id", "\"123e4567-e89b-12d3-a456-426614174000\"".into()),
        ("day", "\"2024-02-29\"".into()),
        ("clock", "\"12:34:56Z\"".into()),
        ("span", "\"P1DT2H\"".into()),
        ("link", "\"https://example.test/path\"".into()),
        ("who", "\"person@example.test\"".into()),
        ("v4", "\"192.168.0.1\"".into()),
        ("v6", "\"2001:db8::1\"".into()),
        ("moment", "\"2024-01-02T03:04:05.678Z\"".into()),
        ("blob", "new Uint8Array([1, 2, 3])".into()),
        ("maybe", "null".into()),
        ("unique", "[\"a\", \"b\"]".into()),
        ("pair", "[\"x\", true]".into()),
        ("choice", "\"alpha\"".into()),
        (
            "tree",
            "{ value: 1, next: { value: 2, next: null } }".into(),
        ),
        ("anything", "{ nested: [1, true] }".into()),
        ("sealed", "{ token: \"opaque\" }".into()),
        ("part", "\"joined\"".into()),
    ])
}

fn value(edit: impl FnOnce(&mut BTreeMap<&'static str, String>)) -> String {
    let mut fields = base();
    edit(&mut fields);
    let members = fields
        .iter()
        .map(|(key, value)| format!("{key:?}: {value}"))
        .collect::<Vec<_>>()
        .join(", ");
    format!("{{ {members} }}")
}

fn case(
    label: &'static str,
    valid: bool,
    edit: impl FnOnce(&mut BTreeMap<&'static str, String>),
) -> Case {
    Case {
        label,
        value: value(edit),
        valid,
    }
}

fn cases() -> Vec<Case> {
    vec![
        case("a fully valid value", true, |_| {}),
        case("a deeply recursive tree", true, |fields| {
            fields.insert(
                "tree",
                "{ value: 1, next: { value: 2, next: { value: 3, next: null } } }".into(),
            );
        }),
        case("a value whose bool is text", false, |fields| {
            fields.insert("flag", "\"true\"".into());
        }),
        case("a small integer below its minimum", false, |fields| {
            fields.insert("small", "-10".into());
        }),
        case("a small integer above its maximum", false, |fields| {
            fields.insert("small", "10".into());
        }),
        case("a small integer outside its multiple", false, |fields| {
            fields.insert("small", "3".into());
        }),
        case("a wide number under the bigint carrier", false, |fields| {
            fields.insert("wide", "9007199254740993".into());
        }),
        case("a float that is infinity", false, |fields| {
            fields.insert("ratio", "Infinity".into());
        }),
        case("a decimal on the number wire", false, |fields| {
            fields.insert("money", "12.34".into());
        }),
        case("a string shorter than its minimum", false, |fields| {
            fields.insert("name", "\"a\"".into());
        }),
        case("a string longer than its maximum", false, |fields| {
            fields.insert("name", "\"abcde\"".into());
        }),
        case("a string outside its pattern", false, |fields| {
            fields.insert("name", "\"ABC\"".into());
        }),
        case("an invalid uuid", false, |fields| {
            fields.insert("id", "\"not-a-uuid\"".into());
        }),
        case("an invalid date", false, |fields| {
            fields.insert("day", "\"2024-02-30\"".into());
        }),
        case("an invalid time", false, |fields| {
            fields.insert("clock", "\"25:00:00Z\"".into());
        }),
        case("an invalid duration", false, |fields| {
            fields.insert("span", "\"2 hours\"".into());
        }),
        case("an invalid uri", false, |fields| {
            fields.insert("link", "\"not a uri\"".into());
        }),
        case("an invalid email", false, |fields| {
            fields.insert("who", "\"not an email\"".into());
        }),
        case("an invalid ipv4", false, |fields| {
            fields.insert("v4", "\"256.0.0.1\"".into());
        }),
        case("an invalid ipv6", false, |fields| {
            fields.insert("v6", "\"2001:db8:::1\"".into());
        }),
        case("a calendar-invalid date-time", false, |fields| {
            fields.insert("moment", "\"2024-02-30T00:00:00Z\"".into());
        }),
        case("bytes outside the base64 carrier", false, |fields| {
            fields.insert("blob", "\"not base64!\"".into());
        }),
        case("a missing required field", false, |fields| {
            fields.remove("who");
        }),
        case("a missing required any", false, |fields| {
            fields.remove("anything");
        }),
        case("a missing required opaque", false, |fields| {
            fields.remove("sealed");
        }),
        case("an absent nullable field", false, |fields| {
            fields.remove("maybe");
        }),
        case("a duplicate array item", false, |fields| {
            fields.insert("unique", "[\"a\", \"a\"]".into());
        }),
        case("a tuple with the wrong length", false, |fields| {
            fields.insert("pair", "[\"x\"]".into());
        }),
        case("a tuple with the wrong item", false, |fields| {
            fields.insert("pair", "[\"x\", \"yes\"]".into());
        }),
        case("an invalid enum variant", false, |fields| {
            fields.insert("choice", "\"gamma\"".into());
        }),
        case("a missing flattened field", false, |fields| {
            fields.remove("part");
        }),
        case("a recursive tree with a wrong leaf", false, |fields| {
            fields.insert(
                "tree",
                "{ value: 1, next: { value: \"two\", next: null } }".into(),
            );
        }),
        case("an unknown property under reject", false, |fields| {
            fields.insert("extra", "true".into());
        }),
    ]
}

fn shapes() -> Document {
    let raw = json!({
        "ir_version": "0.3",
        "info": {"title": "Shapes", "version": "1"},
        "types": {
            "Forms": {
                "kind": "struct",
                "name": ["forms"],
                "direction": "output",
                "openness": "reject",
                "fields": [
                    {"wire": "spread", "ident": ["spread"], "ty": {"ref": "Spread"}, "presence": "required"},
                    {"wire": "marked", "ident": ["marked"], "ty": {"ref": "Marked"}, "presence": "required"},
                    {"wire": "beside", "ident": ["beside"], "ty": {"ref": "Beside"}, "presence": "required"},
                    {"wire": "shapes", "ident": ["shapes"], "ty": {"ref": "Shapes"}, "presence": "required"},
                    {"wire": "table", "ident": ["table"], "ty": {"kind": "map", "key": {"kind": "string"}, "value": {"kind": "string"}}, "presence": "required"},
                    {"wire": "label", "ident": ["label"], "ty": {"kind": "string", "min_length": 2, "max_length": 4}, "presence": "required"}
                ]
            },
            "Spread": {
                "kind": "enum", "name": ["spread"], "direction": "output",
                "repr": {"kind": "external"},
                "variants": [
                    {"wire": "bare", "ident": ["bare"], "payload": {"kind": "unit"}},
                    {"wire": "held", "ident": ["held"], "payload": {"kind": "newtype", "ty": {"kind": "int", "bits": 32, "signed": false, "wire": "number"}}}
                ]
            },
            "Marked": {
                "kind": "enum", "name": ["marked"], "direction": "output",
                "repr": {"kind": "internal", "tag": "kind"},
                "variants": [
                    {"wire": "none", "ident": ["none"], "payload": {"kind": "unit"}},
                    {"wire": "some", "ident": ["some"], "payload": {"kind": "struct", "fields": [
                        {"wire": "at", "ident": ["at"], "ty": {"kind": "int", "bits": 32, "signed": false, "wire": "number"}, "presence": "required"}
                    ]}}
                ]
            },
            "Beside": {
                "kind": "enum", "name": ["beside"], "direction": "output",
                "repr": {"kind": "adjacent", "tag": "kind", "content": "payload"},
                "variants": [
                    {"wire": "none", "ident": ["none"], "payload": {"kind": "unit"}},
                    {"wire": "some", "ident": ["some"], "payload": {"kind": "newtype", "ty": {"kind": "bool"}}}
                ]
            },
            "Shapes": {
                "kind": "enum", "name": ["shapes"], "direction": "output",
                "repr": {"kind": "untagged"},
                "variants": [
                    {"wire": "nothing", "ident": ["nothing"], "payload": {"kind": "unit"}},
                    {"wire": "count", "ident": ["count"], "payload": {"kind": "newtype", "ty": {"kind": "int", "bits": 32, "signed": false, "wire": "number"}}},
                    {"wire": "named", "ident": ["named"], "payload": {"kind": "struct", "fields": [
                        {"wire": "name", "ident": ["name"], "ty": {"kind": "string"}, "presence": "required"}
                    ]}}
                ]
            }
        },
        "operations": [
            {
                "id": [["shapes"], ["read"]],
                "method": "GET",
                "path": [{"lit": "shapes"}],
                "auth": {"alternatives": [], "optional": false},
                "semantics": {"safe": true, "idempotent": true},
                "responses": [{"status": 200, "content": "json", "body": {"ref": "Forms"}}]
            }
        ]
    });
    serde_json::from_value(raw).unwrap()
}

fn shape_value(edit: impl FnOnce(&mut BTreeMap<&'static str, String>)) -> String {
    let mut fields = BTreeMap::from([
        ("spread", "\"bare\"".into()),
        ("marked", "{ kind: \"none\" }".into()),
        ("beside", "{ kind: \"none\" }".into()),
        ("shapes", "null".into()),
        ("table", "{ a: \"x\" }".into()),
        ("label", "\"😀😀\"".into()),
    ]);
    edit(&mut fields);
    let members = fields
        .iter()
        .map(|(key, value)| format!("{key:?}: {value}"))
        .collect::<Vec<_>>()
        .join(", ");
    format!("{{ {members} }}")
}

fn shape_case(
    label: &'static str,
    valid: bool,
    edit: impl FnOnce(&mut BTreeMap<&'static str, String>),
) -> Case {
    Case {
        label,
        value: shape_value(edit),
        valid,
    }
}

fn shape_cases() -> Vec<Case> {
    vec![
        shape_case("every shape at once", true, |_| {}),
        shape_case("an external unit variant", true, |fields| {
            fields.insert("spread", "\"bare\"".into());
        }),
        shape_case("an external payload variant", true, |fields| {
            fields.insert("spread", "{ held: 3 }".into());
        }),
        shape_case("an internal payload variant", true, |fields| {
            fields.insert("marked", "{ kind: \"some\", at: 1 }".into());
        }),
        shape_case("an adjacent payload variant", true, |fields| {
            fields.insert("beside", "{ kind: \"some\", payload: true }".into());
        }),
        shape_case("an untagged struct", true, |fields| {
            fields.insert("shapes", "{ name: \"a\" }".into());
        }),
        shape_case("an unknown external variant", false, |fields| {
            fields.insert("spread", "\"ghost\"".into());
        }),
        shape_case("an external payload of the wrong type", false, |fields| {
            fields.insert("spread", "{ held: \"3\" }".into());
        }),
        shape_case("an internal variant without its tag", false, |fields| {
            fields.insert("marked", "{ at: 1 }".into());
        }),
        shape_case("an unknown internal variant", false, |fields| {
            fields.insert("marked", "{ kind: \"ghost\" }".into());
        }),
        shape_case("an adjacent payload that is missing", false, |fields| {
            fields.insert("beside", "{ kind: \"some\" }".into());
        }),
        shape_case("an adjacent payload of the wrong type", false, |fields| {
            fields.insert("beside", "{ kind: \"some\", payload: 1 }".into());
        }),
        shape_case("a value no untagged variant fits", false, |fields| {
            fields.insert("shapes", "\"text\"".into());
        }),
        shape_case("a map value of the wrong type", false, |fields| {
            fields.insert("table", "{ a: 1 }".into());
        }),
        shape_case("a short astral string", false, |fields| {
            fields.insert("label", "\"😀\"".into());
        }),
        shape_case("a long astral string", false, |fields| {
            fields.insert("label", "\"😀😀😀😀😀\"".into());
        }),
    ]
}

fn harness(cases: &[Case], response: &str) -> String {
    let entries = cases
        .iter()
        .map(|case| {
            format!(
                "  {{ label: {:?}, value: {}, valid: {} }},",
                case.label, case.value, case.valid
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let validator = format!("validate{response}");
    format!(
        r#"import {{ {validator} }} from "./validators.ts";
import {{ validateContract }} from "./runtime/index.ts";

type Case = {{ label: string; value: unknown; valid: boolean }};
const cases: Case[] = [
{entries}
];

for (const {{ label, value, valid }} of cases) {{
  const contract = validateContract(value, {{ ref: "{response}" }});
  const emitted = {validator}(value);
  const contractValid = contract === null;
  const emittedValid = emitted === null;
  if (contractValid !== valid) {{
    throw new Error(`${{label}}: the contract ${{contractValid ? "admits" : "refuses"}} a value the corpus marks ${{valid ? "valid" : "invalid"}}: ${{JSON.stringify(contract)}}`);
  }}
  if (emittedValid !== valid) {{
    throw new Error(`${{label}}: the backend ${{emittedValid ? "admits" : "refuses"}} a value the corpus marks ${{valid ? "valid" : "invalid"}}: ${{JSON.stringify(emitted)}}`);
  }}
  if (contractValid !== emittedValid) {{
    throw new Error(`${{label}}: the backend and the contract disagree: contract ${{JSON.stringify(contract)}}, backend ${{JSON.stringify(emitted)}}`);
  }}
}}
console.log(`${{cases.length}} parity cases passed`);
"#
    )
}

fn options(backend: ValidationBackend, mode: EmitMode) -> TsOptions {
    TsOptions {
        mode,
        errors: ErrorStrategy::Throw,
        runtime: Runtime::Package,
        int64: Int64::Bigint,
        date_mode: DateMode::String,
        validation: ValidationOptions::new(backend),
        ..TsOptions::default()
    }
}

fn emitted(
    document: &Document,
    backend: ValidationBackend,
    mode: EmitMode,
) -> pons_emit_ts::EmitOutput {
    emit_with_options(document, options(backend, mode))
        .unwrap_or_else(|error| panic!("{} must emit the parity document: {error}", backend.name()))
}

fn link(source: &Path, destination: &Path) {
    #[cfg(unix)]
    std::os::unix::fs::symlink(source, destination).unwrap();
    #[cfg(windows)]
    std::os::windows::fs::symlink_dir(source, destination).unwrap();
}

fn install_runtime_package(directory: &Path) {
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("runtime");
    let installed = directory.join("runtime");
    copy_runtime(&source, &installed);
    let scope = directory.join("node_modules").join("@pons");
    fs::create_dir_all(&scope).unwrap();
    link(&installed, &scope.join("runtime"));
}

fn copy_runtime(source: &Path, destination: &Path) {
    fs::create_dir_all(destination).unwrap();
    for entry in fs::read_dir(source).unwrap() {
        let entry = entry.unwrap();
        let target = destination.join(entry.file_name());
        if entry.path().is_dir() {
            copy_runtime(&entry.path(), &target);
        } else {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name.ends_with(".ts") || name == "package.json" {
                fs::copy(entry.path(), target).unwrap();
            }
        }
    }
}

fn install_packages(directory: &Path) {
    let scope = directory.join("node_modules");
    fs::create_dir_all(&scope).unwrap();
    for name in ["zod", "valibot", "arktype", "typebox", "yup", "superstruct"] {
        link(&support::package(name), &scope.join(name));
    }
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
    install_packages(directory.path());
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

fn check(backend: ValidationBackend) {
    let output = emitted(&document(), backend, EmitMode::Strict);
    assert!(
        output.validation_coverage.lossy.is_empty(),
        "{} classified the parity document as lossy: {:?}",
        backend.name(),
        output.validation_coverage.lossy
    );
    assert!(
        output.validation_coverage.unsupported.is_empty(),
        "{} classified the parity document as unsupported: {:?}",
        backend.name(),
        output.validation_coverage.unsupported
    );
    run_bun(&output.files, &harness(&cases(), "Parity"));
}

fn check_shapes(backend: ValidationBackend, expected_lossy: &[&str]) {
    let output = emitted(&shapes(), backend, EmitMode::Lossy);
    let lossy = output
        .validation_coverage
        .lossy
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>();
    assert_eq!(
        lossy,
        expected_lossy,
        "{} classified the shape corpus differently: {:?}",
        backend.name(),
        output.validation_coverage
    );
    assert!(
        output.validation_coverage.unsupported.is_empty(),
        "{} classified the shape corpus as unsupported: {:?}",
        backend.name(),
        output.validation_coverage.unsupported
    );
    run_bun(&output.files, &harness(&shape_cases(), "Forms"));
}

#[test]
fn builtin_agrees_with_the_contract() {
    check(ValidationBackend::Builtin);
}

#[test]
fn zod_agrees_with_the_contract() {
    check(ValidationBackend::Zod);
}

#[test]
fn valibot_agrees_with_the_contract() {
    check(ValidationBackend::Valibot);
}

#[test]
fn arktype_agrees_with_the_contract() {
    check(ValidationBackend::Arktype);
}

#[test]
fn typebox_agrees_with_the_contract() {
    check(ValidationBackend::Typebox);
}

#[test]
fn yup_agrees_with_the_contract() {
    check(ValidationBackend::Yup);
}

#[test]
fn superstruct_agrees_with_the_contract() {
    check(ValidationBackend::Superstruct);
}

#[test]
fn every_backend_that_keeps_the_enum_shapes_exact_agrees_on_them() {
    for backend in [
        ValidationBackend::Builtin,
        ValidationBackend::Zod,
        ValidationBackend::Arktype,
        ValidationBackend::Typebox,
        ValidationBackend::Superstruct,
    ] {
        check_shapes(backend, &[]);
    }
}

#[test]
fn valibot_keeps_the_enum_shapes_exact_and_widens_only_the_map() {
    check_shapes(
        ValidationBackend::Valibot,
        &["operations.shapes.read", "types.Forms"],
    );
}
