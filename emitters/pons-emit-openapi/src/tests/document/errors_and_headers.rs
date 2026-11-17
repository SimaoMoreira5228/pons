use pons_ir::{ErrorStatus, Presence, ResponseHeader, ResponseStatus};
use serde_json::Value;

use super::super::{fixture, string};

#[test]
fn emits_declared_error_response_headers() {
    let mut document = fixture();
    document.operations[0].errors[0]
        .headers
        .push(ResponseHeader {
            wire: "Retry-After".into(),
            ty: string(),
            presence: Presence::Required,
            ext: Default::default(),
        });
    let output: Value = serde_json::from_str(&crate::emit(&document).unwrap()).unwrap();
    let header =
        &output["paths"]["/auth/login"]["post"]["responses"]["4XX"]["headers"]["Retry-After"];
    assert_eq!(header["required"], true);
    assert_eq!(header["schema"]["type"], "string");
}

#[test]
fn emits_success_status_variants() {
    let mut document = fixture();
    let login = document
        .operations
        .iter()
        .position(|operation| operation.id[0][0] == "auth")
        .unwrap();
    document.operations[login].responses[0].status = ResponseStatus::Class(2);
    let output: Value = serde_json::from_str(&crate::emit(&document).unwrap()).unwrap();
    assert!(output["paths"]["/auth/login"]["post"]["responses"]["2XX"].is_object());
    document.operations[login].responses[0].status = ResponseStatus::Unknown;
    document.operations[login].errors[0].status = ErrorStatus::Exact(503);
    let output: Value = serde_json::from_str(&crate::emit(&document).unwrap()).unwrap();
    let responses = &output["paths"]["/auth/login"]["post"]["responses"];
    assert_eq!(responses["default"]["x-pons"]["status"], "unknown");
    assert!(responses.get("2XX").is_none());
    assert!(responses["503"].is_object());
    document.operations[login].responses[0].status = ResponseStatus::Default;
    let output: Value = serde_json::from_str(&crate::emit(&document).unwrap()).unwrap();
    let responses = &output["paths"]["/auth/login"]["post"]["responses"];
    assert_eq!(responses["default"]["x-pons"]["status"], "default");
    assert!(responses.get("2XX").is_none());
    assert!(responses["503"].is_object());
}

#[test]
fn rejects_default_success_and_error_dispatch_collision() {
    let mut document = fixture();
    document.operations[0].responses[0].status = ResponseStatus::Default;
    document.operations[0].errors[0].status = ErrorStatus::Default;
    assert!(
        crate::emit(&document)
            .unwrap_err()
            .0
            .contains("collides with a success response")
    );
}
