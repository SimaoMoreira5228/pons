use pons_ir::{ByteEncoding, Direction, Document, TypeRef, Violation, validate_value};
use serde_json::{Value, json};

fn contract(types: Value) -> Document {
    serde_json::from_value(json!({
        "ir_version": "0.2",
        "info": {"title": "Example", "version": "1"},
        "types": types,
    }))
    .unwrap()
}

fn violations(types: Value, ty: &TypeRef, direction: Direction, value: Value) -> Vec<Violation> {
    let document = contract(types);
    document.validate().unwrap();
    validate_value(&document, ty, direction, &value).unwrap_err()
}

fn messages(found: Vec<Violation>) -> Vec<String> {
    found.iter().map(ToString::to_string).collect()
}

fn checked(types: Value, ty: &TypeRef, value: Value) {
    checked_in(types, ty, Direction::Both, value)
}

fn checked_in(types: Value, ty: &TypeRef, direction: Direction, value: Value) {
    let document = contract(types);
    document.validate().unwrap();
    if let Err(found) = validate_value(&document, ty, direction, &value) {
        panic!("{value} was rejected: {}", messages(found).join(", "));
    }
}

fn inline(extra: Value) -> TypeRef {
    serde_json::from_value(extra).unwrap()
}

fn text(minimum: Option<u64>, maximum: Option<u64>, pattern: Option<&str>) -> TypeRef {
    serde_json::from_value(json!({
        "kind": "string",
        "min_length": minimum,
        "max_length": maximum,
        "pattern": pattern,
    }))
    .unwrap()
}

fn named(id: &str) -> TypeRef {
    TypeRef::Named(pons_ir::NamedRef {
        id: id.into(),
        variant: None,
    })
}

fn narrowed(id: &str, variant: &str) -> TypeRef {
    TypeRef::Named(pons_ir::NamedRef {
        id: id.into(),
        variant: Some(variant.into()),
    })
}

fn struct_type(id: &str, openness: &str, fields: Value, flattened: Value) -> Value {
    json!({
        id: {
            "kind": "struct",
            "name": [id],
            "direction": "both",
            "openness": openness,
            "fields": fields,
            "flattened": flattened,
        }
    })
}

fn field(wire: &str, ty: &TypeRef, presence: &str) -> Value {
    json!({
        "wire": wire,
        "ident": [wire],
        "ty": ty,
        "presence": presence,
    })
}

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

#[test]
fn arrays_check_every_item_and_their_uniqueness() {
    let numbers = inline(json!({"kind": "int", "bits": 8, "signed": false, "wire": "number"}));
    let list = inline(json!({"kind": "array", "item": numbers.clone()}));
    let unique = inline(json!({"kind": "array", "item": numbers, "unique": true}));
    let types = json!({});
    checked(types.clone(), &list, json!([1, 2, 2]));
    checked(types.clone(), &unique, json!([1, 2]));
    assert_eq!(
        messages(violations(
            types.clone(),
            &list,
            Direction::Both,
            json!({"first": 1})
        )),
        ["expected an array, found an object"]
    );
    assert_eq!(
        messages(violations(
            types.clone(),
            &list,
            Direction::Both,
            json!([1, 300])
        )),
        ["/1: integer is outside the u8 range"]
    );
    assert_eq!(
        messages(violations(
            types,
            &unique,
            Direction::Both,
            json!([1, 2, 1])
        )),
        ["array item 2 repeats item 0"]
    );
}

fn literal(text: &str) -> Value {
    serde_json::from_str(text).unwrap()
}

