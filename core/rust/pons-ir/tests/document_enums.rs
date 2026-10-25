mod support;

use pons_ir::TypeKind;
use serde_json::{Value, json};
use support::{contract, field, fixture, refuses, request_body, string, struct_type};

#[test]
fn every_narrowing_of_one_enum_is_checked_not_only_the_first() {
    let enum_type = json!({
        "Choice": {
            "kind": "enum", "name": ["choice"], "direction": "both",
            "repr": {"kind": "internal", "tag": "code"},
            "variants": [
                {"wire": "none", "ident": ["none"], "payload": {"kind": "unit"}},
                {"wire": "held", "ident": ["held"], "payload": {"kind": "newtype", "ty": {"ref": "Secret"}}}
            ]
        }
    });
    let narrowed = |first: &str, second: &str| {
        json!({
            "Envelope": {
                "kind": "struct", "name": ["envelope"], "direction": "input", "openness": "reject",
                "fields": [
                    {"wire": "first", "ident": ["first"], "ty": {"ref": "Choice", "variant": first}, "presence": "optional"},
                    {"wire": "second", "ident": ["second"], "ty": {"ref": "Choice", "variant": second}, "presence": "optional"}
                ]
            }
        })
    };
    let envelope = |first: &str, second: &str| {
        let mut types = json!({
            "Secret": {
                "kind": "struct", "name": ["secret"], "direction": "output", "openness": "reject",
                "fields": [{"wire": "token", "ident": ["token"], "ty": {"kind": "string"}, "presence": "required"}]
            }
        });
        types
            .as_object_mut()
            .unwrap()
            .extend(enum_type.as_object().unwrap().clone());
        types
            .as_object_mut()
            .unwrap()
            .extend(narrowed(first, second).as_object().unwrap().clone());
        request_body(types)
    };

    refuses(
        &envelope("none", "held"),
        "the request body needs a type of the input or both direction, but Secret is declared output",
    );
    refuses(
        &envelope("held", "none"),
        "the request body needs a type of the input or both direction, but Secret is declared output",
    );
    envelope("none", "none").validate().unwrap();
}

#[test]
fn an_internally_tagged_variant_must_carry_an_object_payload() {
    let choice = |repr: Value, payload: Value| {
        contract(json!({
            "Choice": {
                "kind": "enum", "name": ["choice"], "direction": "both", "repr": repr,
                "variants": [{"wire": "held", "ident": ["held"], "payload": payload}]
            }
        }))
    };
    let tuple = json!({"kind": "tuple", "items": [string(), string()]});

    refuses(
        &choice(json!({"kind": "internal", "tag": "code"}), tuple.clone()),
        "internally tagged enum variant held must have an object payload, because the tag shares one object with it",
    );
    for repr in [
        json!({"kind": "external"}),
        json!({"kind": "adjacent", "tag": "code", "content": "value"}),
        json!({"kind": "untagged"}),
    ] {
        choice(repr, tuple.clone())
            .validate()
            .unwrap_or_else(|errors| {
                panic!("{tuple:?} is legal beside a tag of its own: {errors:?}")
            });
    }
    choice(
        json!({"kind": "internal", "tag": "code"}),
        json!({"kind": "newtype", "ty": string()}),
    )
    .validate()
    .unwrap();
    choice(
        json!({"kind": "internal", "tag": "code"}),
        json!({"kind": "struct", "fields": [field("held", string(), "required")]}),
    )
    .validate()
    .unwrap();
}

