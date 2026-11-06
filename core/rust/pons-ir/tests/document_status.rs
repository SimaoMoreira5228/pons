use pons_ir::{Document, ErrorStatus, ResponseStatus};

#[test]
fn response_status_variants_round_trip_and_validate() {
    let mut document: Document =
        serde_json::from_str(include_str!("../../../../spec/fixtures/login.json")).unwrap();
    document.operations[0].responses[0].status = ResponseStatus::Class(2);
    document.validate().unwrap();
    let encoded = serde_json::to_value(&document).unwrap();
    assert_eq!(encoded["operations"][0]["responses"][0]["status"], "2xx");
    document.operations[0].responses[0].status = ResponseStatus::Unknown;
    document.validate().unwrap();
    let encoded = serde_json::to_value(&document).unwrap();
    assert_eq!(
        encoded["operations"][0]["responses"][0]["status"],
        "unknown"
    );
    document.operations[0].responses[0].status = ResponseStatus::Default;
    document.validate().unwrap();
    let encoded = serde_json::to_value(&document).unwrap();
    assert_eq!(
        encoded["operations"][0]["responses"][0]["status"],
        "default"
    );
    document.operations[0].responses[0].status = ResponseStatus::Class(6);
    assert!(document.validate().is_err());
}

#[test]
fn status_defaults_and_classes_keep_their_wire_form() {
    let status: ErrorStatus = serde_json::from_str("\"default\"").unwrap();
    assert_eq!(serde_json::to_string(&status).unwrap(), "\"default\"");
    let status: ErrorStatus = serde_json::from_str("\"5xx\"").unwrap();
    assert_eq!(serde_json::to_string(&status).unwrap(), "\"5xx\"");
}