#[test]
fn a_whole_value_written_with_an_exponent_is_that_integer() {
    let types = struct_type(
        "Counts",
        "reject",
        json!([
            field(
                "small",
                &inline(json!({"kind": "int", "bits": 32, "signed": false, "wire": "number"})),
                "required"
            ),
            field(
                "wide",
                &inline(json!({"kind": "int", "bits": 128, "signed": false, "wire": "number"})),
                "required"
            ),
        ]),
        json!([]),
    );
    let body = |small: &str, wide: &str| json!({"small": literal(small), "wide": literal(wide)});

    for text in [
        "1e3", "1E2", "1.0", "1e0", "1000.0", "1.5e2", "0e-2", "-0.0",
    ] {
        checked(types.clone(), &named("Counts"), body(text, text));
    }
    for text in [
        "1e+21",
        "1e30",
        "1e38",
        "340282366920938463463374607431768211455",
    ] {
        checked(types.clone(), &named("Counts"), body("1", text));
    }
    for (text, found) in [
        ("1.5", "expected an exact integer, found 1.5"),
        ("1e-2", "expected an exact integer, found 1e-2"),
        ("-1e-2", "expected an exact integer, found -1e-2"),
        ("1e30", "integer is outside the u32 range"),
        ("1e39", "integer is outside the u32 range"),
        ("1e400", "integer is outside the u32 range"),
        (
            "340282366920938463463374607431768211456",
            "integer is outside the u32 range",
        ),
    ] {
        assert_eq!(
            messages(violations(
                types.clone(),
                &named("Counts"),
                Direction::Both,
                body(text, "1")
            )),
            [format!("/small: {found}")],
            "{text}"
        );
    }
    for (text, found) in [
        ("1.5", "expected an exact integer, found 1.5"),
        ("1e-2", "expected an exact integer, found 1e-2"),
        ("1e39", "integer is outside the u128 range"),
        ("1e400", "integer is outside the u128 range"),
        (
            "340282366920938463463374607431768211456",
            "integer is outside the u128 range",
        ),
    ] {
        assert_eq!(
            messages(violations(
                types.clone(),
                &named("Counts"),
                Direction::Both,
                body("1", text)
            )),
            [format!("/wide: {found}")],
            "{text}"
        );
    }
}

#[test]
fn a_byte_array_reads_an_exponent_as_the_whole_byte_it_is() {
    let types = json!({});
    let bytes = inline(json!({"kind": "bytes", "encoding": "array"}));

    checked(
        types.clone(),
        &bytes,
        json!([literal("1e1"), literal("1.0"), literal("0e0"), 255]),
    );
    assert_eq!(
        messages(violations(
            types.clone(),
            &bytes,
            Direction::Both,
            json!([literal("1e400")])
        )),
        ["/0: expected a byte between 0 and 255, found 1e+400"]
    );
    assert_eq!(
        messages(violations(
            types.clone(),
            &bytes,
            Direction::Both,
            json!([literal("1e39")])
        )),
        ["/0: expected a byte between 0 and 255, found 1e+39"]
    );
    assert_eq!(
        messages(violations(
            types,
            &bytes,
            Direction::Both,
            json!([literal("1e-1")])
        )),
        ["/0: expected a byte between 0 and 255, found a number"]
    );
}

#[test]
fn unique_compares_numbers_by_value_and_the_rest_structurally() {
    let types = json!({});
    let unique = inline(json!({"kind": "array", "item": {"kind": "any"}, "unique": true}));

    for (value, repeats) in [
        (json!([1, 1.0]), 1),
        (json!([1, 1.0, 2]), 1),
        (json!([0, -0.0]), 1),
        (json!([0.0, -0.0]), 1),
        (json!([1e0, 1]), 1),
        (json!([1e21, 1000000000000000000000u128]), 1),
        (json!(["a", "a"]), 1),
        (json!([{"a": 1, "b": 2}, {"b": 2, "a": 1}]), 1),
        (json!([[1, [2]], [1.0, [2.0]]]), 1),
        (json!([1, 2, 1.0]), 2),
    ] {
        assert_eq!(
            messages(violations(
                types.clone(),
                &unique,
                Direction::Both,
                value.clone()
            )),
            [format!("array item {repeats} repeats item 0")],
            "{value}"
        );
    }
    for value in [
        json!([1, 2]),
        json!([1, 2.0]),
        json!([0, -1.5]),
        json!([1, "1"]),
        json!([9007199254740993u64, 9007199254740993.0]),
        json!([1e21, 1000000000000000000001u128]),
        json!(["a", "b"]),
        json!([{"a": 1}, {"a": 2}]),
        json!([{"a": 1}, {"a": 1, "b": 2}]),
        json!([[1], [1, 2]]),
    ] {
        checked(types.clone(), &unique, value);
    }
}

