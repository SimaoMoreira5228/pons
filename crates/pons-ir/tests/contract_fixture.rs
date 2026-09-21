use pons_ir::Document;

#[test]
fn consumer_can_load_validate_and_canonicalize_the_contract_fixture() {
    let document: Document =
        serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
    document.validate().unwrap();

    let canonical = document.to_canonical_json().unwrap();
    let decoded: Document = serde_json::from_str(&canonical).unwrap();
    decoded.validate().unwrap();
    assert_eq!(document, decoded);
}

#[test]
fn diff_treats_header_names_case_insensitively() {
    let old: Document =
        serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
    let mut new = old.clone();
    let operation = new
        .operations
        .iter_mut()
        .find(|operation| operation.stream.is_some())
        .unwrap();
    operation.headers[0].wire = "x-request-id".into();
    operation.responses[0].headers[0].wire = "x-trace-id".into();

    assert!(old.diff(&new).changes.is_empty());
}
