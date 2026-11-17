use serde_json::{Value, json};

use super::super::fixture;

#[test]
fn emits_openapi_31_paths_schemas_security_and_pons_metadata() {
    let document = fixture();
    let encoded = crate::emit(&document).unwrap();
    assert!(encoded.ends_with('\n'));
    let output: Value = serde_json::from_str(&encoded).unwrap();

    assert_eq!(output["openapi"], "3.1.1");
    assert_eq!(output["info"]["title"], "Example");
    assert_eq!(output["servers"][0]["url"], "https://api.example.test");
    assert_eq!(
        output["paths"]["/auth/login"]["post"]["operationId"],
        "auth.login"
    );
    assert!(
        output["paths"]["/auth/login"]["post"]["security"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        output["paths"]["/auth/login"]["post"]["requestBody"]["content"]["application/json"]["schema"]
            ["$ref"],
        "#/components/schemas/LoginRequest"
    );
    assert_eq!(
        output["paths"]["/events/{cursor}"]["get"]["parameters"][0]["in"],
        "path"
    );
    assert_eq!(
        output["paths"]["/events/{cursor}"]["get"]["parameters"][1]["name"],
        "limit"
    );
    assert_eq!(
        output["paths"]["/events/{cursor}"]["get"]["responses"]["200"]["content"]["text/event-stream"]
            ["schema"]["x-pons-events"][0]["event"],
        "user"
    );
    assert_eq!(
        output["paths"]["/events/{cursor}"]["get"]["responses"]["200"]["headers"]["X-Trace-ID"]["required"],
        true
    );
    assert_eq!(
        output["paths"]["/events/{cursor}"]["get"]["x-pons"]["stream"]["kind"],
        "sse"
    );
    assert!(
        output["paths"]["/events/{cursor}"]["get"]["security"][0]["Tenant"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        output["paths"]["/auth/login"]["post"]["responses"]["4XX"]["content"]["application/json"]["schema"]
            ["properties"]["code"]["const"],
        "invalid"
    );
    assert_eq!(
        output["components"]["securitySchemes"]["Tenant"]["name"],
        "X-Tenant"
    );
    assert_eq!(
        output["components"]["schemas"]["User"]["properties"]["id"]["type"],
        "string"
    );
    assert_eq!(
        output["components"]["schemas"]["User"]["properties"]["id"]["x-pons"]["field"]["ident"][0],
        "id"
    );
    assert_eq!(
        output["components"]["schemas"]["User"]["x-pons"]["type"]["name"][0],
        "user"
    );
    assert_eq!(
        output["components"]["schemas"]["User"]["properties"]["nickname"]["anyOf"][1]["type"],
        "null"
    );
    let locked = output["components"]["schemas"]["LoginError"]["oneOf"]
        .as_array()
        .unwrap()
        .iter()
        .find(|schema| schema["properties"]["code"]["const"] == "locked")
        .unwrap();
    assert!(locked["properties"]["retry_after"]["format"].is_null());
    let email = &output["components"]["schemas"]["LoginRequest"]["properties"]["email"];
    assert_eq!(email["x-pons-format"], json!("email"));
    assert!(email.get("format").is_none(), "{email:#}");
    assert!(
        output["x-pons"]["ext"]["test.example.source/v1"]["kept"]
            .as_bool()
            .unwrap()
    );
}

#[test]
fn preserves_operation_id_punctuation() {
    let mut document = fixture();
    document.operations[0].id = vec![
        vec!["under_score".into()],
        vec!["dot.name".into()],
        vec!["percent%name".into()],
    ];
    let output: Value = serde_json::from_str(&crate::emit(&document).unwrap()).unwrap();
    assert_eq!(
        output["paths"]["/auth/login"]["post"]["operationId"],
        "under_score.dot.name.percent%name"
    );
}

#[test]
fn canonicalizes_equivalent_documents_before_emitting() {
    let mut document = fixture();
    let operation = document
        .operations
        .iter_mut()
        .find(|operation| operation.stream.is_some())
        .unwrap();
    operation.auth.alternatives = vec![vec!["session".into()], vec!["tenant".into()]];
    let expected = crate::emit(&document).unwrap();
    let mut shuffled = document.clone();
    shuffled.operations.reverse();
    shuffled
        .operations
        .iter_mut()
        .find(|operation| operation.stream.is_some())
        .unwrap()
        .auth
        .alternatives
        .reverse();

    assert_eq!(crate::emit(&shuffled).unwrap(), expected);
}
