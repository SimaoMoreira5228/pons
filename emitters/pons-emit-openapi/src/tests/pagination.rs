use serde_json::{Value, json};

#[test]
fn pins_the_pagination_metadata_shape() {
    let output: Value =
        serde_json::from_str(&crate::emit(&super::paginated_fixture()).unwrap()).unwrap();
    assert_eq!(
        output["paths"]["/users"]["get"]["x-pons"]["semantics"]["pagination"],
        json!({
            "request": {"kind": "cursor", "field": "cursor"},
            "items": "/items",
            "next": "/next_cursor",
            "page_size": {"field": "limit", "default": 20, "maximum": 100}
        })
    );

    let output: Value = serde_json::from_str(&crate::emit(&super::fixture()).unwrap()).unwrap();
    assert!(
        output["paths"]["/auth/login"]["post"]["x-pons"]["semantics"]
            .get("pagination")
            .is_none()
    );
}