#[test]
fn tuples_check_arity_and_items() {
    let pair = inline(json!({
        "kind": "tuple",
        "items": [{"kind": "string"}, {"kind": "int", "bits": 8, "signed": false, "wire": "number"}]
    }));
    let types = json!({});
    checked(types.clone(), &pair, json!(["a", 1]));
    assert_eq!(
        messages(violations(
            types.clone(),
            &pair,
            Direction::Both,
            json!(["a"])
        )),
        ["expected a tuple of 2 items, found 1"]
    );
    assert_eq!(
        messages(violations(types, &pair, Direction::Both, json!(["a", "b"]))),
        ["/1: expected a number, found a string"]
    );
}

#[test]
fn maps_require_string_keys_and_check_values() {
    let key = text(Some(1), None, Some("[a-z]+"));
    let counts = inline(
        json!({"kind": "map", "key": key, "value": {"kind": "int", "bits": 8, "signed": false, "wire": "number"}}),
    );
    let types = json!({});
    checked(types.clone(), &counts, json!({"a": 1, "b": 2}));
    assert_eq!(
        messages(violations(
            types.clone(),
            &counts,
            Direction::Both,
            json!([])
        )),
        ["expected an object, found an array"]
    );
    assert_eq!(
        messages(violations(
            types.clone(),
            &counts,
            Direction::Both,
            json!({"a": 300})
        )),
        ["/a: integer is outside the u8 range"]
    );
    assert_eq!(
        messages(violations(types, &counts, Direction::Both, json!({"A": 1}))),
        ["/A: string does not match the pattern [a-z]+"]
    );
}

#[test]
fn nullable_accepts_null_and_any_accepts_everything() {
    let types = json!({});
    let nullable = inline(
        json!({"kind": "nullable", "inner": {"kind": "int", "bits": 8, "signed": false, "wire": "number"}}),
    );
    checked(types.clone(), &nullable, json!(null));
    checked(types.clone(), &nullable, json!(1));
    assert_eq!(
        messages(violations(
            types.clone(),
            &nullable,
            Direction::Both,
            json!("1")
        )),
        ["expected a number, found a string"]
    );
    checked(types.clone(), &inline(json!({"kind": "null"})), json!(null));
    assert_eq!(
        messages(violations(
            types.clone(),
            &inline(json!({"kind": "null"})),
            Direction::Both,
            json!(0)
        )),
        ["expected null, found a number"]
    );
    for value in [
        json!(null),
        json!(1),
        json!("text"),
        json!([1]),
        json!({"a": 1}),
    ] {
        checked(types.clone(), &inline(json!({"kind": "any"})), value);
    }
}

#[test]
fn opaque_values_pass_because_the_codec_owns_them() {
    let types = json!({});
    let opaque = inline(json!({"kind": "opaque", "codec": "example", "wire": "json"}));
    for value in [json!(null), json!("anything"), json!({"nested": [1, 2]})] {
        checked(types.clone(), &opaque, value);
    }
}

#[test]
fn named_types_resolve_and_unknown_references_are_reported() {
    let types = struct_type(
        "User",
        "reject",
        json!([field("id", &text(None, None, None), "required")]),
        json!([]),
    );
    checked(types.clone(), &named("User"), json!({"id": "7"}));
    assert_eq!(
        messages(violations(
            types.clone(),
            &named("Ghost"),
            Direction::Both,
            json!({})
        )),
        ["unknown type Ghost"]
    );
    assert_eq!(
        messages(violations(
            types,
            &named("User"),
            Direction::Both,
            json!({"id": 7})
        )),
        ["/id: expected a string, found a number"]
    );
}

