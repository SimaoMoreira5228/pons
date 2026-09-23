use pons_ir::Document;

type InsertUnknown = fn(&mut serde_json::Value);

#[test]
fn numeric_values_round_trip_without_precision_loss() {
    let source = r#"{
        "ir_version": "0.1",
        "info": {"title": "Example", "version": "1"},
        "types": {
            "Wide": {
                "kind": "struct",
                "name": ["wide"],
                "direction": "both",
                "fields": [{
                    "wire": "value",
                    "ident": ["value"],
                    "ty": {
                        "kind": "int",
                        "bits": 128,
                        "signed": false,
                        "wire": "number",
                        "minimum": 340282366920938463463374607431768211455,
                        "maximum": 340282366920938463463374607431768211455,
                        "multiple_of": 1.2300
                    },
                    "presence": "required",
                    "default": 123456789012345678901234567890.1200
                }]
            }
        }
    }"#;

    let document: Document = serde_json::from_str(source).unwrap();
    document.validate().unwrap();
    let encoded = serde_json::to_string(&document).unwrap();
    assert!(encoded.contains("340282366920938463463374607431768211455"));
    assert!(encoded.contains("123456789012345678901234567890.1200"));
    assert_eq!(document, serde_json::from_str(&encoded).unwrap());
}

#[test]
fn rejects_unknown_members_at_nested_paths() {
    let source: serde_json::Value =
        serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
    let cases: [(&str, InsertUnknown); 5] = [
        ("top", |value| {
            value
                .as_object_mut()
                .unwrap()
                .insert("future".into(), serde_json::json!(true));
        }),
        ("type", |value| {
            value["types"]["User"]
                .as_object_mut()
                .unwrap()
                .insert("future".into(), serde_json::json!(true));
        }),
        ("field", |value| {
            value["types"]["User"]["fields"][0]
                .as_object_mut()
                .unwrap()
                .insert("future".into(), serde_json::json!(true));
        }),
        ("stream", |value| {
            value["operations"][1]["stream"]
                .as_object_mut()
                .unwrap()
                .insert("future".into(), serde_json::json!(true));
        }),
        ("path", |value| {
            value["operations"][0]["path"][0]
                .as_object_mut()
                .unwrap()
                .insert("future".into(), serde_json::json!(true));
        }),
    ];
    for (name, insert) in cases {
        let mut value = source.clone();
        insert(&mut value);
        let error = serde_json::from_value::<Document>(value)
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("unknown field") || error.contains("did not match any variant"),
            "{name}: {error}"
        );
    }
}
