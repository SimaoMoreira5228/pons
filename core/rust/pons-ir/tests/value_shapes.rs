mod support;

use pons_ir::{Direction, validate_value};
use serde_json::{Value, json};
use support::{
    checked, checked_in, contract, field_ref as field, inline, messages, named,
    struct_map as struct_type, text, violations,
};

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

#[test]
fn an_alias_is_another_accepted_spelling_of_a_field_on_the_input_side() {
    let record = |field: Value| {
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
    let aliased = |wire: &str, aliases: Value, presence: &str, access: Option<&str>| {
        let mut field = field(wire, &text(None, None, Some("[a-z]+")), presence);
        field["ext"] = json!({"com.pons.alias/v1": aliases});
        if let Some(access) = access {
            field["access"] = json!(access);
        }
        record(field)
    };

    let types = aliased(
        "user_name",
        json!(["userName", "username"]),
        "required",
        None,
    );
    for value in [
        json!({"user_name": "ada"}),
        json!({"userName": "ada"}),
        json!({"username": "ada"}),
    ] {
        checked_in(types.clone(), &named("Record"), Direction::Input, value);
    }
    assert_eq!(
        messages(violations(
            types.clone(),
            &named("Record"),
            Direction::Input,
            json!({}),
        )),
        ["/user_name: missing required field user_name"]
    );
    assert_eq!(
        messages(violations(
            types.clone(),
            &named("Record"),
            Direction::Input,
            json!({"UserName": "ada"}),
        )),
        [
            "/user_name: missing required field user_name",
            "/UserName: unknown property UserName"
        ]
    );
    assert_eq!(
        messages(violations(
            types.clone(),
            &named("Record"),
            Direction::Input,
            json!({"userName": "9"}),
        )),
        ["/user_name: string does not match the pattern [a-z]+"]
    );
    assert_eq!(
        messages(violations(
            types.clone(),
            &named("Record"),
            Direction::Input,
            json!({"user_name": "ada", "userName": "ada"}),
        )),
        ["/userName: field user_name is written twice, once as user_name"]
    );
    assert_eq!(
        messages(violations(
            types.clone(),
            &named("Record"),
            Direction::Input,
            json!({"user_name": "ada", "username": "ada"}),
        )),
        ["/username: field user_name is written twice, once as user_name"]
    );

    assert_eq!(
        messages(violations(
            types.clone(),
            &named("Record"),
            Direction::Output,
            json!({"userName": "ada"}),
        )),
        [
            "/user_name: missing required field user_name",
            "/userName: unknown property userName"
        ]
    );
    assert_eq!(
        messages(violations(
            types.clone(),
            &named("Record"),
            Direction::Output,
            json!({"username": "ada"}),
        )),
        [
            "/user_name: missing required field user_name",
            "/username: unknown property username"
        ]
    );

    let optional = aliased("nickname", json!(["nickName"]), "optional", None);
    checked_in(
        optional.clone(),
        &named("Record"),
        Direction::Input,
        json!({}),
    );
    checked_in(
        optional,
        &named("Record"),
        Direction::Input,
        json!({"nickName": "ada"}),
    );
}

#[test]
fn an_alias_spells_a_key_in_every_object_the_field_belongs_to() {
    let types = json!({
        "Open": {
            "kind": "struct",
            "name": ["open"],
            "direction": "both",
            "openness": {"preserve_typed": {"value": {
                "kind": "map", "key": {"kind": "string"}, "value": {"kind": "string"}
            }}},
            "fields": [json!({
                "wire": "user_name",
                "ident": ["user_name"],
                "ty": {"kind": "string"},
                "presence": "required",
                "ext": {"com.pons.alias/v1": ["userName"]}
            })],
        },
        "Choice": {
            "kind": "enum",
            "name": ["choice"],
            "direction": "both",
            "repr": {"kind": "untagged"},
            "variants": [{
                "wire": "named",
                "ident": ["named"],
                "payload": {
                    "kind": "struct",
                    "fields": [{
                        "wire": "user_name",
                        "ident": ["user_name"],
                        "ty": {"kind": "string"},
                        "presence": "required",
                        "ext": {"com.pons.alias/v1": ["userName"]}
                    }]
                }
            }],
        }
    });
    checked_in(
        types.clone(),
        &named("Open"),
        Direction::Input,
        json!({"userName": "ada", "note": "hi"}),
    );
    assert_eq!(
        messages(violations(
            types.clone(),
            &named("Open"),
            Direction::Input,
            json!({"userName": "ada", "note": 7}),
        )),
        ["/note: expected a string, found a number"]
    );
    assert_eq!(
        messages(violations(
            types.clone(),
            &named("Open"),
            Direction::Input,
            json!({"user_name": "ada", "userName": "ada", "note": "hi"}),
        )),
        ["/userName: field user_name is written twice, once as user_name"]
    );
    checked_in(
        types.clone(),
        &named("Choice"),
        Direction::Input,
        json!({"userName": "ada"}),
    );
    assert_eq!(
        messages(violations(
            types,
            &named("Choice"),
            Direction::Input,
            json!({}),
        )),
        ["value matches none of the enum variants"]
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
