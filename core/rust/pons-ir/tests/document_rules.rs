use pons_ir::{
    Diagnostic, Direction, Document, ErrorStatus, InlineType, NamedRef, PathSegment, TypeKind,
    TypeRef, Violation, validate_value,
};
use serde_json::{Value, json};
use std::collections::BTreeMap;

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
    serde_json::from_str(include_str!("../../../../spec/fixtures/login.json")).unwrap()
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

const CARRIERS: [&str; 3] = ["query", "form", "path"];

fn carrier_document(types: Value, carrier: &str) -> Document {
    let mut document = contract(types);
    let mut operation = json!({
        "id": [["events"], ["list"]],
        "method": "POST",
        "path": [{"lit": "events"}],
        "responses": [{"status": 200, "content": "json", "body": {"kind": "string"}}],
        "auth": {"alternatives": [], "optional": false},
        "semantics": {"safe": false, "idempotent": false}
    });
    match carrier {
        "query" => operation["query"] = json!({"ref": "Carrier"}),
        "form" => {
            operation["body"] =
                json!({"ty": {"ref": "Carrier"}, "content": "form", "required": true})
        }
        "json" => {
            operation["body"] =
                json!({"ty": {"ref": "Carrier"}, "content": "json", "required": true})
        }
        "response" => operation["responses"][0]["body"] = json!({"ref": "Carrier"}),
        _ => {
            operation["path"] = json!([
                {"lit": "events"},
                {"param": "carrier", "ty": {"ref": "Carrier"}}
            ])
        }
    }
    document
        .operations
        .push(serde_json::from_value(operation).unwrap());
    document
}

fn verdict(document: &Document) -> Vec<String> {
    match document.validate() {
        Ok(()) => Vec::new(),
        Err(errors) => errors.iter().map(ToString::to_string).collect(),
    }
}

fn verdicts(types: Value) -> Vec<Vec<String>> {
    CARRIERS
        .iter()
        .map(|carrier| verdict(&carrier_document(types.clone(), carrier)))
        .collect()
}

fn agreed(types: Value) -> Vec<Vec<String>> {
    let verdicts = verdicts(types);
    let refused = verdicts.iter().filter(|found| !found.is_empty()).count();
    assert!(
        refused == 0 || refused == CARRIERS.len(),
        "one rule means every flat carrier answers a shape the same way: {verdicts:?}"
    );
    verdicts
}

fn carried_by_every_carrier(types: Value) {
    for (carrier, found) in CARRIERS.iter().zip(agreed(types.clone())) {
        assert!(
            found.is_empty(),
            "the {carrier} carrier refused a shape every flat carrier carries: {found:?}"
        );
    }
}

fn refused_by_every_carrier(types: Value, blamed: &str) {
    for (carrier, found) in CARRIERS.iter().zip(agreed(types.clone())) {
        assert!(
            found
                .iter()
                .any(|message| message.contains(blamed) && message.contains(carrier)),
            "the {carrier} carrier must name what it refused: {found:?}"
        );
    }
}

fn field(wire: &str, ty: Value, presence: &str) -> Value {
    json!({"wire": wire, "ident": [wire], "ty": ty, "presence": presence})
}

fn string() -> Value {
    json!({"kind": "string"})
}

fn with(mut types: Value, extra: Value) -> Value {
    types
        .as_object_mut()
        .unwrap()
        .extend(extra.as_object().unwrap().clone());
    types
}

fn carrier_struct(openness: Value, fields: Vec<Value>, flattened: Vec<&str>) -> Value {
    json!({
        "Carrier": {
            "kind": "struct", "name": ["carrier"], "direction": "input", "openness": openness,
            "fields": fields,
            "flattened": flattened.iter().map(|id| json!({"ref": id})).collect::<Vec<_>>()
        }
    })
}

