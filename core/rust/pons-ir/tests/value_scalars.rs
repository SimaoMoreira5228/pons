mod support;

use pons_ir::{ByteEncoding, Direction, Violation};
use serde_json::json;
use support::{checked, inline, messages, text, violations};

#[test]
fn booleans_reject_every_other_shape() {
    let types = json!({});
    checked(types.clone(), &inline(json!({"kind": "bool"})), json!(true));
    let found = violations(
        types,
        &inline(json!({"kind": "bool"})),
        Direction::Both,
        json!("true"),
    );
    assert_eq!(
        messages(found.clone()),
        ["expected a boolean, found a string"]
    );
    assert_eq!(found[0].path(), "");
}

#[test]
fn integers_enforce_their_native_range() {
    let unsigned = inline(json!({"kind": "int", "bits": 8, "signed": false, "wire": "number"}));
    let signed = inline(json!({"kind": "int", "bits": 8, "signed": true, "wire": "number"}));
    let types = json!({});
    checked(types.clone(), &unsigned, json!(255));
    checked(types.clone(), &signed, json!(-128));
    assert_eq!(
        messages(violations(
            types.clone(),
            &unsigned,
            Direction::Both,
            json!(256)
        )),
        ["integer is outside the u8 range"]
    );
    assert_eq!(
        messages(violations(
            types.clone(),
            &unsigned,
            Direction::Both,
            json!(-1)
        )),
        ["integer is outside the u8 range"]
    );
    assert_eq!(
        messages(violations(
            types.clone(),
            &signed,
            Direction::Both,
            json!(-129)
        )),
        ["integer is outside the i8 range"]
    );
    assert_eq!(
        messages(violations(types, &unsigned, Direction::Both, json!(-1.5))),
        ["expected an exact integer, found -1.5"]
    );
}

#[test]
fn integers_enforce_declared_bounds_and_multiples() {
    let ty = inline(json!({
        "kind": "int",
        "bits": 32,
        "signed": true,
        "wire": "number",
        "minimum": -10,
        "maximum": 10,
        "multiple_of": 5
    }));
    let types = json!({});
    checked(types.clone(), &ty, json!(-10));
    checked(types.clone(), &ty, json!(10));
    assert_eq!(
        messages(violations(types.clone(), &ty, Direction::Both, json!(-11))),
        ["integer is below its minimum of -10"]
    );
    assert_eq!(
        messages(violations(types.clone(), &ty, Direction::Both, json!(11))),
        ["integer is above its maximum of 10"]
    );
    assert_eq!(
        messages(violations(types, &ty, Direction::Both, json!(7))),
        ["integer is not a multiple of 5"]
    );
}

#[test]
fn integer_strings_must_be_canonical_and_fit_the_type() {
    let unsigned = inline(json!({"kind": "int", "bits": 64, "signed": false, "wire": "string"}));
    let signed = inline(json!({"kind": "int", "bits": 64, "signed": true, "wire": "string"}));
    let types = json!({});
    checked(types.clone(), &unsigned, json!("0"));
    checked(types.clone(), &unsigned, json!("18446744073709551615"));
    checked(types.clone(), &signed, json!("-9223372036854775808"));
    assert_eq!(
        messages(violations(
            types.clone(),
            &unsigned,
            Direction::Both,
            json!("007")
        )),
        ["expected an exact integer, found 007"]
    );
    assert_eq!(
        messages(violations(
            types.clone(),
            &signed,
            Direction::Both,
            json!("18446744073709551616")
        )),
        ["integer is outside the i64 range"]
    );
    assert_eq!(
        messages(violations(types, &unsigned, Direction::Both, json!(7))),
        ["expected an integer string, found a number"]
    );
}

#[test]
fn floats_match_the_range_the_client_enforces() {
    let single = inline(json!({"kind": "float", "bits": 32}));
    let double = inline(json!({"kind": "float", "bits": 64}));
    let types = json!({});
    checked(types.clone(), &single, json!(3.4e38));
    checked(types.clone(), &double, json!(3.5e38));
    assert_eq!(
        messages(violations(
            types.clone(),
            &single,
            Direction::Both,
            json!(3.5e38)
        )),
        ["number is outside the f32 range"]
    );
    assert_eq!(
        messages(violations(types, &single, Direction::Both, json!("3.5"))),
        ["expected a number, found a string"]
    );
}

