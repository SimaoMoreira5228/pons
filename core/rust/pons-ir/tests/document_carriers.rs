mod support;

use pons_ir::{
    Direction, Document, InlineType, NamedRef, PathSegment, Presence, TypeDef, TypeKind, TypeRef,
};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use support::{
    CARRIERS, access_carrier, agreed, carried_by_every_carrier, carrier_document, carrier_struct,
    contract, field, fixture, refused_by_every_carrier, refuses, string, verdict, with,
};

#[test]
fn a_read_only_query_field_is_refused() {
    let query_document = |access: Option<&str>| {
        let mut field = json!({
            "wire": "tenant", "ident": ["tenant"], "ty": {"kind": "string"}, "presence": "required"
        });
        if let Some(access) = access {
            field["access"] = json!(access);
        }
        let mut document = fixture();
        let TypeKind::Struct { fields, .. } =
            &mut document.types.get_mut("EventQuery").unwrap().definition
        else {
            panic!("EventQuery is not a struct")
        };
        fields[0] = serde_json::from_value(field).unwrap();
        document
    };

    refuses(
        &query_document(Some("read_only")),
        "query field \"tenant\" is read_only, but an OpenAPI query parameter has no read/write direction",
    );
    for accepted in [None, Some("read_write"), Some("write_only")] {
        query_document(accepted).validate().unwrap();
    }

    let mut flattened = query_document(None);
    flattened.types.insert(
        "Page".into(),
        serde_json::from_value(json!({
            "name": ["page"], "direction": "input", "kind": "struct", "openness": "reject",
            "fields": [{"wire": "tenant", "ident": ["tenant"], "ty": {"kind": "string"}, "presence": "required", "access": "read_only"}]
        }))
        .unwrap(),
    );
    let TypeKind::Struct {
        flattened: members, ..
    } = &mut flattened.types.get_mut("EventQuery").unwrap().definition
    else {
        panic!("EventQuery is not a struct")
    };
    *members = vec![TypeRef::Named(NamedRef {
        id: "Page".into(),
        variant: None,
    })];
    refuses(&flattened, "query field \"tenant\" is read_only");
}

#[test]
fn a_required_read_only_field_is_refused_in_every_flat_input_carrier() {
    let carrier_document = |carrier: &str, presence: &str| {
        let mut document = fixture();
        document.types.insert(
            "Carrier".into(),
            serde_json::from_value(json!({
                "name": ["carrier"], "direction": "input", "kind": "struct", "openness": "reject",
                "fields": [{"wire": "tenant", "ident": ["tenant"], "ty": {"kind": "string"}, "presence": presence, "access": "read_only"}]
            }))
            .unwrap(),
        );
        let carrier_type = TypeRef::Named(NamedRef {
            id: "Carrier".into(),
            variant: None,
        });
        match carrier {
            "query" => document.operations[1].query = Some(carrier_type),
            "form" => {
                document.operations[0].body = Some(
                    serde_json::from_value(json!({
                        "ty": {"ref": "Carrier"}, "content": "form", "required": true
                    }))
                    .unwrap(),
                )
            }
            _ => {
                let PathSegment::Param { ty, .. } = &mut document.operations[1].path[1] else {
                    panic!("events.watch reads a path parameter")
                };
                *ty = carrier_type;
            }
        }
        document
    };

    for (carrier, message) in [
        (
            "query",
            "query field \"tenant\" is read_only, but an OpenAPI query parameter has no read/write direction",
        ),
        (
            "form",
            "form body field \"tenant\" is read_only, but a urlencoded form field has no read/write direction",
        ),
        (
            "path",
            "path parameter \"tenant\" is read_only, but a path segment has no read/write direction",
        ),
    ] {
        refuses(&carrier_document(carrier, "required"), message);
        carrier_document(carrier, "optional").validate().unwrap();
    }
    fixture().validate().unwrap();
}

