use serde_json::{Value, json};

fn document(types: Value) -> pons_ir::Document {
    let mut raw: Value =
        serde_json::from_str(include_str!("../../../../spec/fixtures/login.json")).unwrap();
    for (id, definition) in types.as_object().unwrap() {
        raw["types"][id] = definition.clone();
    }
    serde_json::from_value(raw).unwrap()
}

fn newtype(name: &str, inner: &str) -> Value {
    json!({"kind":"newtype","name":[name],"direction":"both","inner":{"ref":inner}})
}

fn newtype_over(name: &[&str], inner: Value) -> Value {
    json!({"kind":"newtype","name":name,"direction":"both","inner":inner})
}

fn cycle_error(types: Value) -> String {
    crate::emit(&document(types))
        .map(|_| String::from("the document was accepted"))
        .unwrap_err()
        .0
}

fn untagged_variant(wire: &str, payload: Value) -> Value {
    json!({"wire": wire, "ident": [wire], "payload": payload})
}

fn assert_cycle(error: &str, members: &[&str]) {
    for member in members {
        let expected =
            format!("the type {member} cannot be resolved because newtypes form a cycle");
        if error.contains(&expected) {
            return;
        }
    }
    panic!("{error}");
}

#[test]
fn rejects_a_self_referential_newtype() {
    let document = document(json!({"Loop": newtype("loop", "Loop")}));
    let error = crate::emit(&document).unwrap_err();
    assert!(
        error
            .0
            .contains("the type Loop cannot be resolved because newtypes form a cycle"),
        "{}",
        error.0
    );
}

#[test]
fn rejects_a_cycle_of_newtypes() {
    let document = document(json!({"A": newtype("a", "B"), "B": newtype("b", "A")}));
    let error = crate::emit(&document).unwrap_err();
    for member in ["A", "B"] {
        let expected =
            format!("the type {member} cannot be resolved because newtypes form a cycle");
        if error.0.contains(&expected) {
            return;
        }
    }
    panic!("{}", error.0);
}

#[test]
fn rejects_a_newtype_that_returns_to_itself_through_an_inline_arm() {
    assert_cycle(
        &cycle_error(json!({
            "Loop": newtype_over(&["loop"], json!({"kind": "nullable", "inner": {"ref": "Loop"}}))
        })),
        &["Loop"],
    );

    assert_cycle(
        &cycle_error(json!({
            "Loop": newtype_over(
                &["loop"],
                json!({"kind": "map", "key": {"kind": "string"}, "value": {"ref": "Loop"}})
            )
        })),
        &["Loop"],
    );
}

#[test]
fn rejects_two_newtypes_that_return_to_one_another_through_an_inline_arm() {
    assert_cycle(
        &cycle_error(json!({
            "A": newtype_over(&["a"], json!({"kind": "nullable", "inner": {"ref": "B"}})),
            "B": newtype_over(&["b"], json!({"kind": "nullable", "inner": {"ref": "A"}}))
        })),
        &["A", "B"],
    );
}

#[test]
fn rejects_a_newtype_that_reaches_itself_through_an_untagged_variant() {
    assert_cycle(
        &cycle_error(json!({
            "Loop": newtype_over(&["loop"], json!({"ref": "Kinds", "variant": "again"})),
            "Kinds": {
                "kind": "enum", "name": ["kinds"], "direction": "both", "repr": {"kind": "untagged"},
                "variants": [
                    untagged_variant("stop", json!({"kind": "unit"})),
                    untagged_variant(
                        "again",
                        json!({"kind": "newtype", "ty": {"kind": "nullable", "inner": {"ref": "Loop"}}})
                    )
                ]
            }
        })),
        &["Loop", "Kinds"],
    );
}

#[test]
fn rejects_a_cycle_that_closes_through_a_second_untagged_narrowing() {
    assert_cycle(
        &cycle_error(json!({
            "A": newtype_over(&["a"], json!({"ref": "Kinds", "variant": "left"})),
            "B": newtype_over(&["b"], json!({"ref": "Kinds", "variant": "right"})),
            "Kinds": {
                "kind": "enum", "name": ["kinds"], "direction": "both", "repr": {"kind": "untagged"},
                "variants": [
                    untagged_variant("left", json!({"kind": "newtype", "ty": {"ref": "B"}})),
                    untagged_variant("right", json!({"kind": "newtype", "ty": {"ref": "A"}}))
                ]
            }
        })),
        &["A", "B"],
    );
}

#[test]
fn rejects_a_cycle_that_closes_through_a_second_narrowing_in_the_other_order() {
    assert_cycle(
        &cycle_error(json!({
            "A": newtype_over(&["a"], json!({"ref": "Kinds", "variant": "right"})),
            "B": newtype_over(&["b"], json!({"ref": "Kinds", "variant": "left"})),
            "Kinds": {
                "kind": "enum", "name": ["kinds"], "direction": "both", "repr": {"kind": "untagged"},
                "variants": [
                    untagged_variant("left", json!({"kind": "newtype", "ty": {"ref": "A"}})),
                    untagged_variant("right", json!({"kind": "newtype", "ty": {"ref": "B"}}))
                ]
            }
        })),
        &["A", "B"],
    );
}

#[test]
fn a_narrowed_untagged_variant_that_names_itself_keeps_the_whole_enum() {
    let document = document(json!({
        "Tree": {
            "kind": "enum", "name": ["tree"], "direction": "both", "repr": {"kind": "untagged"},
            "variants": [
                {"wire": "leaf", "ident": ["leaf"], "payload": {"kind": "unit"}},
                {
                    "wire": "node",
                    "ident": ["node"],
                    "payload": {
                        "kind": "newtype",
                        "ty": {"kind": "array", "item": {"ref": "Tree", "variant": "node"}}
                    }
                }
            ]
        },
        "Root": newtype_over(&["root"], json!({"ref": "Tree", "variant": "node"}))
    }));
    let types = crate::emit(&document)
        .unwrap()
        .into_iter()
        .find(|file| file.path == "types.ts")
        .unwrap()
        .content;
    assert!(types.contains("export type Root = Array<Tree>;"), "{types}");
}

#[test]
fn emits_a_newtype_that_resolves_to_a_struct() {
    let document = document(json!({"Alias": newtype("alias", "User")}));
    let types = crate::emit(&document)
        .unwrap()
        .into_iter()
        .find(|file| file.path == "types.ts")
        .unwrap()
        .content;
    assert!(types.contains("export type Alias = User;"), "{types}");
}
