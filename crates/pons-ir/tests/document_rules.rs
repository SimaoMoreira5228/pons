use pons_ir::{
    Direction, Document, ErrorStatus, NamedRef, PathSegment, TypeKind, TypeRef, Violation,
    validate_value,
};
use serde_json::{Value, json};

fn contract(types: Value) -> Document {
    serde_json::from_value(json!({
        "ir_version": "0.2",
        "info": {"title": "Example", "version": "1"},
        "types": types,
    }))
    .unwrap()
}

fn request_body(types: Value) -> Document {
    let mut document = contract(types);
    document.operations.push(
        serde_json::from_value(json!({
            "id": [["envelope"], ["send"]],
            "method": "POST",
            "path": [{"lit": "envelope"}],
            "body": {"ty": {"ref": "Envelope"}, "content": "json", "required": true},
            "responses": [{"status": 200, "content": "json", "body": {"kind": "string"}}],
            "auth": {"alternatives": [], "optional": false},
            "semantics": {"safe": false, "idempotent": false}
        }))
        .unwrap(),
    );
    document
}

fn fixture() -> Document {
    serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap()
}

fn rejections(document: &Document) -> Vec<String> {
    document
        .validate()
        .unwrap_err()
        .iter()
        .map(ToString::to_string)
        .collect()
}

fn refuses(document: &Document, message: &str) {
    let found = rejections(document);
    assert!(
        found.iter().any(|error| error.contains(message)),
        "{message:?} missing from {found:?}"
    );
}

fn field_type<'a>(document: &'a Document, id: &str, index: usize) -> &'a TypeRef {
    match &document.types[id].definition {
        TypeKind::Struct { fields, .. } => &fields[index].ty,
        other => panic!("{id} is not a struct: {other:?}"),
    }
}

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
fn a_common_error_class_and_the_exact_status_inside_it_both_dispatch() {
    let template = fixture();
    let with_common = |statuses: [ErrorStatus; 2]| {
        let mut document = template.clone();
        document.operations[0].errors.clear();
        let error = template.operations[0].errors[0].clone();
        for status in statuses {
            let mut common = error.clone();
            common.status = status;
            document.common_errors.push(common);
        }
        document
    };

    for dispatchable in [
        [ErrorStatus::Class(4), ErrorStatus::Exact(404)],
        [ErrorStatus::Default, ErrorStatus::Exact(418)],
        [ErrorStatus::Class(4), ErrorStatus::Class(5)],
    ] {
        with_common(dispatchable).validate().unwrap();
    }
    for (shadowing, claimed) in [
        (ErrorStatus::Class(4), ErrorStatus::Class(4)),
        (ErrorStatus::Default, ErrorStatus::Default),
    ] {
        refuses(
            &with_common([claimed.clone(), shadowing.clone()]),
            &format!(
                "error dispatch slot {claimed:?} is shadowed: the common error {shadowing:?} already answers for it"
            ),
        );
    }
}

#[test]
fn an_operation_error_outranks_a_common_error_that_answers_the_same_status() {
    let template = fixture();
    let document = |operation: ErrorStatus, common: ErrorStatus| {
        let mut document = template.clone();
        let error = &mut document.operations[0].errors[0];
        error.status = operation;
        let mut inherited = error.clone();
        inherited.status = common;
        inherited.name = vec!["common".into()];
        document.common_errors.push(inherited);
        document
    };

    document(ErrorStatus::Exact(404), ErrorStatus::Class(4))
        .validate()
        .unwrap();
    document(ErrorStatus::Exact(422), ErrorStatus::Default)
        .validate()
        .unwrap();
    refuses(
        &document(ErrorStatus::Class(4), ErrorStatus::Class(4)),
        "error dispatch slot Class(4) is inherited: the common error Class(4) already answers for it",
    );
    refuses(
        &document(ErrorStatus::Default, ErrorStatus::Default),
        "error dispatch slot Default is inherited: the common error Default already answers for it",
    );
}

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
fn a_query_parameter_and_a_request_header_cannot_name_an_api_key_carrier() {
    let with_carrier = |location: &str, name: &str, collision: &str| {
        let mut document = fixture();
        document.auth_schemes.remove("tenant");
        document.auth_schemes.insert(
            "bearer_key".into(),
            serde_json::from_value(json!({
                "kind": "api_key", "in": location, "name": name
            }))
            .unwrap(),
        );
        document.operations[1].auth.alternatives = vec![vec!["bearer_key".into()]];
        document.operations[1].headers[0].wire = collision.into();
        if location == "query" {
            let TypeKind::Struct { fields, .. } =
                &mut document.types.get_mut("EventQuery").unwrap().definition
            else {
                panic!("EventQuery is not a struct")
            };
            fields[0].wire = collision.into();
        }
        document
    };

    refuses(
        &with_carrier("header", "X-Tenant", "x-tenant"),
        "the request header x-tenant is also the bearer_key apiKey carrier name, so the credential overwrites it",
    );
    refuses(
        &with_carrier("query", "limit", "limit"),
        "the query field \"limit\" is also the bearer_key apiKey carrier name, so the credential overwrites it",
    );
    for accepted in [
        ("header", "X-Other", "X-Tenant"),
        ("query", "cursor", "limit"),
        ("cookie", "limit", "limit"),
        ("query", "Limit", "limit"),
    ] {
        with_carrier(accepted.0, accepted.1, accepted.2)
            .validate()
            .unwrap();
    }
}

#[test]
fn a_credential_carrier_collides_only_where_an_auth_group_names_its_scheme() {
    let with_scheme = |id: &str, scheme: Value, group: Option<usize>| {
        let mut document = fixture();
        document
            .auth_schemes
            .insert(id.into(), serde_json::from_value(scheme).unwrap());
        if let Some(index) = group {
            document.operations[index].auth.alternatives = vec![vec![id.into()]];
        }
        document
    };
    let header = |name: &str| json!({"kind": "api_key", "in": "header", "name": name});

    with_scheme("unused", header("X-Request-ID"), None)
        .validate()
        .unwrap();
    with_scheme("unused", header("X-Request-ID"), Some(0))
        .validate()
        .unwrap();
    refuses(
        &with_scheme("wanted", header("X-Request-ID"), Some(1)),
        "the request header X-Request-ID is also the wanted apiKey carrier name",
    );

    let same_carrier = |alternatives: Vec<Vec<&str>>| {
        let mut document = with_scheme("key", header("x-tenant"), None);
        document.auth_schemes.insert(
            "spare".into(),
            serde_json::from_value(header("X-Other")).unwrap(),
        );
        document.operations[1].auth.alternatives = alternatives
            .into_iter()
            .map(|group| group.into_iter().map(str::to_owned).collect())
            .collect();
        document
    };
    refuses(
        &same_carrier(vec![vec!["tenant", "key"]]),
        "the key and tenant schemes of one auth alternative both write the Header carrier \"x-tenant\"",
    );
    same_carrier(vec![vec!["tenant"], vec!["key"]])
        .validate()
        .unwrap();
    same_carrier(vec![vec!["tenant", "spare"]])
        .validate()
        .unwrap();

    let mut cookies = with_scheme(
        "sid",
        json!({"kind": "cookie_session", "cookie": "sid"}),
        None,
    );
    cookies.auth_schemes.insert(
        "key".into(),
        serde_json::from_value(json!({"kind": "api_key", "in": "cookie", "name": "sid"})).unwrap(),
    );
    cookies.operations[1].auth.alternatives = vec![vec!["key".into(), "sid".into()]];
    refuses(
        &cookies,
        "the sid and key schemes of one auth alternative both write the Cookie carrier \"sid\"",
    );
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