#[test]
fn newtypes_are_transparent() {
    let types = json!({
        "Name": {
            "kind": "newtype",
            "name": ["name"],
            "direction": "both",
            "inner": {"kind": "string", "min_length": 2},
        }
    });
    checked(types.clone(), &named("Name"), json!("ab"));
    assert_eq!(
        messages(violations(
            types,
            &named("Name"),
            Direction::Both,
            json!("a")
        )),
        ["string is shorter than the 2 code points it allows"]
    );
}

#[test]
fn structs_report_missing_fields_and_reject_unknown_ones() {
    let types = struct_type(
        "User",
        "reject",
        json!([
            field("id", &text(None, None, None), "required"),
            field("name", &text(None, None, None), "optional"),
        ]),
        json!([]),
    );
    checked(types.clone(), &named("User"), json!({"id": "7"}));
    assert_eq!(
        messages(violations(
            types.clone(),
            &named("User"),
            Direction::Both,
            json!({})
        )),
        ["/id: missing required field id"]
    );
    assert_eq!(
        messages(violations(
            types,
            &named("User"),
            Direction::Both,
            json!({"id": "7", "role": "admin"})
        )),
        ["/role: unknown property role"]
    );
}

#[test]
fn openness_policies_govern_extra_properties() {
    let id = field("id", &text(None, None, None), "required");
    let count = json!({"kind": "int", "bits": 8, "signed": false, "wire": "number"});
    let types = json!({
        "Counts": {
            "kind": "newtype",
            "name": ["counts"],
            "direction": "both",
            "inner": {"kind": "map", "key": {"kind": "string"}, "value": count},
        },
        "Opaque": {
            "kind": "struct",
            "name": ["opaque"],
            "direction": "both",
            "openness": "preserve_opaque",
            "fields": [id.clone()],
        },
        "Typed": {
            "kind": "struct",
            "name": ["typed"],
            "direction": "both",
            "openness": {"preserve_typed": {"value": {"ref": "Counts"}}},
            "fields": [json!({
                "wire": "id", "ident": ["id"], "ty": count, "presence": "required"
            })],
        },
        "Closed": {
            "kind": "struct",
            "name": ["closed"],
            "direction": "both",
            "openness": "reject",
            "fields": [id],
        },
    });
    checked(
        types.clone(),
        &named("Opaque"),
        json!({"id": "7", "extra": {"any": true}}),
    );
    checked(types.clone(), &named("Typed"), json!({"id": 3, "hits": 3}));
    assert_eq!(
        messages(violations(
            types.clone(),
            &named("Typed"),
            Direction::Both,
            json!({"id": 3, "hits": 300})
        )),
        ["/hits: integer is outside the u8 range"]
    );
    assert_eq!(
        messages(violations(
            types.clone(),
            &named("Typed"),
            Direction::Both,
            json!({"id": 3, "hits": {"a": 1}})
        )),
        ["/hits: expected a number, found an object"]
    );
    assert_eq!(
        messages(violations(
            types,
            &named("Closed"),
            Direction::Both,
            json!({"id": "7", "role": "admin"})
        )),
        ["/role: unknown property role"]
    );
}

#[test]
fn flattened_members_share_the_field_namespace() {
    let types = json!({
        "Audit": {
            "kind": "struct",
            "name": ["audit"],
            "direction": "both",
            "openness": "reject",
            "fields": [field("at", &inline(json!({"kind": "string"})), "required")],
        },
        "Event": {
            "kind": "struct",
            "name": ["event"],
            "direction": "both",
            "openness": "reject",
            "fields": [field("id", &inline(json!({"kind": "string"})), "required")],
            "flattened": [{"ref": "Audit"}],
        }
    });
    checked(
        types.clone(),
        &named("Event"),
        json!({"id": "1", "at": "now"}),
    );
    assert_eq!(
        messages(violations(
            types.clone(),
            &named("Event"),
            Direction::Both,
            json!({"id": "1"})
        )),
        ["/at: missing required field at"]
    );
    assert_eq!(
        messages(violations(
            types,
            &named("Event"),
            Direction::Both,
            json!({"id": "1", "at": "now", "extra": 1})
        )),
        ["/extra: unknown property extra"]
    );
}

