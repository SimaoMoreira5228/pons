#[test]
fn refuses_what_it_cannot_read() {
    let error = pons_openapi::import(b"[]").unwrap_err();
    assert!(error.to_string().contains("a JSON array"), "{error}");

    let error = pons_openapi::import(br#"{"openapi":"2.0"}"#).unwrap_err();
    assert!(error.to_string().contains("2.0"), "{error}");
    assert!(error.to_string().contains("3.0.x and 3.1.x"), "{error}");

    let error = pons_openapi::import(br#"{"info":{"title":"x","version":"1"}}"#).unwrap_err();
    assert!(error.to_string().contains("openapi"), "{error}");
}

#[test]
fn refuses_malformed_json_and_yaml_and_names_both_formats() {
    let error = pons_openapi::import(b"openapi: 3.1.0\ninfo: {title: x\n").unwrap_err();
    assert!(
        matches!(error, pons_openapi::ImportError::Read(_)),
        "{error:?}"
    );
    let message = error.to_string();
    assert!(
        message.contains("the input is not OpenAPI 3.0 or 3.1 JSON or YAML"),
        "{message}"
    );
    assert!(message.contains("JSON:"), "{message}");
    assert!(message.contains("YAML:"), "{message}");
}

#[test]
fn refuses_a_path_parameter_that_is_not_declared() {
    let raw = serde_json::json!({
        "openapi": "3.1.0",
        "info": {"title": "x", "version": "1"},
        "paths": {
            "/things/{id}": {
                "get": {
                    "responses": {"200": {"description": "ok"}}
                }
            }
        }
    });
    let error = pons_openapi::import(&serde_json::to_vec(&raw).unwrap()).unwrap_err();
    assert!(error.to_string().contains("not declared"), "{error}");
}
