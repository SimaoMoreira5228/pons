mod support;

use pons_ir::{
    Direction, Document, InlineType, NamedRef, ObjectOpenness, TypeKind, TypeRef, Violation,
    validate_value,
};
use serde_json::{Value, json};
use support::{
    contract, field, field_type, fixture, refuses, request_body, string, struct_type, with,
};

#[test]
fn a_pattern_that_does_not_compile_is_refused() {
    for pattern in ["[a-", "(?=x)", "a{2,1}", "*x", "a\\"] {
        let document = contract(json!({
            "Text": {
                "kind": "struct", "name": ["text"], "direction": "both", "openness": "reject",
                "fields": [{"wire": "value", "ident": ["value"], "ty": {"kind": "string", "pattern": pattern}, "presence": "optional"}]
            }
        }));
        let reason = match &validate_value(
            &document,
            field_type(&document, "Text", 0),
            Direction::Both,
            &json!("x"),
        )
        .unwrap_err()[..]
        {
            [Violation::UnreadablePattern { reason, .. }] => reason.clone(),
            other => panic!("{pattern:?} did not report an unreadable pattern: {other:?}"),
        };
        refuses(
            &document,
            &format!(
                "string pattern {pattern:?} is not a usable regular expression because {reason}"
            ),
        );
    }

    contract(json!({
        "Text": {
            "kind": "struct", "name": ["text"], "direction": "both", "openness": "reject",
            "fields": [{"wire": "value", "ident": ["value"], "ty": {"kind": "string", "pattern": "^a{2,4}$"}, "presence": "optional"}]
        }
    }))
    .validate()
    .unwrap();
}

#[test]
fn a_default_that_contradicts_its_field_type_is_refused() {
    let defaulted = |ty: Value, default: Value| {
        contract(json!({
            "Text": {
                "kind": "struct", "name": ["text"], "direction": "both", "openness": "reject",
                "fields": [{"wire": "value", "ident": ["value"], "ty": ty, "presence": "optional", "default": default}]
            }
        }))
    };

    refuses(
        &defaulted(json!({"kind": "string", "format": "email"}), json!(null)),
        "the default for field value contradicts its type: expected a string, found null",
    );
    refuses(
        &defaulted(
            json!({"kind": "int", "bits": 8, "signed": false, "wire": "number", "maximum": 10}),
            json!(11),
        ),
        "the default for field value contradicts its type: integer is above its maximum of 10",
    );
    refuses(
        &defaulted(
            json!({"kind": "string", "pattern": "[a-"}),
            json!("anything"),
        ),
        "the default for field value contradicts its type: pattern [a- cannot be evaluated because unterminated character class",
    );
    for accepted in [
        (
            json!({"kind": "string", "format": "email"}),
            json!("a@b.test"),
        ),
        (
            json!({"kind": "int", "bits": 8, "signed": false, "wire": "number", "maximum": 10}),
            json!(10),
        ),
        (
            json!({"kind": "nullable", "inner": {"kind": "string"}}),
            json!(null),
        ),
    ] {
        defaulted(accepted.0, accepted.1).validate().unwrap();
    }

    contract(json!({
        "Text": {"kind": "struct", "name": ["text"], "direction": "both", "openness": "reject",
            "fields": [{"wire": "value", "ident": ["value"], "ty": {"kind": "string"}, "presence": "required"}]}
    }))
    .validate()
    .unwrap();
    refuses(
        &contract(json!({
            "Choice": {
                "kind": "enum", "name": ["choice"], "direction": "both",
                "repr": {"kind": "internal", "tag": "code"},
                "variants": [{"wire": "held", "ident": ["held"], "payload": {"kind": "struct", "fields": [
                    {"wire": "token", "ident": ["token"], "ty": {"kind": "string"}, "presence": "optional", "default": 7}
                ]}}]
            }
        })),
        "the default for field token contradicts its type: expected a string, found a number",
    );
    fixture().validate().unwrap();
}