fn enum_type(id: &str, repr: Value, variants: Value) -> Value {
    json!({
        id: {
            "kind": "enum",
            "name": [id],
            "direction": "both",
            "repr": repr,
            "variants": variants,
        }
    })
}

fn unit(wire: &str) -> Value {
    json!({"wire": wire, "ident": [wire], "payload": {"kind": "unit"}})
}

fn newtype_variant(wire: &str, ty: &TypeRef) -> Value {
    json!({"wire": wire, "ident": [wire], "payload": {"kind": "newtype", "ty": ty}})
}

#[test]
fn string_enums_name_their_variant() {
    let types = enum_type(
        "State",
        json!({"kind": "string"}),
        json!([unit("open"), unit("closed")]),
    );
    checked(types.clone(), &named("State"), json!("open"));
    assert_eq!(
        messages(violations(
            types.clone(),
            &named("State"),
            Direction::Both,
            json!("gone")
        )),
        ["unknown enum variant gone"]
    );
    assert_eq!(
        messages(violations(
            types,
            &named("State"),
            Direction::Both,
            json!(1)
        )),
        ["expected an enum variant name, found a number"]
    );
}

#[test]
fn external_enums_carry_their_payload_beside_the_variant() {
    let count = inline(json!({"kind": "int", "bits": 8, "signed": false, "wire": "number"}));
    let types = enum_type(
        "Shape",
        json!({"kind": "external"}),
        json!([unit("point"), newtype_variant("circle", &count)]),
    );
    checked(types.clone(), &named("Shape"), json!({"circle": 3}));
    checked(types.clone(), &named("Shape"), json!("point"));
    assert_eq!(
        messages(violations(
            types.clone(),
            &named("Shape"),
            Direction::Both,
            json!({"circle": 300})
        )),
        ["/circle: integer is outside the u8 range"]
    );
    assert_eq!(
        messages(violations(
            types.clone(),
            &named("Shape"),
            Direction::Both,
            json!({"circle": 1, "point": null})
        )),
        ["expected an object with one variant, found an object"]
    );
    assert_eq!(
        messages(violations(
            types,
            &named("Shape"),
            Direction::Both,
            json!({"blob": 1})
        )),
        ["unknown enum variant blob"]
    );
}

#[test]
fn internal_and_adjacent_enums_read_their_tag() {
    let text_type = inline(json!({"kind": "string"}));
    let variants = json!([
        unit("invalid"),
        {
            "wire": "locked",
            "ident": ["locked"],
            "payload": {
                "kind": "struct",
                "fields": [field("retry_after", &inline(json!({"kind": "int", "bits": 32, "signed": false, "wire": "number"})), "required")],
            }
        },
        newtype_variant("hint", &text_type),
    ]);
    let internal = enum_type(
        "Internal",
        json!({"kind": "internal", "tag": "code"}),
        variants.clone(),
    );
    let adjacent = enum_type(
        "Adjacent",
        json!({"kind": "adjacent", "tag": "code", "content": "detail"}),
        variants,
    );
    let types = json!({
        "Internal": internal["Internal"].clone(),
        "Adjacent": adjacent["Adjacent"].clone(),
    });
    checked(
        types.clone(),
        &named("Internal"),
        json!({"code": "invalid"}),
    );
    checked(
        types.clone(),
        &named("Internal"),
        json!({"code": "locked", "retry_after": 30}),
    );
    assert_eq!(
        messages(violations(
            types.clone(),
            &named("Internal"),
            Direction::Both,
            json!({"code": "locked"})
        )),
        ["/retry_after: missing required field retry_after"]
    );
    assert_eq!(
        messages(violations(
            types.clone(),
            &named("Internal"),
            Direction::Both,
            json!({"state": "invalid"})
        )),
        ["/code: missing required field code"]
    );
    checked(
        types.clone(),
        &named("Adjacent"),
        json!({"code": "invalid"}),
    );
    checked(
        types.clone(),
        &named("Adjacent"),
        json!({"code": "hint", "detail": "try later"}),
    );
    assert_eq!(
        messages(violations(
            types,
            &named("Adjacent"),
            Direction::Both,
            json!({"code": "hint"})
        )),
        ["/detail: missing required field detail"]
    );
}

