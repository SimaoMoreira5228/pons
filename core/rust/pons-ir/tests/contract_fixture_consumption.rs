use pons_ir::{Document, ErrorStatus};

#[test]
fn consumer_can_load_validate_and_canonicalize_the_contract_fixture() {
    let document: Document =
        serde_json::from_str(include_str!("../../../../spec/fixtures/login.json")).unwrap();
    document.validate().unwrap();

    let canonical = document.to_canonical_json().unwrap();
    let decoded: Document = serde_json::from_str(&canonical).unwrap();
    decoded.validate().unwrap();
    assert_eq!(document, decoded);
}

#[test]
fn rich_document_round_trips_and_validates() {
    let document: Document =
        serde_json::from_str(include_str!("../../../../spec/fixtures/login.json")).unwrap();
    document.validate().unwrap();
    let encoded = serde_json::to_value(&document).unwrap();
    assert_eq!(encoded["ext"]["test.example.source/v1"]["kept"], true);
    assert_eq!(
        encoded["types"]["User"]["ext"]["test.example.source/v1"],
        "kept"
    );
    assert_eq!(encoded["auth_schemes"]["tenant"]["in"], "header");
    let decoded: Document = serde_json::from_value(encoded).unwrap();
    assert_eq!(document, decoded);
    assert_eq!(
        document.operations[0].errors[0].status,
        ErrorStatus::Class(4)
    );
}
