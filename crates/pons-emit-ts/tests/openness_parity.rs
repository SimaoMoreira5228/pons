use pons_ir::Document;
use serde_json::{Value, json};

fn string() -> Value {
    json!({ "kind": "string" })
}

fn count() -> Value {
    json!({ "kind": "int", "bits": 16, "signed": false, "wire": "number" })
}

fn count_schema() -> Value {
    json!({
        "type": "integer",
        "minimum": 0,
        "maximum": 65535,
        "x-pons-integer": {"bits": 16, "signed": false, "wire": "number"}
    })
}

fn map(value: Value) -> Value {
    json!({ "kind": "map", "key": string(), "value": value })
}

fn field(wire: &str, ty: Value, presence: &str) -> Value {
    json!({"wire": wire, "ident": [wire], "ty": ty, "presence": presence})
}

fn document(openness: Value, fields: Value) -> Document {
    let mut raw: Value =
        serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
    raw["types"]["Metadata"] = json!({
        "kind": "struct",
        "name": ["metadata"],
        "direction": "both",
        "openness": openness,
        "fields": fields,
    });
    serde_json::from_value(raw).unwrap()
}

fn agree(openness: Value, fields: Value, typescript: &str, additional: Value) {
    let document = document(openness, fields);
    let openapi: Value = serde_json::from_str(
        &pons_emit_openapi::emit(&document)
            .expect("the OpenAPI emitter must accept a map-shaped openness"),
    )
    .unwrap();
    let types = pons_emit_ts::emit(&document)
        .expect("the TypeScript emitter must agree")
        .into_iter()
        .find(|file| file.path == "types.ts")
        .unwrap()
        .content;
    assert!(
        types.contains(typescript),
        "expected {typescript} in\n{types}"
    );
    let schema = &openapi["components"]["schemas"]["Metadata"];
    assert_eq!(schema["additionalProperties"], additional);
}

fn rejects(name: &str, openness: Value) {
    let document = document(openness, json!([]));
    assert!(
        pons_emit_openapi::emit(&document).is_err(),
        "{name}: the OpenAPI emitter must reject"
    );
    assert!(
        pons_emit_ts::emit(&document).is_err(),
        "{name}: the TypeScript emitter must reject"
    );
}

#[test]
fn map_openness_types_extra_properties_with_the_map_value() {
    agree(
        json!({ "preserve_typed": { "value": map(count()) } }),
        json!([field("count", count(), "required")]),
        "export type Metadata = { \"count\": number; [key: string]: number };",
        count_schema(),
    );
    agree(
        json!({ "preserve_typed": { "value": map(count()) } }),
        json!([]),
        "export type Metadata = { [key: string]: number };",
        count_schema(),
    );
    agree(
        json!({ "preserve_typed": { "value": map(count()) } }),
        json!([field("count", count(), "optional")]),
        "export type Metadata = { \"count\"?: number; [key: string]: number | undefined };",
        count_schema(),
    );
}

#[test]
fn a_map_field_beside_a_map_shaped_openness_stays_typescript_unrepresentable() {
    let document = document(
        json!({ "preserve_typed": { "value": map(count()) } }),
        json!([field("tags", map(count()), "required")]),
    );
    let openapi: Value =
        serde_json::from_str(&pons_emit_openapi::emit(&document).unwrap()).unwrap();
    assert_eq!(
        openapi["components"]["schemas"]["Metadata"]["additionalProperties"],
        count_schema()
    );
    assert!(
        pons_emit_ts::emit(&document)
            .unwrap_err()
            .0
            .contains("typed object openness")
    );
}