#[test]
fn untagged_enums_accept_the_first_variant_that_fits() {
    let text_type = inline(json!({"kind": "string"}));
    let count = inline(json!({"kind": "int", "bits": 8, "signed": false, "wire": "number"}));
    let types = enum_type(
        "Value",
        json!({"kind": "untagged"}),
        json!([
            unit("nothing"),
            newtype_variant("text", &text_type),
            newtype_variant("count", &count),
        ]),
    );
    checked(types.clone(), &named("Value"), json!(null));
    checked(types.clone(), &named("Value"), json!("text"));
    checked(types.clone(), &named("Value"), json!(3));
    assert_eq!(
        messages(violations(
            types,
            &named("Value"),
            Direction::Both,
            json!(300)
        )),
        ["value matches none of the enum variants"]
    );
}

#[test]
fn a_narrowed_reference_validates_only_its_variant() {
    let text_type = inline(json!({"kind": "string"}));
    let types = enum_type(
        "Shape",
        json!({"kind": "external"}),
        json!([unit("point"), newtype_variant("label", &text_type)]),
    );
    checked(
        types.clone(),
        &narrowed("Shape", "point"),
        json!({"point": null}),
    );
    assert_eq!(
        messages(violations(
            types.clone(),
            &narrowed("Shape", "point"),
            Direction::Both,
            json!({"label": "x"})
        )),
        ["expected enum variant point, found label"]
    );
    assert_eq!(
        messages(violations(
            types,
            &narrowed("Shape", "ghost"),
            Direction::Both,
            json!({"point": null})
        )),
        ["expected enum variant ghost, found point"]
    );
}

#[test]
fn recursive_types_terminate() {
    let types = json!({
        "Node": {
            "kind": "struct",
            "name": ["node"],
            "direction": "both",
            "openness": "reject",
            "fields": [
                field("value", &inline(json!({"kind": "int", "bits": 8, "signed": false, "wire": "number"})), "required"),
                field("next", &inline(json!({"kind": "nullable", "inner": {"ref": "Node"}})), "required"),
            ],
        },
        "Deaf": {
            "kind": "struct",
            "name": ["deaf"],
            "direction": "both",
            "openness": "reject",
            "fields": [field("child", &named("Deaf"), "required")],
        },
        "Wrapped": {
            "kind": "newtype",
            "name": ["wrapped"],
            "direction": "both",
            "inner": {"ref": "Node"},
        }
    });
    let chain = json!({"value": 1, "next": {"value": 2, "next": {"value": 3, "next": null}}});
    checked(types.clone(), &named("Node"), chain);
    assert_eq!(
        messages(violations(
            types.clone(),
            &named("Node"),
            Direction::Both,
            json!({"value": 1, "next": {"value": 2}})
        )),
        ["/next/next: missing required field next"]
    );
    assert_eq!(
        messages(violations(
            types.clone(),
            &named("Deaf"),
            Direction::Both,
            json!({"child": {"child": {}}})
        )),
        ["/child/child/child: missing required field child"]
    );
    checked(
        types.clone(),
        &named("Wrapped"),
        json!({"value": 1, "next": null}),
    );
}

