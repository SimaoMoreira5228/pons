use pons_ir::Document;
use serde_json::Value;

#[test]
fn checked_in_schema_matches_the_rust_model() {
    let generated = serde_json::to_value(schemars::schema_for!(Document)).unwrap();
    let checked_in: Value =
        serde_json::from_str(include_str!("../../../../spec/ir.schema.json")).unwrap();
    assert_eq!(generated, checked_in);
}
