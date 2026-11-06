use pons_ir::{Document, ValidationError};

#[test]
fn validation_finds_nested_unknown_types_and_bad_contract_links() {
    let json = r#"{
        "ir_version": "0.2",
        "info": {"title": "Example", "version": "1"},
        "types": {"Items": {"kind": "struct", "openness": "reject", "name": ["items"], "direction": "both", "fields": [
            {"wire": "items", "ident": ["items"], "ty": {"kind": "array", "item": {"ref": "Missing"}}, "presence": "required"}
        ]}},
        "operations": [{
            "id": [["items"], ["get"]], "method": "GET", "path": [],
            "responses": [{"status": 200, "content": "empty", "headers": [
                {"wire": "X-Missing", "ty": {"ref": "MissingHeader"}, "presence": "required"}
            ]}],
            "auth": {"alternatives": [["missing_scheme"]], "optional": false},
            "semantics": {"safe": true, "idempotent": true}
        }, {
            "id": [["items"], ["get"]], "method": "GET", "path": [],
            "auth": {"alternatives": [], "optional": false},
            "semantics": {"safe": true, "idempotent": true}
        }]
    }"#;

    let document: Document = serde_json::from_str(json).unwrap();
    let errors = document.validate().unwrap_err();
    assert!(errors.iter().any(|error| matches!(error, ValidationError::UnknownType { reference, .. } if reference == "Missing")));
    assert!(errors.iter().any(|error| matches!(error, ValidationError::UnknownType { reference, .. } if reference == "MissingHeader")));
    assert!(errors.iter().any(|error| matches!(error, ValidationError::UnknownAuthScheme { scheme, .. } if scheme == "missing_scheme")));
    assert!(
        errors
            .iter()
            .any(|error| matches!(error, ValidationError::DuplicateOperation { .. }))
    );
}
