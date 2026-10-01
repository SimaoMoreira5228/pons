use pons_ir::Document;
use serde_json::{Value, json};

fn struct_type(name: &str, field: &str, flattened: Value) -> Value {
    json!({
        "kind": "struct",
        "name": [name],
        "direction": "both",
        "openness": "reject",
        "fields": [
            {"wire": field, "ident": [field], "ty": {"kind": "string"}, "presence": "required"}
        ],
        "flattened": flattened,
    })
}

fn document(types: Value) -> Document {
    let mut raw: Value =
        serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
    for (id, value) in types.as_object().unwrap() {
        raw["types"][id] = value.clone();
    }
    serde_json::from_value(raw).unwrap()
}

fn rejects(types: Value, message: &str) {
    let document = document(types);
    assert!(
        pons_emit_openapi::emit(&document).is_err(),
        "the OpenAPI emitter must reject the flattened member"
    );
    let error = pons_emit_ts::emit(&document).unwrap_err();
    assert!(error.0.contains(message), "{}", error.0);
}

#[test]
fn both_emitters_reject_a_flattened_enum() {
    rejects(
        json!({"Outer": struct_type("outer", "a", json!([{"ref": "LoginError"}]))}),
        "the type LoginError cannot be inlined into a struct",
    );
}

#[test]
fn both_emitters_reject_a_narrowed_flattened_variant() {
    rejects(
        json!({
            "Outer": struct_type("outer", "a", json!([{"ref": "LoginError", "variant": "invalid"}]))
        }),
        "the invalid variant of the type LoginError cannot be inlined into a struct",
    );
}

#[test]
fn both_emitters_reject_a_recursive_flatten() {
    let document = document(json!({
        "Outer": struct_type("outer", "a", json!([{"ref": "Inner"}])),
        "Inner": struct_type("inner", "b", json!([{"ref": "Outer"}])),
    }));
    assert!(pons_emit_openapi::emit(&document).is_err());
    let error = pons_emit_ts::emit(&document).unwrap_err();
    for member in ["Outer", "Inner"] {
        let expected =
            format!("the type {member} cannot be inlined because flattened members form a cycle");
        if error.0.contains(&expected) {
            return;
        }
    }
    panic!("{}", error.0);
}

#[test]
fn both_emitters_reject_a_flatten_cycle_through_a_newtype() {
    let document = document(json!({
        "Outer": struct_type("outer", "a", json!([{"ref": "Alias"}])),
        "Alias": {"kind": "newtype", "name": ["alias"], "direction": "both", "inner": {"ref": "Inner"}},
        "Inner": struct_type("inner", "b", json!([{"ref": "Alias"}])),
    }));
    assert!(pons_emit_openapi::emit(&document).is_err());
    let error = pons_emit_ts::emit(&document).unwrap_err();
    for member in ["Outer", "Alias", "Inner"] {
        let expected =
            format!("the type {member} cannot be inlined because flattened members form a cycle");
        if error.0.contains(&expected) {
            return;
        }
    }
    panic!("{}", error.0);
}

#[test]
fn both_emitters_accept_a_flattened_newtype_over_a_struct() {
    let document = document(json!({
        "Inner": struct_type("inner", "b", json!([])),
        "Alias": {"kind": "newtype", "name": ["alias"], "direction": "both", "inner": {"ref": "Inner"}},
        "Outer": struct_type("outer", "a", json!([{"ref": "Alias"}])),
    }));
    assert!(pons_emit_openapi::emit(&document).is_ok());
    let types = pons_emit_ts::emit(&document)
        .unwrap()
        .into_iter()
        .find(|file| file.path == "types.ts")
        .unwrap()
        .content;
    assert!(
        types.contains("export type Outer = { \"a\": string } & Alias;"),
        "{types}"
    );
}

#[test]
fn only_the_typescript_emitter_accepts_a_flattened_opaque_member() {
    let document = document(json!({
        "Outer": struct_type("outer", "a", json!([{"kind": "opaque", "codec": "flatten.codec", "wire": "json"}])),
    }));
    let error = pons_emit_openapi::emit(&document).unwrap_err();
    assert!(
        error
            .0
            .contains("an inline type cannot be inlined into a struct because only struct members are representable"),
        "{}",
        error.0
    );
    let types = pons_emit_ts::emit(&document)
        .unwrap()
        .into_iter()
        .find(|file| file.path == "types.ts")
        .unwrap()
        .content;
    assert!(
        types.contains("export type Outer = { \"a\": string } & unknown;"),
        "{types}"
    );
}