#[test]
fn a_named_map_openness_behaves_like_the_inline_map() {
    let mut document = document(
        json!({ "preserve_typed": { "value": { "ref": "Counts" } } }),
        json!([field("count", count(), "required")]),
    );
    document.types.insert(
        "Counts".into(),
        serde_json::from_value(json!({
            "kind": "newtype",
            "name": ["counts"],
            "direction": "both",
            "inner": map(count()),
        }))
        .unwrap(),
    );
    let openapi: Value =
        serde_json::from_str(&pons_emit_openapi::emit(&document).unwrap()).unwrap();
    assert_eq!(
        openapi["components"]["schemas"]["Metadata"]["additionalProperties"],
        count_schema()
    );
    let types = pons_emit_ts::emit(&document)
        .unwrap()
        .into_iter()
        .find(|file| file.path == "types.ts")
        .unwrap()
        .content;
    assert!(
        types.contains("export type Metadata = { \"count\": number; [key: string]: number };"),
        "{types}"
    );
}

#[test]
fn a_flattened_typed_openness_unwraps_the_map_value_like_a_direct_one() {
    let mut raw: Value =
        serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
    raw["types"]["Extras"] = json!({
        "kind": "struct",
        "name": ["extras"],
        "direction": "both",
        "openness": {"preserve_typed": {"value": map(count())}},
        "fields": [field("count", count(), "required")],
    });
    raw["types"]["Metadata"] = json!({
        "kind": "struct",
        "name": ["metadata"],
        "direction": "both",
        "openness": {"preserve_typed": {"value": map(count())}},
        "fields": [],
        "flattened": [{"ref": "Extras"}],
    });
    let document: Document = serde_json::from_value(raw).unwrap();
    let openapi: Value =
        serde_json::from_str(&pons_emit_openapi::emit(&document).unwrap()).unwrap();
    let metadata = &openapi["components"]["schemas"]["Metadata"];
    assert_eq!(metadata["unevaluatedProperties"], count_schema());
    assert!(
        metadata["allOf"].as_array().unwrap()[0]
            .get("unevaluatedProperties")
            .is_none(),
        "the openness keyword must sit beside the allOf to see its annotations"
    );
    let types = pons_emit_ts::emit(&document)
        .unwrap()
        .into_iter()
        .find(|file| file.path == "types.ts")
        .unwrap()
        .content;
    assert!(
        types.contains("export type Metadata = { [key: string]: number } & Extras;"),
        "{types}"
    );
}

#[test]
fn a_flattened_map_is_rejected_by_both() {
    let mut raw: Value =
        serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
    raw["types"]["Metadata"] = json!({
        "kind": "struct",
        "name": ["metadata"],
        "direction": "both",
        "openness": {"preserve_typed": {"value": map(count())}},
        "fields": [],
        "flattened": [map(string())],
    });
    let document: Document = serde_json::from_value(raw).unwrap();
    assert!(pons_emit_openapi::emit(&document).unwrap_err().0.contains(
        "is not a struct, because a flattened member contributes its fields to one object"
    ));
    assert!(pons_emit_ts::emit(&document).unwrap_err().0.contains(
        "is not a struct, because a flattened member contributes its fields to one object"
    ));
}

#[test]
fn a_flattened_opaque_carrier_types_the_struct_as_its_codec_value() {
    let mut raw: Value =
        serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
    raw["types"]["Metadata"] = json!({
        "kind": "struct",
        "name": ["metadata"],
        "direction": "both",
        "openness": {"preserve_typed": {"value": map(count())}},
        "fields": [field("name", string(), "required")],
        "flattened": [{"kind": "opaque", "codec": "flatten.codec", "wire": "json"}],
    });
    let document: Document = serde_json::from_value(raw).unwrap();
    let types = pons_emit_ts::emit(&document)
        .unwrap()
        .into_iter()
        .find(|file| file.path == "types.ts")
        .unwrap()
        .content;
    assert!(types.contains("export type Metadata = unknown;"), "{types}");
}

#[test]
fn a_value_that_is_not_a_map_keeps_its_own_type() {
    agree(
        json!({ "preserve_typed": { "value": string() } }),
        json!([field("name", string(), "required")]),
        "export type Metadata = { \"name\": string; [key: string]: string };",
        json!({ "type": "string" }),
    );
}

#[test]
fn a_map_openness_with_non_string_keys_is_rejected_by_both() {
    rejects(
        "non-string map keys",
        json!({
            "preserve_typed": {
                "value": {"kind": "map", "key": count(), "value": count()}
            }
        }),
    );
}