#[test]
fn a_typed_openness_value_types_only_the_extra_keys() {
    let openness = |fields: Vec<Value>, flattened: Vec<&str>, value: Value| {
        with(
            json!({
                "Carrier": {
                    "kind": "struct", "name": ["carrier"], "direction": "both",
                    "openness": {"preserve_typed": {"value": value}},
                    "fields": fields,
                    "flattened": flattened.iter().map(|id| json!({"ref": id})).collect::<Vec<_>>()
                }
            }),
            json!({
                "Nested": struct_type("nested", vec![field("deep", json!({"ref": "Value"}), "optional")], vec![], json!("reject")),
                "Value": {"kind": "newtype", "name": ["value"], "direction": "both", "inner": string()}
            }),
        )
    };
    let number = json!({"kind": "int", "bits": 32, "signed": false, "wire": "number"});

    for accepted in [
        openness(
            vec![field("kind", string(), "required")],
            vec![],
            json!({"kind": "map", "key": string(), "value": number}),
        ),
        openness(
            vec![field("count", number.clone(), "required")],
            vec![],
            json!({"kind": "map", "key": string(), "value": string()}),
        ),
        openness(vec![], vec!["Nested"], string()),
        openness(vec![], vec!["Nested"], number.clone()),
        openness(vec![], vec![], string()),
    ] {
        contract(accepted).validate().unwrap();
    }

    refuses(
        &contract(openness(
            vec![],
            vec![],
            json!({"kind": "map", "key": number, "value": number}),
        )),
        "map keys must use an inline string type",
    );
}

#[test]
fn a_typed_openness_value_spares_a_field_its_own_direction_drops() {
    let read_only = |wire: &str| {
        json!({"wire": wire, "ident": [wire], "ty": string(), "presence": "optional",
            "access": "read_only"})
    };
    let carrier = |direction: &str, hidden: Value| {
        contract(json!({
            "Nested": {
                "kind": "struct", "name": ["nested"], "direction": direction, "openness": "reject",
                "fields": [hidden]
            },
            "Carrier": struct_type(
                "carrier",
                vec![],
                vec![json!({"ref": "Nested"})],
                json!({"preserve_typed": {"value": {
                    "kind": "map", "key": string(),
                    "value": {"kind": "int", "bits": 32, "signed": false, "wire": "number"}
                }}})
            )
        }))
    };

    contract(with(
        json!({"Carrier": {
            "kind": "struct", "name": ["carrier"], "direction": "input", "openness": "reject",
            "fields": [read_only("kind")]
        }}),
        json!({}),
    ))
    .validate()
    .unwrap();
    carrier("input", read_only("kind")).validate().unwrap();
    carrier("output", read_only("kind")).validate().unwrap();
    carrier("input", field("kind", string(), "required"))
        .validate()
        .unwrap();
}

#[test]
fn a_typed_openness_value_meets_a_wide_integer_on_its_own_number_shape() {
    let carrier = |field_type: Value, extra: Value| {
        contract(json!({
            "Carrier": struct_type(
                "carrier",
                vec![field("count", field_type, "required")],
                vec![],
                json!({"preserve_typed": {"value": {
                    "kind": "map", "key": string(), "value": extra
                }}})
            )
        }))
    };
    let number = |bits| json!({"kind": "int", "bits": bits, "signed": false, "wire": "number"});

    for (field_type, extra) in [
        (number(64), number(32)),
        (number(64), number(64)),
        (number(32), number(64)),
        (json!({"kind": "float", "bits": 64}), number(16)),
        (
            json!({"kind": "int", "bits": 64, "signed": false, "wire": "string"}),
            string(),
        ),
    ] {
        carrier(field_type, extra).validate().unwrap();
    }
}

