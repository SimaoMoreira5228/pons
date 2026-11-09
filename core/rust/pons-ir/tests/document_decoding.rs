use serde_json::{Value, json};

#[test]
fn decoding_refuses_a_version_it_does_not_read_before_reading_anything_else() {
    let mut raw: Value = serde_json::from_str(fixture_json()).unwrap();
    raw["ir_version"] = json!("0.4");
    raw["operations"][0]["method"] = json!("BREW");
    let error = pons_ir::decode(&serde_json::to_vec(&raw).unwrap()).unwrap_err();
    assert_eq!(
        error.to_string(),
        "unsupported IR version 0.4, this build reads 0.3"
    );

    let mut raw: Value = serde_json::from_str(fixture_json()).unwrap();
    raw.as_object_mut().unwrap().remove("ir_version");
    let error = pons_ir::decode(&serde_json::to_vec(&raw).unwrap()).unwrap_err();
    assert_eq!(
        error.to_string(),
        "ir_version: the document does not carry a string ir_version"
    );

    let error = pons_ir::decode(b"{\"ir_version\": ").unwrap_err();
    assert!(error.to_string().starts_with("invalid JSON: "), "{error}");
}

#[test]
fn decoding_names_the_member_it_stopped_on() {
    let mut raw: Value = serde_json::from_str(fixture_json()).unwrap();
    raw["operations"][0]["responses"][0]["status"] = json!("nope");
    let error = pons_ir::decode(&serde_json::to_vec(&raw).unwrap()).unwrap_err();
    assert_eq!(
        error.to_string(),
        "operations[0].responses[0].status: invalid HTTP status class nope"
    );

    let mut raw: Value = serde_json::from_str(fixture_json()).unwrap();
    raw["operations"][0]["path"][0]["l1t"] = json!("x");
    let error = pons_ir::decode(&serde_json::to_vec(&raw).unwrap()).unwrap_err();
    assert!(
        error
            .to_string()
            .starts_with("operations[0].path[0]: data did not match any variant"),
        "{error}"
    );

    let mut raw: Value = serde_json::from_str(fixture_json()).unwrap();
    raw["operations"][1]["headers"][0]["presence"] = json!("maybe");
    let error = pons_ir::decode(&serde_json::to_vec(&raw).unwrap()).unwrap_err();
    assert!(
        error
            .to_string()
            .starts_with("operations[1].headers[0].presence: unknown variant `maybe`"),
        "{error}"
    );

    let mut raw: Value = serde_json::from_str(fixture_json()).unwrap();
    raw["types"]["LoginError"]["repr"]["tag"] = json!(5);
    let error = pons_ir::decode(&serde_json::to_vec(&raw).unwrap()).unwrap_err();
    assert_eq!(
        error.to_string(),
        "types.LoginError: invalid type: integer `5`, expected a string"
    );
}

fn fixture_json() -> &'static str {
    include_str!("../../../../spec/fixtures/login.json")
}