#[test]
fn direction_controls_read_only_and_write_only_fields() {
    let field = |wire: &str, presence: &str, access: &str| {
        json!({
            "wire": wire,
            "ident": [wire],
            "ty": {"kind": "string"},
            "presence": presence,
            "access": access,
        })
    };
    let defaulted = |mut field: Value| {
        field["default"] = json!("filled");
        field
    };
    let record = |fields| {
        json!({
            "Record": {
                "kind": "struct",
                "name": ["record"],
                "direction": "both",
                "openness": "reject",
                "fields": fields,
            }
        })
    };
    let mixed = record(json!([
        defaulted(field("server_id", "required", "read_only")),
        defaulted(field("secret", "required", "write_only")),
        field("note", "optional", "write_only"),
    ]));
    let open = record(json!([
        field("server_id", "optional", "read_only"),
        field("tag", "required", "read_write"),
    ]));

    checked_in(
        open.clone(),
        &named("Record"),
        Direction::Input,
        json!({"tag": "t"}),
    );
    checked_in(
        open.clone(),
        &named("Record"),
        Direction::Output,
        json!({"server_id": "a", "tag": "t"}),
    );
    assert_eq!(
        messages(violations(
            mixed.clone(),
            &named("Record"),
            Direction::Input,
            json!({"server_id": "a", "secret": "b"})
        )),
        ["/server_id: field server_id is read_only and cannot cross the input boundary"]
    );
    checked_in(
        mixed.clone(),
        &named("Record"),
        Direction::Input,
        json!({"secret": "b"}),
    );
    assert_eq!(
        messages(violations(
            mixed.clone(),
            &named("Record"),
            Direction::Output,
            json!({"server_id": "a", "secret": "b"})
        )),
        ["/secret: field secret is write_only and cannot cross the output boundary"]
    );
    checked_in(
        mixed.clone(),
        &named("Record"),
        Direction::Output,
        json!({"server_id": "a"}),
    );
    assert_eq!(
        messages(violations(
            mixed.clone(),
            &named("Record"),
            Direction::Output,
            json!({"server_id": "a", "note": "n"})
        )),
        ["/note: field note is write_only and cannot cross the output boundary"]
    );
    assert_eq!(
        messages(violations(
            mixed,
            &named("Record"),
            Direction::Input,
            json!({})
        )),
        ["/secret: missing required field secret"]
    );
    assert_eq!(
        messages(violations(
            open,
            &named("Record"),
            Direction::Input,
            json!({"server_id": "a", "tag": "t"})
        )),
        ["/server_id: field server_id is read_only and cannot cross the input boundary"]
    );
}

#[test]
fn a_default_fills_the_key_a_direction_drops() {
    let record = |default: Option<Value>, access: &str| {
        let mut field = field("tenant", &text(None, None, None), "required");
        field["access"] = json!(access);
        if let Some(default) = default {
            field["default"] = default;
        }
        json!({
            "Record": {
                "kind": "struct",
                "name": ["record"],
                "direction": "both",
                "openness": "reject",
                "fields": [field],
            }
        })
    };
    let checked = |access: &str, default: Option<Value>, value: Value| {
        let direction = match access {
            "read_only" => Direction::Input,
            _ => Direction::Output,
        };
        let document = contract(record(default, access));
        document.validate().unwrap();
        validate_value(&document, &named("Record"), direction, &value)
            .err()
            .map(messages)
            .unwrap_or_default()
    };

    for (access, message) in [
        (
            "read_only",
            "/tenant: field tenant is read_only and cannot cross the input boundary",
        ),
        (
            "write_only",
            "/tenant: field tenant is write_only and cannot cross the output boundary",
        ),
    ] {
        assert!(
            contract(record(None, access)).validate().is_err(),
            "{access}: a required key the dropping side waits for cannot be filled"
        );
        assert!(checked(access, Some(json!("acme")), json!({})).is_empty());
        assert_eq!(
            checked(access, Some(json!("acme")), json!({"tenant": "acme"})),
            [message]
        );
    }
}
