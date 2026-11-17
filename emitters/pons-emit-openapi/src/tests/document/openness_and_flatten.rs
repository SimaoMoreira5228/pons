use pons_ir::{ObjectOpenness, TypeKind};
use serde_json::{Value, json};

use super::super::{fixture, string};

#[test]
fn emits_object_openness_policies() {
    let mut document = fixture();
    let user = document.types.get_mut("User").unwrap();
    let TypeKind::Struct {
        fields, flattened, ..
    } = &user.definition
    else {
        unreachable!()
    };
    user.definition = TypeKind::Struct {
        fields: fields.clone(),
        flattened: flattened.clone(),
        openness: ObjectOpenness::PreserveTyped { value: string() },
    };
    let login = document.types.get_mut("LoginRequest").unwrap();
    let TypeKind::Struct {
        fields, flattened, ..
    } = &login.definition
    else {
        unreachable!()
    };
    login.definition = TypeKind::Struct {
        fields: fields.clone(),
        flattened: flattened.clone(),
        openness: ObjectOpenness::PreserveOpaque,
    };
    let output: Value = serde_json::from_str(&crate::emit(&document).unwrap()).unwrap();
    assert_eq!(
        output["components"]["schemas"]["User"]["additionalProperties"]["type"],
        "string"
    );
    assert_eq!(
        output["components"]["schemas"]["LoginRequest"]["additionalProperties"],
        json!(true)
    );
    assert_eq!(
        output["components"]["schemas"]["EventQuery"]["additionalProperties"],
        json!(false)
    );
}

#[test]
fn rejects_nullable_flatten_before_emitting_a_contract() {
    let mut raw: serde_json::Value =
        serde_json::from_str(include_str!("../../../../../spec/fixtures/login.json")).unwrap();
    raw["types"]["LoginRequest"]["flattened"] = serde_json::json!([{
        "kind": "nullable",
        "inner": {"ref": "User"}
    }]);
    let document: pons_ir::Document = serde_json::from_value(raw).unwrap();
    for mode in [crate::EmitMode::Strict, crate::EmitMode::Lossy] {
        assert!(
            crate::emit_with_mode(&document, mode)
                .unwrap_err()
                .0
                .contains("nullable flattened")
        );
    }
}

#[test]
fn rejects_opaque_query_objects() {
    let mut document = fixture();
    let query = document.types.get_mut("EventQuery").unwrap();
    let TypeKind::Struct {
        fields, flattened, ..
    } = &query.definition
    else {
        unreachable!()
    };
    query.definition = TypeKind::Struct {
        fields: fields.clone(),
        flattened: flattened.clone(),
        openness: ObjectOpenness::PreserveOpaque,
    };
    assert!(crate::emit(&document).is_err());
}
