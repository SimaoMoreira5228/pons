mod support;

use std::collections::BTreeMap;

use pons_emit_ts::{
    DateMode, EmitMode, FieldCase, Int64, Newtypes, TsOptions, ValidationBackend,
    ValidationOptions, emit_with_options,
};
use pons_ir::Document;
use serde_json::json;
use support::runtime::{
    fixture, generated_file as file, kitchen_sink, lossy_options as zod_options,
    run_zod as run_bun, zod_files as zod,
};

#[test]
fn every_construct_the_contract_declares_reaches_a_schema() {
    let document = kitchen_sink();
    let files = zod(&document, zod_options());
    let validators = file(&files, "validators.ts");
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
        (
            "a uuid",
            "\"uuid\": z.string().regex(UUID, \"expected a uuid\")",
        ),
        ("a day", "\"day\": day()"),
        ("a clock", "\"clock\": clock()"),
        (
            "a duration",
            "\"span\": z.string().regex(DURATION, \"expected a duration\")",
        ),
        ("a uri", "\"link\": uri()"),
        (
            "an email",
            "\"who\": z.string().regex(EMAIL, \"expected an email\")",
        ),
        (
            "an ipv4",
            "\"v4\": z.string().regex(IPV4, \"expected an ipv4 address\")",
        ),
        (
            "an ipv6",
            "\"v6\": z.string().regex(IPV6, \"expected an ipv6 address\")",
        ),
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
            validators.contains(expected),
            "{construct} reaches a schema: {validators}"
        );
    }
    for (construct, expected) in [
        (
            "a string enum",
            "const ChoicesOneSchema = z.literal(\"one\");",
        ),
        (
            "an external enum",
            "const SpreadsBareSchema = z.union([z.literal(\"bare\"), oneKey(\"bare\", z.unknown())]);",
        ),
        (
            "an external payload",
            "const SpreadsHeldSchema = oneKey(\"held\", integer(z.int(), 32, false));",
        ),
        (
            "an internal enum",
            "const MarkedSomeSchema = z.object({\"kind\": z.literal(\"some\"), \"at\": integer(z.int(), 32, false)});",
        ),
        (
            "an adjacent enum",
            "const BesideSomeSchema = z.object({\"kind\": z.literal(\"some\"), \"payload\": z.boolean()});",
        ),
        (
            "an untagged enum",
            "const ShapesSchema = z.union([ShapesCountSchema, ShapesFullSchema, ShapesNamedSchema, ShapesNothingSchema]);",
        ),
        ("an untagged unit", "const ShapesNothingSchema = z.null();"),
        (
            "a newtype chain",
            "const AliasSchema = z.lazy(() => DeeperSchema);",
        ),
        (
            "a flattened member",
            "const WholeSchema = z.object({\"a\": z.string(), \"b\": z.boolean()}).strict();",
        ),
        (
            "an opaque flattened member",
            "const SealedOpaqueSchema = z.unknown();",
        ),
        (
            "rejected openness",
            "const SealedSchema = z.object({\"a\": z.string()}).strict();",
        ),
        (
            "typed openness",
            "const TypedSchema = z.object({\"a\": integer(z.int(), 32, false)}).catchall(integer(z.int(), 32, false));",
        ),
        (
            "opaque openness",
            "const LooseSchema = z.object({\"a\": z.string()});",
        ),
        ("a recursive type", "z.lazy(() => TreeSchema)"),
        (
            "a narrowed variant",
            "export const validateShapesCount: Validator = mismatch(ShapesCountSchema);",
        ),
    ] {
        assert!(
            validators.contains(expected)
                || file(&files, "operations/kitchen.ts").contains(expected),
            "{construct} reaches a schema: {validators}"
        );
    }
    let operations = file(&files, "operations/kitchen.ts");
    assert!(
        operations
            .contains("import { validateScalars, validateShapesCount } from \"../validators.js\";"),
        "the operation imports what it calls: {operations}"
    );
    assert!(
        operations.contains("validate: validateScalars"),
        "a declared response carries the check: {operations}"
    );
    assert!(
        operations.contains("validate: validateShapesCount"),
        "a narrowed declared error carries the narrowed check: {operations}"
    );
    for name in ["ScalarsSchema", "ShapesCountSchema"] {
        assert!(
            validators.contains(&format!("const {name} =")),
            "{name} is the private schema its validator wraps: {validators}"
        );
        assert!(
            !validators.contains(&format!("export const {name} =")),
            "{name} is not part of the public surface: {validators}"
        );
    }
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
    let schemas = |options: TsOptions| file(&zod(&document, options), "validators.ts").to_owned();
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
    let partial = file(&zod(&one_sided, TsOptions::default()), "validators.ts").to_owned();
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
fn the_direction_split_reaches_the_output_schema_the_validator_uses() {
    let document = kitchen_sink();
    let files = zod(&document, zod_options());
    let schemas = file(&files, "validators.ts");
    assert!(
        !schemas.contains("ReadingSchema") && !schemas.contains("validateReading"),
        "an input-only type needs no response schema or validator: {schemas}"
    );
    assert!(
        schemas.contains("const WrittenSchema = z.object({\"out\": z.string(), \"server\": z.string()}).strict();"),
        "an output type keeps the read_only field the server sends: {schemas}"
    );
    assert!(
        !schemas.contains("const WrittenSchema = z.object({\"out\": z.string()})"),
        "{schemas}"
    );
    assert!(
        schemas.contains(
            "const GatheredSchema = z.object({\"own\": z.string(), \"id\": z.string()}).strict();"
        ),
        "a member flattened from an output type answers to that type's direction: {schemas}"
    );
}

#[test]
fn a_decimal_bound_reaches_the_integer_call_as_an_integer() {
    let mut raw = fixture();
    raw["types"] = json!({
        "Bounded": {
            "kind": "struct", "name": ["bounded"], "direction": "output", "openness": "reject",
            "fields": [{
                "wire": "count",
                "ident": ["count"],
                "ty": {
                    "kind": "int",
                    "bits": 32,
                    "signed": true,
                    "wire": "number",
                    "minimum": 5.0,
                    "maximum": 1e3
                },
                "presence": "required"
            }]
        }
    });
    raw["operations"] = json!([{
        "id": [["bounded"], ["read"]],
        "method": "GET",
        "path": [{"lit": "bounded"}],
        "auth": {"alternatives": [], "optional": false},
        "semantics": {"safe": true, "idempotent": true},
        "responses": [{"status": 200, "content": "json", "body": {"ref": "Bounded"}}],
        "errors": []
    }]);
    let document: Document = serde_json::from_value(raw).unwrap();
    let files = zod(&document, TsOptions::default());
    let validators = file(&files, "validators.ts");
    assert!(
        validators.contains("\"count\": integer(z.int(), 32, true, \"5\", \"1000\")"),
        "a decimal bound is written as the integer it names: {validators}"
    );
    let harness = r#"
import { validateBounded } from "./validators.ts";

const admits = (label: string, value: unknown) => {
  const found = validateBounded(value);
  if (found !== null) throw new Error(`${label} was refused: ${JSON.stringify(found)}`);
};
const refuses = (label: string, value: unknown) => {
  if (validateBounded(value) === null) throw new Error(`${label} was admitted`);
};

admits("the minimum", { count: 5 });
admits("the maximum", { count: 1000 });
refuses("below the minimum", { count: 4 });
refuses("above the maximum", { count: 1001 });
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
            validation: ValidationOptions::new(ValidationBackend::Zod),
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
