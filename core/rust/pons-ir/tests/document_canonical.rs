mod support;

use pons_ir::{Direction, validate_value};
use serde_json::{Value, json};
use support::{contract, field_type, refuses};

#[test]
fn canonicalize_makes_a_reorder_of_flattened_members_the_same_document() {
    let flattened_document = |members: [&str; 2]| {
        let document = contract(json!({
            "Left": {
                "kind": "struct", "name": ["left"], "direction": "both", "openness": "reject",
                "fields": [{"wire": "a", "ident": ["a"], "ty": {"kind": "string"}, "presence": "optional"}]
            },
            "Right": {
                "kind": "struct", "name": ["right"], "direction": "both", "openness": "reject",
                "fields": [{"wire": "b", "ident": ["b"], "ty": {"kind": "string"}, "presence": "optional"}]
            },
            "Holder": {
                "kind": "struct", "name": ["holder"], "direction": "both", "openness": "reject",
                "flattened": members.map(|id| json!({"ref": id}))
            }
        }));
        document.validate().unwrap();
        document
    };

    let mut forwards = flattened_document(["Left", "Right"]);
    let mut backwards = flattened_document(["Right", "Left"]);
    forwards.canonicalize();
    backwards.canonicalize();
    assert_eq!(
        serde_json::to_value(&forwards).unwrap(),
        serde_json::to_value(&backwards).unwrap()
    );
    assert!(forwards.diff(&backwards).changes.is_empty());
}

#[test]
fn an_integer_bound_spelled_with_an_exponent_is_the_integer_it_denotes() {
    let bounded = |name: &str, text: &str, bits: u16| {
        let bound = serde_json::from_str::<Value>(text).unwrap();
        contract(json!({
            "Count": {
                "kind": "struct", "name": ["count"], "direction": "both", "openness": "reject",
                "fields": [json!({"wire": "a", "ident": ["a"],
                    "ty": {"kind": "int", "bits": bits, "signed": true, "wire": "number", name: bound},
                    "presence": "optional"})]
            }
        }))
    };

    for name in ["minimum", "maximum", "multiple_of"] {
        for text in ["1e2", "1E2", "1.5e2", "1e3", "1.0", "1e0", "100"] {
            bounded(name, text, 32)
                .validate()
                .unwrap_or_else(|found| panic!("{name} {text} is an exact integer: {found:?}"));
        }
    }
    for text in ["-1e2", "0e-2", "0"] {
        for name in ["minimum", "maximum"] {
            bounded(name, text, 32)
                .validate()
                .unwrap_or_else(|found| panic!("{name} {text} is an exact integer: {found:?}"));
        }
        refuses(
            &bounded("multiple_of", text, 32),
            "integer multiple_of must be greater than zero",
        );
    }
    for text in ["1e-2", "1.5", "0.5e-1", "-1.5e-1"] {
        for name in ["minimum", "maximum", "multiple_of"] {
            refuses(
                &bounded(name, text, 32),
                &format!("integer {name} must be an integral value"),
            );
        }
    }
    for (name, text, bits, message) in [
        (
            "minimum",
            "1e400",
            128,
            "integer minimum is above its native range",
        ),
        (
            "maximum",
            "1e400",
            128,
            "integer maximum is above its native range",
        ),
        (
            "maximum",
            "-1e400",
            128,
            "integer maximum is below its native range",
        ),
        (
            "minimum",
            "1e39",
            128,
            "integer minimum is above its native range",
        ),
        (
            "multiple_of",
            "1e400",
            32,
            "integer multiple_of is too large to be an exact integer",
        ),
    ] {
        refuses(&bounded(name, text, bits), message);
    }
    refuses(
        &bounded("minimum", "1e39", 32),
        "integer minimum is above its native range",
    );
    bounded("maximum", "1e30", 128).validate().unwrap();
    bounded("multiple_of", "1e30", 128).validate().unwrap();
}