#[test]
fn a_field_the_input_side_drops_and_still_requires_is_refused_where_every_carrier_refuses_it() {
    let tail = "the client drops the field on encode and every server built from the document waits for a key the client cannot send";

    for position in ["json", "query", "form", "path"] {
        let found = verdict(&carrier_document(
            access_carrier("input", "required", "read_only", None),
            position,
        ));
        assert!(
            found
                .iter()
                .any(|error| error.contains("tenant") && error.contains(tail)),
            "the {position} carrier must refuse a body no request can answer: {found:?}"
        );
        for legal in [
            access_carrier("input", "required", "read_only", Some(json!("acme"))),
            access_carrier("input", "optional", "read_only", None),
            access_carrier("input", "required", "write_only", None),
            access_carrier("input", "required", "read_write", None),
        ] {
            carrier_document(legal, position)
                .validate()
                .unwrap_or_else(|errors| panic!("{position} refused {errors:?}"));
        }
    }
    refuses(
        &contract(access_carrier("both", "required", "read_only", None)),
        tail,
    );
    carrier_document(
        access_carrier("both", "required", "read_only", Some(json!("acme"))),
        "json",
    )
    .validate()
    .unwrap();
}

#[test]
fn a_field_the_output_side_drops_and_still_requires_is_the_same_rule_the_other_way() {
    let tail = "the server drops the field on decode and every client built from the document waits for a field the server cannot send";

    for direction in ["output", "both"] {
        let found = verdict(&carrier_document(
            access_carrier(direction, "required", "write_only", None),
            "response",
        ));
        assert!(
            found
                .iter()
                .any(|error| error.contains("tenant") && error.contains(tail)),
            "a {direction} response must refuse a body no response can answer: {found:?}"
        );
        carrier_document(
            access_carrier(direction, "required", "write_only", Some(json!("acme"))),
            "response",
        )
        .validate()
        .unwrap();
        carrier_document(
            access_carrier(direction, "optional", "write_only", None),
            "response",
        )
        .validate()
        .unwrap();
    }
    carrier_document(
        access_carrier("output", "required", "read_only", None),
        "response",
    )
    .validate()
    .unwrap();
    carrier_document(
        access_carrier("input", "required", "write_only", None),
        "json",
    )
    .validate()
    .unwrap();
}

#[test]
fn a_named_newtype_over_a_nullable_is_the_field_the_inline_one_is() {
    let nullable = json!({"kind": "nullable", "inner": string()});
    let types = |inner: Value, presence: &str| {
        json!({
            "MaybeName": {
                "kind": "newtype", "name": ["maybe", "name"], "direction": "input", "inner": inner
            },
            "Carrier": {
                "kind": "struct", "name": ["carrier"], "direction": "input", "openness": "reject",
                "fields": [field("name", json!({"ref": "MaybeName"}), presence)]
            }
        })
    };

    refused_by_every_carrier(types(nullable.clone(), "required"), "name");
    carried_by_every_carrier(types(nullable.clone(), "optional"));

    let inline = |presence: &str| {
        json!({
            "Carrier": {
                "kind": "struct", "name": ["carrier"], "direction": "input", "openness": "reject",
                "fields": [field("name", nullable.clone(), presence)]
            }
        })
    };
    refused_by_every_carrier(inline("required"), "name");
    carried_by_every_carrier(inline("optional"));
}

#[test]
fn a_form_body_is_judged_by_the_record_rule_the_query_uses() {
    for openness in [
        json!("reject"),
        json!({"preserve_typed": {"value": string()}}),
        json!("preserve_opaque"),
    ] {
        let types = carrier_struct(
            openness.clone(),
            vec![field("a", string(), "optional")],
            vec![],
        );
        if openness == json!("reject") {
            carried_by_every_carrier(types);
        } else {
            refused_by_every_carrier(types, "Carrier");
        }
    }

    refused_by_every_carrier(
        with(
            carrier_struct(
                json!("reject"),
                vec![field("b", string(), "optional")],
                vec!["Member"],
            ),
            json!({
                "Member": {
                    "kind": "struct", "name": ["member"], "direction": "input",
                    "openness": "preserve_opaque",
                    "fields": [field("a", string(), "optional")]
                }
            }),
        ),
        "Member",
    );
    refused_by_every_carrier(
        with(
            carrier_struct(
                json!("reject"),
                vec![field("a", string(), "optional")],
                vec!["Member"],
            ),
            json!({
                "Nested": {
                    "kind": "struct", "name": ["nested"], "direction": "input", "openness": "reject",
                    "fields": [field("deep", string(), "optional")]
                },
                "Member": {
                    "kind": "struct", "name": ["member"], "direction": "input", "openness": "reject",
                    "fields": [field("deep", json!({"ref": "Nested"}), "optional")]
                }
            }),
        ),
        "deep",
    );

    for (top, path_carries) in [
        (
            json!({"kind": "map", "key": string(), "value": string()}),
            false,
        ),
        (json!({"kind": "array", "item": string()}), false),
        (
            json!({"kind": "opaque", "codec": "axum.form", "wire": "unknown"}),
            false,
        ),
        (string(), true),
    ] {
        let types = json!({"Carrier": {
            "kind": "newtype", "name": ["carrier"], "direction": "input", "inner": top
        }});
        for carrier in ["query", "form"] {
            let found = verdict(&carrier_document(types.clone(), carrier));
            assert_eq!(found.len(), 1, "one rule means one message: {found:?}");
            assert!(
                found[0].contains("struct") && found[0].contains(carrier),
                "the {carrier} carrier writes one key per field of a record: {found:?}"
            );
        }
        let found = verdict(&carrier_document(types, "path"));
        if path_carries {
            assert!(
                found.is_empty(),
                "a path segment spells a string: {found:?}"
            );
            continue;
        }
        assert_eq!(found.len(), 1, "one rule means one message: {found:?}");
        assert!(
            found[0].contains("path parameter \"carrier\""),
            "a path parameter that is not a record is one value it cannot spell: {found:?}"
        );
    }
    carried_by_every_carrier(json!({
        "Carrier": {
            "kind": "newtype", "name": ["carrier"], "direction": "input",
            "inner": {"ref": "CarrierBody"}
        },
        "CarrierBody": {
            "kind": "struct", "name": ["carrier", "body"], "direction": "input", "openness": "reject",
            "fields": [field("a", string(), "optional")]
        }
    }));
}