#[test]
fn decimals_accept_the_number_and_string_wires() {
    let number = inline(json!({"kind": "decimal", "wire": "number"}));
    let string = inline(json!({"kind": "decimal", "wire": "string"}));
    let types = json!({});
    checked(types.clone(), &number, json!(0.1));
    for text in ["1", "-1.50", ".5", "1e300", "1."] {
        checked(types.clone(), &string, json!(text));
    }
    assert_eq!(
        messages(violations(
            types.clone(),
            &string,
            Direction::Both,
            json!("1.2.3")
        )),
        ["expected a decimal string, found \"1.2.3\""]
    );
    assert_eq!(
        messages(violations(
            types.clone(),
            &string,
            Direction::Both,
            json!("0x10")
        )),
        ["expected a decimal string, found \"0x10\""]
    );
    assert_eq!(
        messages(violations(types, &number, Direction::Both, json!("1"))),
        ["expected a number, found a string"]
    );
}

#[test]
fn strings_count_code_points_and_match_patterns_unanchored() {
    let bounded = text(Some(2), Some(3), None);
    let types = json!({});
    checked(types.clone(), &bounded, json!("éé"));
    assert_eq!(
        messages(violations(
            types.clone(),
            &bounded,
            Direction::Both,
            json!("é")
        )),
        ["string is shorter than the 2 code points it allows"]
    );
    assert_eq!(
        messages(violations(
            types.clone(),
            &bounded,
            Direction::Both,
            json!("éééé")
        )),
        ["string is longer than the 3 code points it allows"]
    );

    let pattern = text(None, None, Some("[0-9]{3}"));
    checked(types.clone(), &pattern, json!("id-123-x"));
    assert_eq!(
        messages(violations(
            types,
            &pattern,
            Direction::Both,
            json!("id-12-x")
        )),
        ["string does not match the pattern [0-9]{3}"]
    );
}

#[test]
fn patterns_that_do_not_compile_become_violations() {
    let pattern = text(None, None, Some("^[a-"));
    let types = json!({});
    let found = violations(types.clone(), &pattern, Direction::Both, json!("anything"));
    assert_eq!(
        messages(found.clone()),
        ["pattern ^[a- cannot be evaluated because unterminated character class"]
    );
    assert!(matches!(found[0], Violation::UnreadablePattern { .. }));

    let lookahead = text(None, None, Some("(?=a)b"));
    assert_eq!(
        messages(violations(types, &lookahead, Direction::Both, json!("ab"))),
        ["pattern (?=a)b cannot be evaluated because lookahead is not supported"]
    );
}

#[test]
fn bytes_follow_the_padding_rules_the_client_uses() {
    let types = json!({});
    for (encoding, shaped) in [
        (ByteEncoding::Hex, vec!["", "0aff", "0A"]),
        (ByteEncoding::Base64, vec!["", "YWJj", "YWI=", "YQ=="]),
        (
            ByteEncoding::Base64url,
            vec!["", "YWJj", "YWI=", "YQ==", "YWI", "YQ"],
        ),
    ] {
        let ty = inline(json!({"kind": "bytes", "encoding": encoding}));
        for text in shaped {
            checked(types.clone(), &ty, json!(text));
        }
    }

    let base64 = inline(json!({"kind": "bytes", "encoding": "base64"}));
    for rejected in ["YWI", "YQ", "YWJjZA", "YW ="] {
        assert_eq!(
            messages(violations(
                types.clone(),
                &base64,
                Direction::Both,
                json!(rejected)
            )),
            ["invalid base64 bytes"],
            "{rejected}"
        );
    }

    let hex = inline(json!({"kind": "bytes", "encoding": "hex"}));
    assert_eq!(
        messages(violations(
            types.clone(),
            &hex,
            Direction::Both,
            json!("abc")
        )),
        ["invalid hexadecimal bytes"]
    );

    let array = inline(json!({"kind": "bytes", "encoding": "array"}));
    checked(types.clone(), &array, json!([0, 255]));
    assert_eq!(
        messages(violations(types, &array, Direction::Both, json!([0, 256]))),
        ["/1: expected a byte between 0 and 255, found 256"]
    );
}
