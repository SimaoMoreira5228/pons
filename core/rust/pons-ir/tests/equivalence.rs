use std::collections::BTreeMap;

use pons_ir::{Document, equivalent, equivalent_ignoring_request_rejections};
use serde_json::{Map, Value, json};

fn exact() -> Value {
    serde_json::from_str(include_str!("../../../../spec/fixtures/exact.json"))
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

fn add_rejection_types(value: &mut Value) {
    value["types"]["Violation"] = json!({
        "kind": "struct",
        "openness": "reject",
        "name": ["violation"],
        "direction": "output",
        "fields": [
            {
                "wire": "path",
                "ident": ["path"],
                "ty": {"kind": "string"},
                "presence": "required"
            }
        ]
    });
    value["types"]["ViolationReport"] = json!({
        "kind": "struct",
        "openness": "reject",
        "name": ["violation", "report"],
        "direction": "output",
        "fields": [
            {
                "wire": "violations",
                "ident": ["violations"],
                "ty": {"kind": "array", "item": {"ref": "Violation"}},
                "presence": "required"
            }
        ]
    });
}

fn inject_request_rejection(value: &mut Value, body: Option<&str>) {
    for operation in value["operations"]
        .as_array_mut()
        .expect("the fixture has operations")
    {
        let operation = operation
            .as_object_mut()
            .expect("an operation is an object");
        let errors = operation
            .entry("errors")
            .or_insert_with(|| Value::Array(Vec::new()));
        let mut error = json!({
            "status": 422,
            "name": ["request", "rejected"],
            "content": if body.is_some() { "json" } else { "empty" }
        });
        if let Some(body) = body {
            error["body"] = json!({"ref": body});
        }
        errors
            .as_array_mut()
            .expect("errors is an array")
            .push(error);
    }
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

#[test]
fn injected_request_rejections_are_ignored_under_the_explicit_policy() {
    let without = document(exact());
    let mut value = exact();
    inject_request_rejection(&mut value, None);
    let with = document(value);

    equivalent_ignoring_request_rejections(&without, &with)
        .expect("the injected request rejection is ignored");
    equivalent_ignoring_request_rejections(&with, &without).expect("the policy is symmetric");

    let errors =
        equivalent(&without, &with).expect_err("the default comparison keeps the injected error");
    assert!(
        errors.iter().any(|error| error.path.contains("errors")),
        "{errors:?}"
    );
}

#[test]
fn a_422_error_with_another_name_is_not_a_request_rejection() {
    let left = document(exact());
    let mut value = exact();
    value["operations"][1]["errors"] = json!([
        {"status": 422, "name": ["request", "invalid"], "content": "empty"}
    ]);
    let errors = equivalent_ignoring_request_rejections(&left, &document(value))
        .expect_err("a differently named 422 is an application error");
    assert!(
        errors.iter().any(|error| error.path.contains("errors")),
        "{errors:?}"
    );
}

#[test]
fn request_rejection_body_types_are_ignored_with_their_errors() {
    let without = document(exact());
    let mut value = exact();
    add_rejection_types(&mut value);
    inject_request_rejection(&mut value, Some("ViolationReport"));
    let with = document(value);

    equivalent_ignoring_request_rejections(&without, &with)
        .expect("the rejection body types leave with the injected errors");
    equivalent(&without, &with)
        .expect_err("the default comparison keeps the injected errors and their types");
}

#[test]
fn a_differently_named_422_keeps_its_body_type_compared() {
    let mut value = exact();
    add_rejection_types(&mut value);
    value["operations"][1]["errors"] = json!([{
        "status": 422,
        "name": ["request", "invalid"],
        "content": "json",
        "body": {"ref": "ViolationReport"}
    }]);

    let errors = equivalent_ignoring_request_rejections(&document(value), &document(exact()))
        .expect_err("a differently named 422 is an application error");
    assert!(
        errors.iter().any(|error| error.path.contains("errors")),
        "{errors:?}"
    );
    assert!(
        errors.iter().any(|error| error.path.contains("types")),
        "{errors:?}"
    );
}

#[test]
fn a_type_shared_with_a_real_operation_stays_compared() {
    let mut left = exact();
    add_rejection_types(&mut left);
    inject_request_rejection(&mut left, Some("ViolationReport"));
    left["operations"][0]["responses"]
        .as_array_mut()
        .expect("responses is an array")
        .push(json!({"status": 201, "content": "json", "body": {"ref": "ViolationReport"}}));

    let mut right = exact();
    add_rejection_types(&mut right);
    right["operations"][0]["responses"]
        .as_array_mut()
        .expect("responses is an array")
        .push(json!({"status": 201, "content": "json", "body": {"ref": "ViolationReport"}}));
    right["types"]["Violation"]["fields"][0]["wire"] = json!("location");

    let errors = equivalent_ignoring_request_rejections(&document(left), &document(right))
        .expect_err("a type the rejection body shares with a real operation stays compared");
    assert!(
        errors.iter().any(|error| error.path.contains("types")),
        "{errors:?}"
    );
}

#[test]
fn an_unrelated_unused_type_stays_a_difference() {
    let mut left = exact();
    add_rejection_types(&mut left);
    inject_request_rejection(&mut left, Some("ViolationReport"));
    left["types"]["UnusedOnlyLeft"] = json!({
        "kind": "struct",
        "openness": "reject",
        "name": ["unused", "only", "left"],
        "direction": "output",
        "fields": []
    });

    let errors = equivalent_ignoring_request_rejections(&document(left), &document(exact()))
        .expect_err("an unrelated unused type is still compared");
    assert!(
        errors.iter().any(|error| error.path.contains("unused")),
        "{errors:?}"
    );
}

#[test]
fn an_unused_type_keeps_the_rejection_body_it_references() {
    let unused = json!({
        "kind": "struct",
        "openness": "reject",
        "name": ["unused"],
        "direction": "output",
        "fields": [
            {
                "wire": "report",
                "ident": ["report"],
                "ty": {"ref": "ViolationReport"},
                "presence": "required"
            }
        ]
    });

    let mut left = exact();
    add_rejection_types(&mut left);
    inject_request_rejection(&mut left, Some("ViolationReport"));
    left["types"]["Unused"] = unused.clone();

    let mut right = exact();
    add_rejection_types(&mut right);
    right["types"]["Unused"] = unused;

    equivalent_ignoring_request_rejections(&document(left), &document(right)).expect(
        "a retained unused type keeps the rejection body it references, leaving no dangling ref",
    );
}

#[test]
fn mutually_recursive_request_rejection_types_terminate() {
    let mut value = exact();
    value["types"]["RejectionNode"] = json!({
        "kind": "struct",
        "openness": "reject",
        "name": ["rejection", "node"],
        "direction": "output",
        "fields": [
            {
                "wire": "next",
                "ident": ["next"],
                "ty": {"kind": "nullable", "inner": {"ref": "RejectionNode"}},
                "presence": "optional"
            },
            {
                "wire": "detail",
                "ident": ["detail"],
                "ty": {"ref": "RejectionDetail"},
                "presence": "optional"
            }
        ]
    });
    value["types"]["RejectionDetail"] = json!({
        "kind": "struct",
        "openness": "reject",
        "name": ["rejection", "detail"],
        "direction": "output",
        "fields": [
            {
                "wire": "owner",
                "ident": ["owner"],
                "ty": {"kind": "nullable", "inner": {"ref": "RejectionNode"}},
                "presence": "optional"
            }
        ]
    });
    inject_request_rejection(&mut value, Some("RejectionNode"));

    equivalent_ignoring_request_rejections(&document(exact()), &document(value))
        .expect("a mutually recursive rejection body terminates and is ignored");
}

#[test]
fn a_422_success_response_is_not_a_request_rejection() {
    let left = document(exact());
    let mut value = exact();
    value["operations"][1]["responses"]
        .as_array_mut()
        .expect("responses is an array")
        .push(json!({"status": 422, "content": "empty"}));
    let errors = equivalent_ignoring_request_rejections(&left, &document(value))
        .expect_err("a 422 response is a success response, not an error");
    assert!(
        errors.iter().any(|error| error.path.contains("responses")),
        "{errors:?}"
    );
}

#[test]
fn the_exact_fixture_round_trip_stays_equivalent() {
    let original = document(exact());
    let canonical = original
        .to_canonical_json()
        .expect("the exact fixture canonicalizes");
    let round_trip = document(
        serde_json::from_str(&canonical).expect("the canonical fixture is an IR document"),
    );

    equivalent(&original, &round_trip).expect("the fixture round trip stays equivalent");
    equivalent_ignoring_request_rejections(&original, &round_trip)
        .expect("the fixture round trip stays equivalent under the explicit policy");
}

#[test]
fn pagination_descriptors_are_compared() {
    let mut value = exact();
    {
        let operation = value["operations"]
            .as_array_mut()
            .expect("operations are an array")
            .iter_mut()
            .find(|operation| operation.get("query").is_some())
            .expect("the fixture has a query operation");
        operation["semantics"]["pagination"] = json!({
            "request": {"kind": "cursor", "field": "limit"},
            "items": "/items",
            "next": "/next"
        });
    }
    let left = document(value.clone());
    equivalent(&left, &left).expect("a document is equivalent to itself");

    value["operations"]
        .as_array_mut()
        .expect("operations are an array")
        .iter_mut()
        .find(|operation| operation.get("query").is_some())
        .expect("the fixture has a query operation")["semantics"]["pagination"]["next"] =
        json!("/next_cursor");
    let right = document(value);
    let errors = equivalent(&left, &right).expect_err("the next locator differs");
    assert!(
        errors
            .iter()
            .any(|error| error.path.contains("semantics/pagination")),
        "{errors:?}"
    );
}