#[test]
fn a_required_nullable_field_is_refused_by_the_nullable_rule_in_every_carrier() {
    let types = |presence: &str| {
        json!({
            "Carrier": {
                "kind": "struct", "name": ["carrier"], "direction": "input", "openness": "reject",
                "fields": [field("email", json!({"kind": "nullable", "inner": string()}), presence)]
            }
        })
    };
    for (carrier, found) in CARRIERS.iter().zip(agreed(types("required"))) {
        assert_eq!(found.len(), 1, "{carrier} must refuse it once: {found:?}");
        assert!(
            found[0].contains("email")
                && found[0].contains("nullable")
                && found[0].contains(carrier),
            "{carrier} must blame the field that is required and nullable: {found:?}"
        );
        assert!(
            !found[0].contains("cannot carry nested objects"),
            "a scalar is not a nested object: {found:?}"
        );
    }
    carried_by_every_carrier(types("optional"));
}

#[test]
fn a_path_carrier_is_judged_by_the_value_and_duplicate_rules_the_query_uses() {
    let nested = json!({
        "Nested": {
            "kind": "struct", "name": ["nested"], "direction": "input", "openness": "reject",
            "fields": [field("deep", string(), "optional")]
        }
    });
    for member in [
        json!({"kind": "map", "key": string(), "value": string()}),
        json!({"ref": "Nested"}),
    ] {
        refused_by_every_carrier(
            with(
                carrier_struct(
                    json!("reject"),
                    vec![field("member", member, "optional")],
                    vec![],
                ),
                nested.clone(),
            ),
            "member",
        );
    }

    refused_by_every_carrier(
        with(
            carrier_struct(
                json!("reject"),
                vec![field(
                    "kind",
                    json!({"kind": "int", "bits": 32, "signed": false, "wire": "string"}),
                    "required",
                )],
                vec!["Seal"],
            ),
            json!({
                "Seal": {
                    "kind": "struct", "name": ["seal"], "direction": "input", "openness": "reject",
                    "fields": [field("kind", string(), "required")]
                }
            }),
        ),
        "kind",
    );
}

#[test]
fn a_default_fills_the_key_a_read_only_field_drops() {
    let types = |default: Option<Value>| {
        let mut read_only = field("tenant", string(), "required");
        read_only["access"] = json!("read_only");
        if let Some(default) = default {
            read_only["default"] = default;
        }
        carrier_struct(json!("reject"), vec![read_only], vec![])
    };

    refused_by_every_carrier(types(None), "tenant");
    carried_by_every_carrier(types(Some(json!("acme"))));

    let mut optional = field("tenant", string(), "optional");
    optional["access"] = json!("read_only");
    carried_by_every_carrier(carrier_struct(json!("reject"), vec![optional], vec![]));
}