#[test]
fn a_flattened_member_is_a_struct_or_a_transparent_carrier_over_one() {
    let newtype_over = |name: &str, inner: Value| json!({"kind": "newtype", "name": [name], "direction": "both", "inner": inner});
    let opaque = json!({"kind": "opaque", "codec": "example", "wire": "unknown"});
    let flattened_document = |member: Value| {
        let mut document = request_body(json!({
            "Envelope": struct_type("envelope", vec![field("a", string(), "required")], vec![member], json!("reject")),
            "Seal": struct_type("seal", vec![field("b", string(), "required")], vec![], json!("reject")),
            "Deep": struct_type("deep", vec![], vec![json!({"ref": "Seal"})], json!("reject")),
            "Choice": {
                "kind": "enum", "name": ["choice"], "direction": "both",
                "repr": {"kind": "string"},
                "variants": [{"wire": "one", "ident": ["one"], "payload": {"kind": "unit"}}]
            }
        }));
        for (id, definition) in [
            ("Alias", newtype_over("alias", json!({"ref": "Seal"}))),
            ("Opaque", newtype_over("opaque", opaque.clone())),
            (
                "OpaqueAlias",
                newtype_over("opaque", json!({"ref": "Opaque"})),
            ),
            ("OverChoice", newtype_over("over", json!({"ref": "Choice"}))),
            (
                "OverMap",
                newtype_over(
                    "over",
                    json!({"kind": "map", "key": string(), "value": string()}),
                ),
            ),
        ] {
            document
                .types
                .insert(id.into(), serde_json::from_value(definition).unwrap());
        }
        document
    };

    for (member, blamed) in [
        (
            json!({"kind": "string"}),
            "flattened {\"kind\":\"string\"} is not a struct",
        ),
        (
            json!({"kind": "map", "key": string(), "value": string()}),
            "flattened {\"kind\":\"map\",\"key\":{\"kind\":\"string\"},\"value\":{\"kind\":\"string\"}} is not a struct",
        ),
        (
            json!({"kind": "array", "item": string()}),
            "flattened {\"kind\":\"array\",\"item\":{\"kind\":\"string\"}} is not a struct",
        ),
        (
            json!({"kind": "tuple", "items": [string()]}),
            "flattened {\"kind\":\"tuple\",\"items\":[{\"kind\":\"string\"}]} is not a struct",
        ),
        (
            json!({"ref": "Choice", "variant": "one"}),
            "flattened {\"ref\":\"Choice\",\"variant\":\"one\"} is not a struct",
        ),
        (
            json!({"ref": "Choice"}),
            "flattened {\"ref\":\"Choice\"} is not a struct",
        ),
        (
            json!({"ref": "OverChoice"}),
            "flattened {\"ref\":\"OverChoice\"} is not a struct",
        ),
        (
            json!({"ref": "OverChoice", "variant": "one"}),
            "flattened {\"ref\":\"OverChoice\",\"variant\":\"one\"} is not a struct",
        ),
        (
            json!({"ref": "OverMap"}),
            "flattened {\"ref\":\"OverMap\"} is not a struct",
        ),
        (
            json!({"ref": "Seal", "variant": "one"}),
            "flattened {\"ref\":\"Seal\",\"variant\":\"one\"} is not a struct",
        ),
        (
            json!({"kind": "nullable", "inner": {"ref": "Seal"}}),
            "nullable flattened types are not representable",
        ),
    ] {
        refuses(&flattened_document(member), blamed);
    }

    for member in [
        json!({"ref": "Seal"}),
        json!({"ref": "Alias"}),
        json!({"ref": "Deep"}),
        json!({"ref": "Opaque"}),
        json!({"ref": "OpaqueAlias"}),
        opaque.clone(),
    ] {
        flattened_document(member).validate().unwrap();
    }
}

#[test]
fn a_flatten_cycle_is_refused() {
    let cycles = |blamed: &str| {
        format!("the type {blamed} cannot be inlined because flattened members form a cycle")
    };

    for (types, blamed) in [
        (
            json!({
                "Envelope": struct_type("envelope", vec![], vec![json!({"ref": "Alias"})], json!("reject")),
                "Alias": {"kind": "newtype", "name": ["alias"], "direction": "both", "inner": {"ref": "Inner"}},
                "Inner": struct_type("inner", vec![field("b", string(), "required")], vec![json!({"ref": "Alias"})], json!("reject"))
            }),
            "Alias",
        ),
        (
            json!({
                "Envelope": struct_type("envelope", vec![], vec![json!({"ref": "Outer"})], json!("reject")),
                "Outer": struct_type("outer", vec![], vec![json!({"ref": "Envelope"})], json!("reject"))
            }),
            "Envelope",
        ),
        (
            json!({
                "Envelope": struct_type("envelope", vec![], vec![json!({"ref": "Envelope"})], json!("reject"))
            }),
            "Envelope",
        ),
    ] {
        refuses(&request_body(types), &cycles(blamed));
    }

    contract(json!({
        "Loop": struct_type("loop",
            vec![field("again", json!({"kind": "array", "item": {"ref": "Loop"}}), "optional")],
            vec![], json!("reject")),
        "Alias": {"kind": "newtype", "name": ["alias"], "direction": "both", "inner": {"ref": "Seal"}},
        "Seal": struct_type("seal", vec![field("b", string(), "required")], vec![], json!("reject")),
        "Nested": struct_type("nested", vec![], vec![json!({"ref": "Seal"})], json!("reject"))
    }))
    .validate()
    .unwrap();
}

