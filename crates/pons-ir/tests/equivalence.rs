use std::collections::BTreeMap;

use pons_ir::{Document, equivalent};
use serde_json::{Map, Value, json};

fn exact() -> Value {
    serde_json::from_str(include_str!("../../../fixtures/exact.json"))
        .expect("the exact fixture is JSON")
}

fn document(value: Value) -> Document {
    serde_json::from_value(value).expect("the JSON is an IR document")
}

fn rename_ids(value: &mut Value, prefix: &str) {
    let types = value["types"].as_object().expect("the fixture has types");
    let mapping: BTreeMap<String, String> = types
        .keys()
        .map(|id| (id.clone(), format!("{prefix}{id}")))
        .collect();
    let mut renamed = Map::new();
    for (id, definition) in types {
        renamed.insert(mapping[id].clone(), definition.clone());
    }
    value["types"] = Value::Object(renamed);
    rewrite_refs(value, &mapping);
}

fn rewrite_refs(value: &mut Value, mapping: &BTreeMap<String, String>) {
    match value {
        Value::Object(object) => {
            if let Some(Value::String(id)) = object.get("ref")
                && let Some(renamed) = mapping.get(id)
            {
                object.insert("ref".into(), Value::String(renamed.clone()));
            }
            for child in object.values_mut() {
                rewrite_refs(child, mapping);
            }
        }
        Value::Array(items) => {
            for item in items {
                rewrite_refs(item, mapping);
            }
        }
        _ => {}
    }
}

#[test]
fn a_document_is_equivalent_to_itself() {
    let left = document(exact());
    let right = document(exact());
    equivalent(&left, &right).expect("the same fixture is equivalent");
}

#[test]
fn type_ids_are_opaque() {
    let left = document(exact());
    let mut value = exact();
    rename_ids(&mut value, "com.example.");
    let right = document(value);
    equivalent(&left, &right).expect("ids are compared by structure and display name");
}

#[test]
fn origin_and_doc_whitespace_are_ignored() {
    let mut value = exact();
    value["types"]["User"]["origin"] = json!("app::User");
    value["types"]["User"]["doc"] = json!({"summary": "  A   user "});
    value["operations"][0]["origin"] = json!({"source": "app::login"});
    value["operations"][0]["doc"] = json!({"summary": "Log in", "tags": [" auth "]});
    let right = document(value);

    let mut other = exact();
    other["types"]["User"]["doc"] = json!({"summary": "A user"});
    other["operations"][0]["doc"] = json!({"summary": "  Log in ", "tags": ["auth"]});
    equivalent(&right, &document(other)).expect("origin and doc whitespace are ignored");
}

#[test]
fn field_wire_changes_are_reported() {
    let left = document(exact());
    let mut value = exact();
    value["types"]["User"]["fields"][0]["wire"] = json!("identifier");
    let errors = equivalent(&left, &document(value)).expect_err("the wire name differs");
    assert!(
        errors.iter().any(|error| error.path.contains("types")),
        "{errors:?}"
    );
}

#[test]
fn extension_differences_are_reported() {
    let left = document(exact());
    let mut value = exact();
    value["ext"]["test.example.source/v1"] = json!("changed");
    let errors = equivalent(&left, &document(value)).expect_err("extensions matter");
    assert!(
        errors.iter().any(|error| error.path.contains("ext")),
        "{errors:?}"
    );
}

#[test]
fn ordering_does_not_create_a_difference() {
    let left = document(exact());
    let mut value = exact();
    value["operations"] = Value::Array(
        value["operations"]
            .as_array()
            .expect("operations are an array")
            .iter()
            .rev()
            .cloned()
            .collect(),
    );
    value["types"]["User"]["fields"] = Value::Array(
        value["types"]["User"]["fields"]
            .as_array()
            .expect("fields are an array")
            .iter()
            .rev()
            .cloned()
            .collect(),
    );
    equivalent(&left, &document(value)).expect("canonical ordering removes the difference");
}

#[test]
fn duplicate_display_names_are_refused() {
    let left = document(exact());
    let mut value = exact();
    let mut user = value["types"]["User"].clone();
    user["origin"] = json!("another::User");
    value["types"]["com.example.User"] = user;
    let errors = equivalent(&left, &document(value)).expect_err("the display name is ambiguous");
    assert!(
        errors
            .iter()
            .any(|error| error.message.contains("display name")),
        "{errors:?}"
    );
}

#[test]
fn references_to_unknown_types_are_refused() {
    let left = document(exact());
    let mut value = exact();
    value["types"]["User"]["fields"][0]["ty"] = json!({"ref": "Missing"});
    let errors = equivalent(&left, &document(value)).expect_err("the reference is broken");
    assert!(
        errors
            .iter()
            .any(|error| error.message.contains("does not define")),
        "{errors:?}"
    );
}