fn access_carrier(direction: &str, presence: &str, access: &str, default: Option<Value>) -> Value {
    let mut field = field("tenant", string(), presence);
    field["access"] = json!(access);
    if let Some(default) = default {
        field["default"] = default;
    }
    let mut types = carrier_struct(json!("reject"), vec![field], vec![]);
    types["Carrier"]["direction"] = json!(direction);
    types
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

fn struct_type(name: &str, fields: Vec<Value>, flattened: Vec<Value>, openness: Value) -> Value {
    json!({
        "kind": "struct", "name": [name], "direction": "both",
        "openness": openness, "fields": fields, "flattened": flattened
    })
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
fn a_declared_media_type_must_name_the_body_the_client_writes() {
    let body = |content: &str, media_type: &str, ty: Value| {
        let mut document = request_body(json!({
            "Envelope": struct_type("envelope", vec![field("email", string(), "required")], vec![], json!("reject")),
            "Blob": {"kind": "newtype", "name": ["blob"], "direction": "input",
                "inner": {"kind": "bytes", "encoding": "base64"}}
        }));
        let body = &mut document.operations[0].body.as_mut().unwrap();
        body.content = serde_json::from_value(json!(content)).unwrap();
        body.media = Some(pons_ir::MediaDescriptor::from_declared(media_type));
        body.ty = serde_json::from_value(ty).unwrap();
        document
    };
    let envelope = json!({"ref": "Envelope"});
    let blob = json!({"ref": "Blob"});

    for (content, media_type) in [
        ("form", "application/json"),
        ("multipart", "application/json"),
        ("text", "application/json"),
        ("text", "application/x-www-form-urlencoded"),
        ("bytes", "application/json"),
        ("bytes", "application/vnd.acme+json"),
        ("bytes", "multipart/form-data"),
    ] {
        refuses(
            &body(
                content,
                media_type,
                if content == "bytes" {
                    blob.clone()
                } else {
                    envelope.clone()
                },
            ),
            &format!("has media type {media_type}, which does not name the"),
        );
    }
    for (content, media_type, ty) in [
        ("json", "application/json", envelope.clone()),
        (
            "json",
            "application/vnd.acme+json; charset=utf-8",
            envelope.clone(),
        ),
        ("problem_json", "application/problem+json", envelope.clone()),
        (
            "form",
            "application/x-www-form-urlencoded",
            envelope.clone(),
        ),
        ("multipart", "multipart/form-data", envelope.clone()),
        ("text", "text/plain; charset=utf-8", envelope.clone()),
        ("text", "text/csv", envelope.clone()),
        ("bytes", "application/octet-stream", blob.clone()),
        ("bytes", "application/pdf; version=1", blob.clone()),
    ] {
        body(content, media_type, ty).validate().unwrap();
    }

    let mut response = fixture();
    response.operations[0].responses[0].content = pons_ir::ContentKind::Text;
    response.operations[0].responses[0].media =
        Some(pons_ir::MediaDescriptor::from_declared("application/json"));
    refuses(
        &response,
        "response 200 body has media type application/json, which does not name the text body the client writes",
    );
    let mut error = fixture();
    error.operations[0].errors[0].content = pons_ir::ErrorContent::Text;
    error.operations[0].errors[0].media =
        Some(pons_ir::MediaDescriptor::from_declared("application/json"));
    refuses(
        &error,
        "error body has media type application/json, which does not name the text body the client writes",
    );
}

#[test]
fn a_stream_request_body_is_refused() {
    let mut document = fixture();
    document.operations[0].body.as_mut().unwrap().content = pons_ir::ContentKind::Stream;
    refuses(
        &document,
        "a request body cannot be stream content, because no built-in target reads one",
    );
}

#[test]
fn a_default_error_collides_only_with_a_catch_all_success() {
    let with_statuses = |response: pons_ir::ResponseStatus, error: ErrorStatus| {
        let mut document = fixture();
        let declared = &mut document.operations[0].responses[0];
        declared.status = response;
        if matches!(response, pons_ir::ResponseStatus::Exact(status) if matches!(status, 100..=199 | 204 | 205 | 304))
        {
            declared.content = pons_ir::ContentKind::Empty;
            declared.body = None;
        }
        document.operations[0].errors[0].status = error;
        document
    };
    let exact = |status| pons_ir::ResponseStatus::Exact(status);
    let collides = "collides with a success response";

    refuses(
        &with_statuses(pons_ir::ResponseStatus::Default, ErrorStatus::Default),
        &format!("error status Default {collides}"),
    );
    refuses(
        &with_statuses(pons_ir::ResponseStatus::Unknown, ErrorStatus::Default),
        &format!("error status Default {collides}"),
    );
    refuses(
        &with_statuses(pons_ir::ResponseStatus::Default, ErrorStatus::Exact(200)),
        &format!("error status Exact(200) {collides}"),
    );
    refuses(
        &with_statuses(pons_ir::ResponseStatus::Default, ErrorStatus::Class(2)),
        &format!("error status Class(2) {collides}"),
    );

    for document in [
        with_statuses(exact(200), ErrorStatus::Default),
        with_statuses(exact(204), ErrorStatus::Default),
        with_statuses(exact(302), ErrorStatus::Default),
        with_statuses(exact(404), ErrorStatus::Default),
        with_statuses(pons_ir::ResponseStatus::Class(4), ErrorStatus::Default),
        with_statuses(pons_ir::ResponseStatus::Class(2), ErrorStatus::Default),
        with_statuses(pons_ir::ResponseStatus::Default, ErrorStatus::Exact(404)),
        with_statuses(exact(404), ErrorStatus::Class(5)),
    ] {
        document.validate().unwrap();
    }
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
fn an_operation_scope_the_flows_do_not_declare_is_refused() {
    let oauth = |scopes: Value| {
        let mut document = fixture();
        document.auth_schemes.insert(
            "oauth".into(),
            serde_json::from_value(json!({
                "kind": "o_auth2",
                "flows": [{
                    "kind": "client_credentials",
                    "token_url": "https://auth.example.test/token",
                    "scopes": scopes
                }]
            }))
            .unwrap(),
        );
        document.operations[1].auth.alternatives = vec![vec!["oauth".into()]];
        document.operations[1].auth.scopes =
            BTreeMap::from([("oauth".into(), vec!["read".into()])]);
        document
    };

    oauth(json!({"read": "Read", "write": "Write"}))
        .validate()
        .unwrap();
    refuses(
        &oauth(json!({"write": "Write"})),
        "the OAuth2 scheme oauth does not declare the read scope",
    );
}

#[test]
fn an_oauth_flow_url_must_be_an_absolute_https_url() {
    let flow = |kind: &str, url: &str| {
        let mut document = fixture();
        document.auth_schemes.insert(
            "oauth".into(),
            serde_json::from_value(json!({
                "kind": "o_auth2",
                "flows": [{ "kind": kind, "authorization_url": url, "token_url": url }]
            }))
            .unwrap(),
        );
        document
    };

    for url in [
        "https://auth.example.test/authorize",
        "https://auth.example.test/authorize?audience=api",
        "https://user@auth.example.test/authorize",
    ] {
        flow("authorization_code", url).validate().unwrap();
    }
    for url in [
        "/authorize",
        "http://auth.example.test/authorize",
        "https://",
        "",
    ] {
        refuses(
            &flow("authorization_code", url),
            "OAuth2 authorization URL for oauth must be an absolute HTTPS URL",
        );
        refuses(
            &flow("client_credentials", url),
            "OAuth2 token URL for oauth must be an absolute HTTPS URL",
        );
    }
}

#[test]
fn scopes_are_legal_on_every_auth_scheme() {
    let with_scopes = |schemes: Value, scopes: &[&str]| {
        let mut document = fixture();
        document
            .auth_schemes
            .extend(serde_json::from_value::<BTreeMap<String, _>>(schemes).unwrap());
        document.operations[1].auth.alternatives = vec![vec!["tenant".into()]];
        document.operations[1].auth.scopes = BTreeMap::from([(
            "tenant".to_owned(),
            scopes.iter().map(|scope| (*scope).to_owned()).collect(),
        )]);
        document
    };
    let oauth = |scopes: Value| {
        json!({
            "tenant": {
                "kind": "o_auth2",
                "flows": [{
                    "kind": "client_credentials",
                    "token_url": "https://auth.example.test/token",
                    "scopes": scopes
                }]
            }
        })
    };

    for scheme in [
        json!({"tenant": {"kind": "bearer", "format": "jwt"}}),
        json!({"tenant": {"kind": "api_key", "in": "header", "name": "X-Tenant"}}),
        json!({"tenant": {"kind": "custom", "description": "per tenant"}}),
        oauth(json!({"read": "Read"})),
    ] {
        with_scopes(scheme, &["read"]).validate().unwrap();
    }

    refuses(
        &with_scopes(oauth(json!({"write": "Write"})), &["read"]),
        "the OAuth2 scheme tenant does not declare the read scope",
    );
    refuses(
        &with_scopes(
            json!({"tenant": {"kind": "bearer", "format": "jwt"}}),
            &["read", "read"],
        ),
        "auth scheme tenant repeats scope read",
    );
    refuses(
        &with_scopes(json!({"tenant": {"kind": "bearer", "format": "jwt"}}), &[]),
        "auth scheme tenant has an empty scope list",
    );
}

#[test]
fn an_empty_error_name_is_refused() {
    for name in [vec![], vec!["bad".into(), String::new()]] {
        let mut document = fixture();
        document.operations[0].errors[0].name = name;
        refuses(&document, "error name must contain non-empty name segments");
    }
    fixture().validate().unwrap();
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
fn a_websocket_message_needs_the_direction_the_operation_uses() {
    let socket = |client: &str, server: &str| {
        let mut document = contract(json!({
            "Sent": {"kind": "struct", "name": ["sent"], "direction": "input", "openness": "reject",
                "fields": [field("a", string(), "required")]},
            "Received": {"kind": "struct", "name": ["received"], "direction": "output", "openness": "reject",
                "fields": [field("b", string(), "required")]}
        }));
        document.operations.push(
            serde_json::from_value(json!({
                "id": [["socket"]],
                "method": "GET",
                "path": [{"lit": "socket"}],
                "responses": [{"status": 200, "content": "stream"}],
                "stream": {"kind": "websocket", "client": {"ref": client}, "server": {"ref": server}},
                "auth": {"alternatives": [], "optional": false},
                "semantics": {"safe": true, "idempotent": true}
            }))
            .unwrap(),
        );
        document
    };

    socket("Sent", "Received").validate().unwrap();
    refuses(
        &socket("Received", "Received"),
        "the websocket client message needs a type of the input or both direction, but Received is declared output",
    );
    refuses(
        &socket("Sent", "Sent"),
        "the websocket server message needs a type of the output or both direction, but Sent is declared input",
    );
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

#[test]
fn extension_keys_carry_a_reverse_dns_namespace_and_a_version() {
    let with_key = |key: &str| {
        let mut document = fixture();
        document
            .ext
            .insert(key.to_owned(), serde_json::json!({"kept": true}));
        document
    };

    for key in [
        "com.example.cursor/v1",
        "test.example.source/v1",
        "a.b-c.d-e/v0",
        "x.y1.z1/v12",
        "com.pons.alias/v1",
        "com.pons.open_variant/v1",
    ] {
        with_key(key).validate().unwrap_or_else(|errors| {
            panic!("{key} should be accepted: {errors:?}");
        });
    }

    for (key, message) in [
        ("source.note", "is not shaped reverse-dns.namespace/vN"),
        ("com.example/v1", "is not shaped reverse-dns.namespace/vN"),
        (
            "com.example.cursor",
            "is not shaped reverse-dns.namespace/vN",
        ),
        (
            "com.example.cursor/extra/v1",
            "is not shaped reverse-dns.namespace/vN",
        ),
        (
            "com.example.cursor/v",
            "is not shaped reverse-dns.namespace/vN",
        ),
        (
            "com.example.cursor/v01",
            "is not shaped reverse-dns.namespace/vN",
        ),
        (
            "com.example.cursor/1",
            "is not shaped reverse-dns.namespace/vN",
        ),
        ("com.example./v1", "is not shaped reverse-dns.namespace/vN"),
        ("comexample/v1", "is not shaped reverse-dns.namespace/vN"),
        (
            "Com.Example/cursor/v1",
            "is not shaped reverse-dns.namespace/vN",
        ),
        (
            "com.example.Cursor/v1",
            "is not shaped reverse-dns.namespace/vN",
        ),
        (
            "com.example.curs-or/v1/x",
            "is not shaped reverse-dns.namespace/vN",
        ),
        (
            "x-pons.internal/cursor/v1",
            "starts with \"x-pons\", which this project reserves for itself",
        ),
        (
            "x-pons",
            "starts with \"x-pons\", which this project reserves for itself",
        ),
    ] {
        refuses(&with_key(key), message);
    }
}

#[test]
fn every_object_carrying_an_extension_names_its_owner() {
    let mut document = fixture();
    let bad = |key: &str| BTreeMap::from([(key.to_owned(), serde_json::json!(1))]);
    document.ext = bad("note");
    document.info.ext = bad("note");
    document.types.get_mut("User").unwrap().ext = bad("note");
    document.auth_schemes.insert(
        "custom".into(),
        pons_ir::AuthScheme::Custom {
            description: "d".into(),
            adapter: None,
            ext: bad("note"),
        },
    );
    document.operations[0].ext = bad("note");
    document.operations[0].auth.ext = bad("note");
    document.operations[0].body.as_mut().unwrap().ext = bad("note");
    document.operations[0].provides.as_mut().unwrap().ext = bad("note");
    document.operations[0].errors[0].ext = bad("note");
    document.operations[0].responses[0].ext = bad("note");
    document.operations[1].headers[0].ext = bad("note");
    document
        .common_errors
        .push(document.operations[0].errors[0].clone());
    document.diagnostics.push(Diagnostic {
        severity: pons_ir::DiagnosticSeverity::Info,
        code: "C".into(),
        message: "m".into(),
        origin: None,
        fidelity: None,
        capability: None,
        ext: bad("note"),
    });

    let found = rejections(&document);
    for owner in [
        "document:",
        "info:",
        "type User:",
        "auth scheme custom:",
        "operations[0]:",
        "operations[0].auth:",
        "operations[0].body:",
        "operations[0].provides:",
        "operations[0].errors[0]:",
        "operations[0].responses[0]:",
        "operations[1].headers[0]:",
        "common_errors[0]:",
        "document diagnostics[0]:",
    ] {
        assert!(
            found
                .iter()
                .any(|error| error.starts_with(&format!("{owner} extension key \"note\""))),
            "{owner} did not report the key: {found:?}"
        );
    }
}

#[test]
fn an_opaque_value_carries_its_own_extensions() {
    let mut document = fixture();
    let TypeKind::Struct { fields, .. } = &mut document.types.get_mut("User").unwrap().definition
    else {
        panic!()
    };
    fields[0].ty = TypeRef::Inline(InlineType::Opaque {
        codec: "example".into(),
        wire: pons_ir::OpaqueWire::Json,
        ext: BTreeMap::from([("com.example.codec/v1".into(), serde_json::json!(true))]),
    });
    document.validate().unwrap();

    let raw = serde_json::to_value(&document).unwrap();
    assert_eq!(
        raw["types"]["User"]["fields"][0]["ty"]["ext"]["com.example.codec/v1"],
        serde_json::json!(true)
    );
    assert!(
        raw["types"]["User"]["fields"][0]["ty"]["ext"]
            .as_object()
            .is_some_and(|ext| !ext.is_empty())
    );

    let TypeKind::Struct { fields, .. } = &mut document.types.get_mut("User").unwrap().definition
    else {
        panic!()
    };
    let TypeRef::Inline(InlineType::Opaque { ext, .. }) = &mut fields[0].ty else {
        panic!()
    };
    ext.insert("note".into(), serde_json::json!(true));
    refuses(
        &document,
        "extension key \"note\" is not shaped reverse-dns.namespace/vN",
    );
}

#[test]
fn an_opaque_value_must_name_a_codec() {
    let mut document = fixture();
    let TypeKind::Struct { fields, .. } = &mut document.types.get_mut("User").unwrap().definition
    else {
        panic!()
    };
    fields[0].ty = TypeRef::Inline(InlineType::Opaque {
        codec: "   ".into(),
        wire: pons_ir::OpaqueWire::Json,
        ext: BTreeMap::new(),
    });
    refuses(&document, "opaque codec cannot be empty");
}

fn paginated(request: Value, pagination: Value) -> Document {
    serde_json::from_value(json!({
        "ir_version": "0.2",
        "info": {"title": "Example", "version": "1"},
        "types": {
            "EventQuery": {
                "kind": "struct", "name": ["event", "query"], "direction": "input", "openness": "reject",
                "fields": [
                    {"wire": "cursor", "ident": ["cursor"], "ty": {"kind": "string"}, "presence": "optional"},
                    {"wire": "offset", "ident": ["offset"], "ty": {"kind": "int", "bits": 32, "signed": false, "wire": "number"}, "presence": "optional"},
                    {"wire": "limit", "ident": ["limit"], "ty": {"kind": "int", "bits": 16, "signed": false, "wire": "number"}, "presence": "optional"}
                ]
            },
            "EventPage": {
                "kind": "struct", "name": ["event", "page"], "direction": "output", "openness": "reject",
                "fields": [
                    {"wire": "items", "ident": ["items"], "ty": {"kind": "array", "item": {"kind": "string"}}, "presence": "required"},
                    {"wire": "next", "ident": ["next"], "ty": {"kind": "nullable", "inner": {"kind": "string"}}, "presence": "optional"}
                ]
            }
        },
        "operations": [{
            "id": [["events"], ["list"]],
            "method": "GET",
            "path": [{"lit": "events"}],
            "query": request,
            "responses": [{"status": 200, "content": "json", "body": {"ref": "EventPage"}}],
            "auth": {"alternatives": [], "optional": false},
            "semantics": {"safe": true, "idempotent": true, "pagination": pagination}
        }]
    }))
    .unwrap()
}

#[test]
fn a_cursor_page_records_its_request_field_and_locators() {
    let cursor = paginated(
        json!({"ref": "EventQuery"}),
        json!({
            "request": {"kind": "cursor", "field": "cursor"},
            "items": "/items",
            "next": "/next",
            "page_size": {"field": "limit", "default": 20, "maximum": 100}
        }),
    );
    cursor.validate().unwrap();

    let offset = paginated(
        json!({"ref": "EventQuery"}),
        json!({
            "request": {"kind": "offset", "field": "offset"},
            "items": "/items"
        }),
    );
    offset.validate().unwrap();
}

#[test]
fn a_page_fields_must_name_the_operation_query_or_body() {
    let missing = paginated(
        json!({"ref": "EventQuery"}),
        json!({
            "request": {"kind": "cursor", "field": "after"},
            "items": "/items"
        }),
    );
    refuses(
        &missing,
        "pagination field \"after\" is not declared on the operation query or body",
    );

    let missing_size = paginated(
        json!({"ref": "EventQuery"}),
        json!({
            "request": {"kind": "offset", "field": "offset"},
            "items": "/items",
            "page_size": {"field": "per_page"}
        }),
    );
    refuses(
        &missing_size,
        "pagination field \"per_page\" is not declared on the operation query or body",
    );
}

#[test]
fn pagination_locators_are_json_pointers() {
    let bad_items = paginated(
        json!({"ref": "EventQuery"}),
        json!({
            "request": {"kind": "cursor", "field": "cursor"},
            "items": "items"
        }),
    );
    refuses(
        &bad_items,
        "pagination item locator: a JSON pointer must be empty or start with '/'",
    );

    let bad_next = paginated(
        json!({"ref": "EventQuery"}),
        json!({
            "request": {"kind": "cursor", "field": "cursor"},
            "items": "/items",
            "next": "/next~2"
        }),
    );
    refuses(
        &bad_next,
        "pagination next locator: a JSON pointer has an invalid escape",
    );

    let empty = paginated(
        json!({"ref": "EventQuery"}),
        json!({
            "request": {"kind": "cursor", "field": "cursor"},
            "items": ""
        }),
    );
    empty.validate().unwrap();
}

#[test]
fn a_page_size_default_cannot_exceed_its_maximum() {
    let invalid = paginated(
        json!({"ref": "EventQuery"}),
        json!({
            "request": {"kind": "offset", "field": "offset"},
            "items": "/items",
            "page_size": {"field": "limit", "default": 50, "maximum": 10}
        }),
    );
    refuses(
        &invalid,
        "pagination page size default 50 exceeds its maximum 10",
    );

    let zero = paginated(
        json!({"ref": "EventQuery"}),
        json!({
            "request": {"kind": "offset", "field": "offset"},
            "items": "/items",
            "page_size": {"field": "limit", "default": 0}
        }),
    );
    refuses(&zero, "pagination page size default must be positive");
}

#[test]
fn a_pagination_field_can_live_on_the_request_body() {
    let mut document = paginated(
        json!({"ref": "EventQuery"}),
        json!({
            "request": {"kind": "offset", "field": "offset"},
            "items": "/items"
        }),
    );
    {
        let operation = &mut document.operations[0];
        operation.method = pons_ir::Method::Post;
        operation.query = None;
        operation.body = Some(
            serde_json::from_value(json!({
                "ty": {"ref": "EventQuery"},
                "content": "json",
                "required": true
            }))
            .unwrap(),
        );
    }
    document.validate().unwrap();
}

#[test]
fn decoding_refuses_a_version_it_does_not_read_before_reading_anything_else() {
    let mut raw: Value = serde_json::from_str(fixture_json()).unwrap();
    raw["ir_version"] = json!("0.3");
    raw["operations"][0]["method"] = json!("BREW");
    let error = pons_ir::decode(&serde_json::to_vec(&raw).unwrap()).unwrap_err();
    assert_eq!(
        error.to_string(),
        "unsupported IR version 0.3, this build reads 0.2"
    );

    let mut raw: Value = serde_json::from_str(fixture_json()).unwrap();
    raw.as_object_mut().unwrap().remove("ir_version");
    let error = pons_ir::decode(&serde_json::to_vec(&raw).unwrap()).unwrap_err();
    assert_eq!(
        error.to_string(),
        "ir_version: the document does not carry a string ir_version"
    );

    let error = pons_ir::decode(b"{\"ir_version\": ").unwrap_err();
    assert!(error.to_string().starts_with("invalid JSON: "), "{error}");
}

#[test]
fn decoding_names_the_member_it_stopped_on() {
    let mut raw: Value = serde_json::from_str(fixture_json()).unwrap();
    raw["operations"][0]["responses"][0]["status"] = json!("nope");
    let error = pons_ir::decode(&serde_json::to_vec(&raw).unwrap()).unwrap_err();
    assert_eq!(
        error.to_string(),
        "operations[0].responses[0].status: invalid HTTP status class nope"
    );

    let mut raw: Value = serde_json::from_str(fixture_json()).unwrap();
    raw["operations"][0]["path"][0]["l1t"] = json!("x");
    let error = pons_ir::decode(&serde_json::to_vec(&raw).unwrap()).unwrap_err();
    assert!(
        error
            .to_string()
            .starts_with("operations[0].path[0]: data did not match any variant"),
        "{error}"
    );

    let mut raw: Value = serde_json::from_str(fixture_json()).unwrap();
    raw["operations"][1]["headers"][0]["presence"] = json!("maybe");
    let error = pons_ir::decode(&serde_json::to_vec(&raw).unwrap()).unwrap_err();
    assert!(
        error
            .to_string()
            .starts_with("operations[1].headers[0].presence: unknown variant `maybe`"),
        "{error}"
    );

    let mut raw: Value = serde_json::from_str(fixture_json()).unwrap();
    raw["types"]["LoginError"]["repr"]["tag"] = json!(5);
    let error = pons_ir::decode(&serde_json::to_vec(&raw).unwrap()).unwrap_err();
    assert_eq!(
        error.to_string(),
        "types.LoginError: invalid type: integer `5`, expected a string"
    );
}

fn fixture_json() -> &'static str {
    include_str!("../../../../spec/fixtures/login.json")
}