#[test]
fn a_newtype_cycle_is_refused() {
    let cyclic = |inner: &str, other: &str| {
        let mut types = json!({
            "Left": {"kind": "newtype", "name": ["left"], "direction": "both", "inner": {"ref": inner}},
            "Right": {"kind": "newtype", "name": ["right"], "direction": "both", "inner": {"ref": other}}
        });
        if inner == other {
            types.as_object_mut().unwrap().remove("Right");
        }
        contract(types)
    };

    refuses(
        &cyclic("Left", "Left"),
        "the type Left cannot be resolved because newtypes form a cycle",
    );
    refuses(
        &cyclic("Right", "Left"),
        "the type Left cannot be resolved because newtypes form a cycle",
    );
    contract(json!({
        "Left": {"kind": "newtype", "name": ["left"], "direction": "both",
            "inner": {"kind": "nullable", "inner": {"ref": "Right"}}},
        "Right": {"kind": "newtype", "name": ["right"], "direction": "both", "inner": {"kind": "string"}}
    }))
    .validate()
    .unwrap();
}

#[test]
fn a_required_field_cannot_carry_a_default() {
    let field_with = |presence: &str, access: Option<&str>, default: Value| {
        let mut field = field("email", string(), presence);
        if let Some(access) = access {
            field["access"] = json!(access);
        }
        field["default"] = default;
        json!({"Text": {
            "kind": "struct", "name": ["text"], "direction": "both", "openness": "reject",
            "fields": [field]
        }})
    };
    let message = "the required field email cannot carry a default, because its key is always on the wire and only a side that drops the key is filled by one";

    refuses(
        &contract(field_with("required", None, json!("a@b.test"))),
        message,
    );
    refuses(
        &contract(field_with(
            "required",
            Some("read_write"),
            json!("a@b.test"),
        )),
        message,
    );
    for accepted in [
        field_with("optional", None, json!("a@b.test")),
        field_with("required", Some("read_only"), json!("a@b.test")),
        field_with("required", Some("write_only"), json!("a@b.test")),
        field_with("required", None, json!(null)).clone(),
    ] {
        if accepted["Text"]["fields"][0]["default"].is_null() {
            let mut without = accepted.clone();
            without["Text"]["fields"][0]
                .as_object_mut()
                .unwrap()
                .remove("default");
            contract(without).validate().unwrap();
            continue;
        }
        contract(accepted).validate().unwrap();
    }
}

#[test]
fn rejects_nullable_flattened_types() {
    let mut document: Document =
        serde_json::from_str(include_str!("../../../../spec/fixtures/login.json")).unwrap();
    let TypeKind::Struct { flattened, .. } =
        &mut document.types.get_mut("User").unwrap().definition
    else {
        panic!()
    };
    flattened.push(TypeRef::Inline(InlineType::Nullable {
        inner: Box::new(TypeRef::Inline(InlineType::Bool)),
    }));
    assert!(
        document
            .validate()
            .unwrap_err()
            .iter()
            .any(|error| error.to_string().contains("nullable flattened"))
    );
}

#[test]
fn object_openness_is_explicit_and_typed_references_are_validated() {
    let mut document: Document =
        serde_json::from_str(include_str!("../../../../spec/fixtures/login.json")).unwrap();
    let user = document.types.get_mut("User").unwrap();
    let TypeKind::Struct {
        fields, flattened, ..
    } = &user.definition
    else {
        unreachable!()
    };
    user.definition = TypeKind::Struct {
        fields: fields.clone(),
        flattened: flattened.clone(),
        openness: ObjectOpenness::PreserveTyped {
            value: TypeRef::Named(NamedRef {
                id: "Missing".into(),
                variant: None,
            }),
        },
    };
    let errors = document.validate().unwrap_err();
    assert!(
        errors
            .iter()
            .any(|error| error.to_string().contains("Missing"))
    );
    let encoded = serde_json::to_value(&document).unwrap();
    assert_eq!(
        encoded["types"]["User"]["openness"]["preserve_typed"]["value"]["ref"],
        "Missing"
    );
}