#[test]
fn a_form_body_carries_one_array_level_because_a_query_and_a_path_carry_none() {
    let array = |levels: usize| {
        let mut item = string();
        for _ in 0..levels {
            item = json!({"kind": "array", "item": item});
        }
        carrier_struct(
            json!("reject"),
            vec![field("tags", item, "optional")],
            vec![],
        )
    };

    carrier_document(array(1), "form").validate().unwrap();
    for carrier in ["query", "path"] {
        let found = verdict(&carrier_document(array(1), carrier));
        assert_eq!(
            found.len(),
            1,
            "the {carrier} carrier writes one value per key: {found:?}"
        );
    }

    refused_by_every_carrier(array(2), "tags");
}

#[test]
fn a_nullable_field_axum_lowers_to_an_absent_key_is_a_field_every_carrier_carries() {
    carried_by_every_carrier(carrier_struct(
        json!("reject"),
        vec![field("email", string(), "optional")],
        vec![],
    ));

    refused_by_every_carrier(
        carrier_struct(
            json!("reject"),
            vec![field(
                "email",
                json!({"kind": "nullable", "inner": string()}),
                "required",
            )],
            vec![],
        ),
        "email",
    );
}

#[test]
fn a_request_header_the_runtime_owns_is_refused() {
    let with_header = |wire: &str| {
        let mut document = fixture();
        let operation = document.operations.last_mut().unwrap();
        operation.headers[0].wire = wire.into();
        document.operations[0].headers.clear();
        document
    };
    let message = "is owned by the request body or the security model, so every client and server built from the document writes it too";

    for wire in ["accept", "Accept", "authorization", "cookie"] {
        refuses(
            &with_header(wire),
            &format!("request header {wire} {message}"),
        );
    }
    with_header("content-type").validate().unwrap();
    with_header("X-Request-ID").validate().unwrap();

    let mut with_body = fixture();
    for wire in ["accept", "authorization", "content-type", "cookie"] {
        with_body.operations[0].headers =
            vec![serde_json::from_value(json!({
            "wire": wire, "ident": ["owned"], "ty": {"kind": "string"}, "presence": "required"
        }))
        .unwrap()];
        refuses(&with_body, &format!("request header {wire} {message}"));
    }

    let mut response = fixture();
    response.operations[0].responses.push(
        serde_json::from_value(json!({
            "status": 201, "content": "empty", "headers": [
                {"wire": "Set-Cookie", "ty": {"kind": "string"}, "presence": "required"},
                {"wire": "Content-Type", "ty": {"kind": "string"}, "presence": "required"}
            ]
        }))
        .unwrap(),
    );
    response.validate().unwrap();
}

#[test]
fn rejects_required_nullable_headers_and_path_parameters() {
    fn fixture() -> Document {
        serde_json::from_str(include_str!("../../../../spec/fixtures/login.json")).unwrap()
    }
    fn nullable() -> TypeRef {
        TypeRef::Inline(InlineType::Nullable {
            inner: Box::new(TypeRef::Inline(InlineType::Bool)),
        })
    }
    fn rejects(document: &Document, message: &str) {
        let errors = document
            .validate()
            .unwrap_err()
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>();
        assert!(
            errors.iter().any(|error| error.contains(message)),
            "{message:?} missing from {errors:?}"
        );
    }

    let mut document = fixture();
    let watch = &mut document.operations[1];
    watch.headers[0].presence = Presence::Required;
    watch.headers[0].ty = nullable();
    rejects(
        &document,
        "required request header X-Request-ID cannot be nullable",
    );

    let mut document = fixture();
    document.types.insert(
        "MaybeHeader".into(),
        TypeDef {
            name: vec!["maybe".into(), "header".into()],
            direction: Direction::Input,
            definition: TypeKind::Newtype { inner: nullable() },
            origin: None,
            doc: None,
            diagnostics: Vec::new(),
            ext: BTreeMap::new(),
        },
    );
    let watch = &mut document.operations[1];
    watch.headers[0].presence = Presence::Required;
    watch.headers[0].ty = TypeRef::Named(NamedRef {
        id: "MaybeHeader".into(),
        variant: None,
    });
    rejects(
        &document,
        "required request header X-Request-ID cannot be nullable",
    );

    let mut document = fixture();
    document.operations[1].path[1] = PathSegment::Param {
        param: "cursor".into(),
        ty: nullable(),
    };
    rejects(
        &document,
        "required path parameter cursor cannot be nullable",
    );

    let mut document = fixture();
    document.operations[1].headers[0].ty = nullable();
    document.validate().unwrap();
}