#[test]
fn a_bound_spelled_with_an_exponent_bounds_the_values_it_denotes() {
    let counted = |name: &str, text: &str| {
        let bound = serde_json::from_str::<Value>(text).unwrap();
        contract(json!({
            "Count": {
                "kind": "struct", "name": ["count"], "direction": "both", "openness": "reject",
                "fields": [json!({"wire": "a", "ident": ["a"],
                    "ty": {"kind": "int", "bits": 32, "signed": true, "wire": "number", name: bound},
                    "presence": "required"})]
            }
        }))
    };
    let counted_value = |name: &str, text: &str, found: &str| {
        let document = counted(name, text);
        document.validate().unwrap();
        validate_value(
            &document,
            field_type(&document, "Count", 0),
            Direction::Both,
            &serde_json::from_str::<Value>(found).unwrap(),
        )
    };

    for (name, found, accepted) in [
        ("minimum", "1000", true),
        ("minimum", "1e3", true),
        ("minimum", "999", false),
        ("maximum", "1000", true),
        ("maximum", "1e3", true),
        ("maximum", "1001", false),
    ] {
        assert_eq!(
            counted_value(name, "1e3", found).is_ok(),
            accepted,
            "{name} 1e3 against {found}"
        );
    }
    assert!(counted_value("multiple_of", "1e2", "300").is_ok());
    assert!(counted_value("multiple_of", "1e2", "301").is_err());
}

#[test]
fn an_alias_list_is_checked_and_sorted_into_one_canonical_form() {
    let record = |fields: Value| {
        contract(json!({
            "Record": {
                "kind": "struct", "name": ["record"], "direction": "both", "openness": "reject",
                "fields": fields
            }
        }))
    };
    let keyed = |wire: &str, aliases: Value| {
        json!({
            "wire": wire, "ident": [wire], "ty": {"kind": "string"}, "presence": "optional",
            "ext": {"com.pons.alias/v1": aliases}
        })
    };
    let plain = json!({
        "wire": "nickname", "ident": ["nickname"], "ty": {"kind": "string"}, "presence": "optional"
    });

    record(json!([
        keyed("user_name", json!(["userName", "username"])),
        plain.clone()
    ]))
    .validate()
    .unwrap();

    for (aliases, message) in [
        (
            json!("userName"),
            "must be an array of accepted input wire names",
        ),
        (json!([["userName"]]), "must be a non-empty string"),
        (json!([7]), "must be a non-empty string"),
        (json!([""]), "must be a non-empty string"),
        (json!(["userName", "userName"]), "repeat userName"),
        (
            json!(["user_name"]),
            "cannot repeat its own wire name user_name",
        ),
        (json!(["nickname"]), "is already a key type Record owns"),
    ] {
        refuses(
            &record(json!([plain.clone(), keyed("user_name", aliases)])),
            message,
        );
    }
    refuses(
        &record(json!([keyed("nickname", json!(["nick"])), plain])),
        "duplicate field nickname",
    );

    let mut forwards = record(json!([keyed("user_name", json!(["userName", "username"]))]));
    let mut backwards = record(json!([keyed("user_name", json!(["username", "userName"]))]));
    forwards.validate().unwrap();
    backwards.validate().unwrap();
    forwards.canonicalize();
    backwards.canonicalize();
    assert_eq!(
        forwards.to_canonical_json().unwrap(),
        backwards.to_canonical_json().unwrap()
    );
    assert!(forwards.diff(&backwards).changes.is_empty());
}

#[test]
fn an_alias_is_refused_where_the_request_side_never_reads_the_key() {
    let keyed = |access: Option<&str>, direction: &str| {
        let mut field = json!({
            "wire": "server_id", "ident": ["server_id"], "ty": {"kind": "string"},
            "presence": "optional", "ext": {"com.pons.alias/v1": ["serverId"]}
        });
        if let Some(access) = access {
            field["access"] = json!(access);
            field["default"] = json!("server");
        }
        contract(json!({
            "Record": {
                "kind": "struct", "name": ["record"], "direction": direction,
                "openness": "reject", "fields": [field]
            }
        }))
    };
    keyed(None, "input").validate().unwrap();
    keyed(None, "both").validate().unwrap();
    keyed(Some("write_only"), "both").validate().unwrap();
    refuses(
        &keyed(None, "output"),
        "name input spellings of a key that never crosses a request boundary",
    );
    refuses(
        &keyed(Some("read_only"), "both"),
        "name input spellings of a key that never crosses a request boundary",
    );
    refuses(
        &keyed(Some("read_only"), "input"),
        "name input spellings of a key that never crosses a request boundary",
    );
}