#[test]
fn a_reserved_tag_name_is_refused_through_a_newtype_payload() {
    let with_payload = |payload: Value| {
        contract(json!({
            "Choice": {
                "kind": "enum", "name": ["choice"], "direction": "both",
                "repr": {"kind": "internal", "tag": "code"},
                "variants": [{"wire": "held", "ident": ["held"], "payload": payload}]
            },
            "Secret": {
                "kind": "struct", "name": ["secret"], "direction": "both", "openness": "reject",
                "fields": [{"wire": "code", "ident": ["code"], "ty": {"kind": "string"}, "presence": "required"}]
            },
            "Wrapped": {
                "kind": "newtype", "name": ["wrapped"], "direction": "both", "inner": {"ref": "Secret"}
            }
        }))
    };

    for held in [json!({"ref": "Secret"}), json!({"ref": "Wrapped"})] {
        refuses(
            &with_payload(json!({"kind": "newtype", "ty": held})),
            "variant held uses a reserved enum field",
        );
    }
    with_payload(json!({"kind": "newtype", "ty": {"kind": "string"}}))
        .validate()
        .unwrap();
}

#[test]
fn a_reserved_tag_reached_through_a_flattened_member_is_refused() {
    let enveloped = |payload: Value| {
        contract(json!({
            "Choice": {
                "kind": "enum", "name": ["choice"], "direction": "both",
                "repr": {"kind": "internal", "tag": "code"},
                "variants": [{"wire": "held", "ident": ["held"], "payload": payload}]
            },
            "Tagged": struct_type("tagged", vec![field("code", string(), "required")], vec![], json!("reject")),
            "Envelope": struct_type("envelope", vec![], vec![json!({"ref": "Tagged"})], json!("reject")),
            "Payload": {"kind": "newtype", "name": ["payload"], "direction": "both", "inner": {"ref": "Envelope"}}
        }))
    };

    refuses(
        &enveloped(json!({"kind": "newtype", "ty": {"ref": "Payload"}})),
        "variant held uses a reserved enum field",
    );
    enveloped(json!({"kind": "newtype", "ty": string()}))
        .validate()
        .unwrap();
}

#[test]
fn an_empty_enum_tag_is_refused() {
    let with_repr = |repr: Value| {
        let mut document = fixture();
        let definition = document.types.get_mut("LoginError").unwrap();
        let TypeKind::Enum { repr: found, .. } = &mut definition.definition else {
            panic!("LoginError is not an enum")
        };
        *found = serde_json::from_value(repr).unwrap();
        document
    };

    refuses(
        &with_repr(json!({"kind": "internal", "tag": ""})),
        "enum tag cannot be empty",
    );
    refuses(
        &with_repr(json!({"kind": "adjacent", "tag": "code", "content": ""})),
        "adjacent enum content cannot be empty",
    );
    with_repr(json!({"kind": "adjacent", "tag": "code", "content": "payload"}))
        .validate()
        .unwrap();
    with_repr(json!({"kind": "external"})).validate().unwrap();
}

#[test]
fn a_catch_all_variant_is_refused_as_the_only_variant_of_an_input_type() {
    let state = |direction: &str, variants: Value| {
        contract(json!({
            "State": {
                "kind": "enum", "name": ["state"], "direction": direction,
                "repr": {"kind": "string"},
                "variants": variants
            }
        }))
    };
    let catch_all = json!({
        "wire": "other", "ident": ["other"], "payload": {"kind": "unit"},
        "ext": {"com.pons.open_variant/v1": true}
    });
    let open = json!({"wire": "open", "ident": ["open"], "payload": {"kind": "unit"}});

    state("output", json!([catch_all.clone()]))
        .validate()
        .unwrap();
    for direction in ["input", "both"] {
        state(direction, json!([catch_all.clone(), open.clone()]))
            .validate()
            .unwrap();
        refuses(
            &state(direction, json!([catch_all.clone()])),
            "the only variant other is the com.pons.open_variant/v1 catch-all",
        );
    }
    refuses(
        &state(
            "input",
            json!([
                json!({
                    "wire": "other", "ident": ["other"],
                    "payload": {"kind": "newtype", "ty": {"kind": "string"}},
                    "ext": {"com.pons.open_variant/v1": true}
                }),
                open
            ]),
        ),
        "must have a unit payload",
    );
    state(
        "input",
        json!([json!({
            "wire": "other", "ident": ["other"], "payload": {"kind": "unit"},
            "ext": {"com.pons.open_variant/v1": false}
        })]),
    )
    .validate()
    .unwrap();
}
