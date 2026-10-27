use pons_ir::{
    AuthScheme, ContentKind, CredentialLocator, Document, EnumRepr, ErrorStatus, InlineType,
    Method, Presence, ResponseHeader, ResponseStatus, StreamKind, TypeKind, TypeRef,
    VariantPayload,
};
use std::collections::BTreeMap;

#[test]
fn diff_treats_header_names_case_insensitively() {
    let mut old: Document =
        serde_json::from_str(include_str!("../../../../spec/fixtures/login.json")).unwrap();
    old.operations[0].errors[0].headers.push(ResponseHeader {
        wire: "Retry-After".into(),
        ty: TypeRef::Inline(InlineType::String {
            format: None,
            min_length: None,
            max_length: None,
            pattern: None,
        }),
        presence: Presence::Required,
        ext: BTreeMap::new(),
    });
    let mut common = old.operations[0].errors[0].clone();
    common.name = vec!["common".into()];
    common.status = ErrorStatus::Class(5);
    old.common_errors.push(common);

    let mut new = old.clone();
    new.operations[0].errors[0].headers[0].wire = "retry-after".into();
    new.common_errors[0].headers[0].wire = "retry-after".into();
    let operation = new
        .operations
        .iter_mut()
        .find(|operation| operation.stream.is_some())
        .unwrap();
    operation.headers[0].wire = "x-request-id".into();
    operation.responses[0].headers[0].wire = "x-trace-id".into();

    assert!(old.diff(&new).changes.is_empty());

    let mut changed = old.clone();
    changed.operations[0].errors[0].headers[0].presence = Presence::Optional;
    assert!(
        old.diff(&changed)
            .changes
            .iter()
            .any(|change| change.level == pons_ir::ChangeLevel::Breaking)
    );
}

#[test]
fn validation_rejects_ambiguous_adjacent_enums() {
    let mut document: Document =
        serde_json::from_str(include_str!("../../../../spec/fixtures/login.json")).unwrap();
    let definition = document.types.get_mut("LoginError").unwrap();
    let TypeKind::Enum { repr, .. } = &mut definition.definition else {
        panic!()
    };
    *repr = EnumRepr::Adjacent {
        tag: "code".into(),
        content: "code".into(),
    };
    assert!(
        document
            .validate()
            .unwrap_err()
            .iter()
            .any(|error| error.to_string().contains("tag and content must differ"))
    );

    let definition = document.types.get_mut("LoginError").unwrap();
    let TypeKind::Enum { repr, variants } = &mut definition.definition else {
        panic!()
    };
    *repr = EnumRepr::Adjacent {
        tag: "code".into(),
        content: "payload".into(),
    };
    let VariantPayload::Struct { fields } = &mut variants[1].payload else {
        panic!()
    };
    fields[0].wire = "payload".into();
    assert!(
        document
            .validate()
            .unwrap_err()
            .iter()
            .any(|error| error.to_string().contains("reserved enum field"))
    );
}

#[test]
fn validation_rejects_ambiguous_sse_events() {
    let mut document: Document =
        serde_json::from_str(include_str!("../../../../spec/fixtures/login.json")).unwrap();
    {
        let operation = document
            .operations
            .iter_mut()
            .find(|operation| operation.stream.is_some())
            .unwrap();
        let Some(pons_ir::StreamDescription {
            kind: StreamKind::Sse { events },
        }) = &mut operation.stream
        else {
            panic!()
        };
        events[0].event.clear();
    }
    assert!(
        document
            .validate()
            .unwrap_err()
            .iter()
            .any(|error| error.to_string().contains("event name cannot be empty"))
    );

    {
        let operation = document
            .operations
            .iter_mut()
            .find(|operation| operation.stream.is_some())
            .unwrap();
        let Some(pons_ir::StreamDescription {
            kind: StreamKind::Sse { events },
        }) = &mut operation.stream
        else {
            panic!()
        };
        events[0].event = "user".into();
        let duplicate = events[0].clone();
        events.push(duplicate);
    }
    assert!(
        document
            .validate()
            .unwrap_err()
            .iter()
            .any(|error| error.to_string().contains("duplicate SSE event user"))
    );
}

#[test]
fn validation_rejects_sse_event_names_containing_crlf() {
    let base: Document =
        serde_json::from_str(include_str!("../../../../spec/fixtures/login.json")).unwrap();
    for separator in ['\n', '\r'] {
        for name in [
            format!("user{separator}admin"),
            format!("{separator}user"),
            format!("user{separator}"),
        ] {
            let mut document = base.clone();
            let operation = document
                .operations
                .iter_mut()
                .find(|operation| operation.stream.is_some())
                .unwrap();
            let Some(StreamKind::Sse { events }) =
                operation.stream.as_mut().map(|stream| &mut stream.kind)
            else {
                panic!()
            };
            events[0].event = name.clone();
            let reported = document.validate().unwrap_err();
            assert!(
                reported
                    .iter()
                    .any(|error| error.to_string().contains("line separator")),
                "expected {name:?} to be rejected, got {reported:?}"
            );
        }
    }
    for separator in ['\u{85}', '\u{2028}', '\u{2029}'] {
        let mut document = base.clone();
        let operation = document
            .operations
            .iter_mut()
            .find(|operation| operation.stream.is_some())
            .unwrap();
        let Some(StreamKind::Sse { events }) =
            operation.stream.as_mut().map(|stream| &mut stream.kind)
        else {
            panic!()
        };
        events[0].event = format!("user{separator}admin");
        assert!(
            document.validate().is_ok(),
            "expected {separator:?} to be accepted"
        );
    }
}

#[test]
fn validation_rejects_bodies_on_bodyless_http_responses() {
    let base: Document =
        serde_json::from_str(include_str!("../../../../spec/fixtures/login.json")).unwrap();
    for status in [100, 204, 205, 304] {
        let mut document = base.clone();
        document.operations[0].responses[0].status = ResponseStatus::Exact(status);
        assert!(
            document
                .validate()
                .unwrap_err()
                .iter()
                .any(|error| error.to_string().contains("cannot carry a body"))
        );
    }

    let mut document = base.clone();
    document.operations[0].responses[0].status = ResponseStatus::Class(1);
    assert!(
        document
            .validate()
            .unwrap_err()
            .iter()
            .any(|error| error.to_string().contains("cannot carry a body"))
    );

    let mut document = base.clone();
    document.operations[0].responses[0].status = ResponseStatus::Class(1);
    document.operations[0].responses[0].content = ContentKind::Empty;
    document.operations[0].responses[0].body = None;
    document.validate().unwrap();

    let mut document = base.clone();
    document.operations[1].responses[0].status = ResponseStatus::Exact(204);
    assert!(
        document
            .validate()
            .unwrap_err()
            .iter()
            .any(|error| error.to_string().contains("cannot carry a body"))
    );

    let mut document = base.clone();
    document.operations[0].method = Method::Head;
    assert!(
        document
            .validate()
            .unwrap_err()
            .iter()
            .any(|error| error.to_string().contains("cannot carry a body"))
    );

    let mut document = base.clone();
    let common_error = document.operations[0].errors[0].clone();
    document.common_errors.push(common_error);
    document.operations[0].errors.clear();
    document.operations[0].method = Method::Head;
    assert!(
        document
            .validate()
            .unwrap_err()
            .iter()
            .any(|error| error.to_string().contains("cannot carry a body"))
    );

    let mut document = base.clone();
    document.operations[0].errors[0].status = ErrorStatus::Class(1);
    assert!(
        document
            .validate()
            .unwrap_err()
            .iter()
            .any(|error| error.to_string().contains("cannot carry a body"))
    );

    for class in [2, 3] {
        let mut document = base.clone();
        document.operations[0].errors[0].status = ErrorStatus::Class(class);
        assert!(
            document
                .validate()
                .unwrap_err()
                .iter()
                .any(|error| error.to_string().contains("cannot carry a body"))
        );
    }

    let mut document = base.clone();
    document.operations[0].errors[0].status = ErrorStatus::Class(4);
    assert!(document.validate().is_ok());

    let mut document = base;
    document.operations[0].errors[0].status = ErrorStatus::Exact(204);
    assert!(
        document
            .validate()
            .unwrap_err()
            .iter()
            .any(|error| error.to_string().contains("cannot carry a body"))
    );
}

#[test]
fn validation_rejects_malformed_credential_pointers() {
    let base: Document =
        serde_json::from_str(include_str!("../../../../spec/fixtures/login.json")).unwrap();
    let mut document = base.clone();
    document.operations[0].provides.as_mut().unwrap().value = CredentialLocator::Body {
        pointer: "token".into(),
    };
    assert!(
        document
            .validate()
            .unwrap_err()
            .iter()
            .any(|error| error.to_string().contains("must be empty or start with"))
    );

    let mut document = base.clone();
    document.operations[0].provides.as_mut().unwrap().value = CredentialLocator::Body {
        pointer: "/bad~2".into(),
    };
    assert!(
        document
            .validate()
            .unwrap_err()
            .iter()
            .any(|error| error.to_string().contains("invalid escape"))
    );

    let mut document = base.clone();
    document.operations[0].provides.as_mut().unwrap().expires_at = Some(CredentialLocator::Body {
        pointer: "/bad~".into(),
    });
    assert!(
        document
            .validate()
            .unwrap_err()
            .iter()
            .any(|error| error.to_string().contains("incomplete escape"))
    );

    let mut document = base;
    document.operations[0].provides.as_mut().unwrap().value = CredentialLocator::Body {
        pointer: String::new(),
    };
    document.operations[0].provides.as_mut().unwrap().expires_at = None;
    assert!(document.validate().is_ok());
}

#[test]
fn validation_rejects_invalid_header_names() {
    let base: Document =
        serde_json::from_str(include_str!("../../../../spec/fixtures/login.json")).unwrap();
    let mut document = base.clone();
    let pons_ir::AuthScheme::ApiKey { name, .. } = document.auth_schemes.get_mut("tenant").unwrap()
    else {
        panic!()
    };
    *name = "bad header".into();
    assert!(
        document
            .validate()
            .unwrap_err()
            .iter()
            .any(|error| error.to_string().contains("API key header"))
    );

    let mut document = base.clone();
    document.operations[0].provides.as_mut().unwrap().value = CredentialLocator::Header {
        name: "bad header".into(),
    };
    assert!(
        document
            .validate()
            .unwrap_err()
            .iter()
            .any(|error| error.to_string().contains("credential header"))
    );

    let mut document = base.clone();
    document.operations[0].errors[0]
        .headers
        .push(ResponseHeader {
            wire: "bad header".into(),
            ty: TypeRef::Inline(InlineType::String {
                format: None,
                min_length: None,
                max_length: None,
                pattern: None,
            }),
            presence: Presence::Required,
            ext: Default::default(),
        });
    assert!(
        document
            .validate()
            .unwrap_err()
            .iter()
            .any(|error| error.to_string().contains("not a valid HTTP field name"))
    );

    let mut document = base;
    document.operations[0].provides.as_mut().unwrap().expires_at =
        Some(CredentialLocator::Header {
            name: String::new(),
        });
    assert!(
        document
            .validate()
            .unwrap_err()
            .iter()
            .any(|error| error.to_string().contains("credential header"))
    );
}

#[test]
fn a_query_field_the_encoding_cannot_carry_is_refused_by_check() {
    fn with_field(presence: &str) -> Document {
        let mut value: serde_json::Value =
            serde_json::from_str(include_str!("../../../../spec/fixtures/login.json")).unwrap();
        value["types"]["EventQuery"]["fields"]
            .as_array_mut()
            .unwrap()
            .push(serde_json::json!({
                "wire": "cursor",
                "ident": ["cursor"],
                "ty": {"kind": "nullable", "inner": {"kind": "string"}},
                "presence": presence
            }));
        serde_json::from_value(value).unwrap()
    }

    let errors = with_field("required")
        .validate()
        .unwrap_err()
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    assert!(
        errors.iter().any(|error| error.contains("\"cursor\"")
            && error.contains("no encoding for a present null")),
        "a required nullable query field has no encoding: {errors:?}"
    );

    with_field("optional").validate().unwrap();

    let mut value: serde_json::Value =
        serde_json::from_str(include_str!("../../../../spec/fixtures/login.json")).unwrap();
    value["types"]["EventQuery"]["fields"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::json!({
            "wire": "user",
            "ident": ["user"],
            "ty": {"ref": "User"},
            "presence": "optional"
        }));
    let document: Document = serde_json::from_value(value).unwrap();
    assert!(
        document
            .validate()
            .unwrap_err()
            .iter()
            .any(|error| error.to_string().contains("unambiguous encoding")),
        "a query field that is not a scalar still has no encoding"
    );
}

#[test]
fn validation_rejects_empty_query_and_cookie_names() {
    let base: Document =
        serde_json::from_str(include_str!("../../../../spec/fixtures/login.json")).unwrap();
    let mut document = base.clone();
    document.auth_schemes.insert(
        "query".into(),
        AuthScheme::ApiKey {
            location: pons_ir::ApiKeyLocation::Query,
            name: String::new(),
        },
    );
    assert!(
        document
            .validate()
            .unwrap_err()
            .iter()
            .any(|error| error.to_string().contains("carrier names cannot be empty"))
    );

    let mut document = base.clone();
    document.auth_schemes.insert(
        "cookie".into(),
        AuthScheme::CookieSession {
            cookie: String::new(),
        },
    );
    assert!(
        document
            .validate()
            .unwrap_err()
            .iter()
            .any(|error| error.to_string().contains("carrier names cannot be empty"))
    );

    let mut document = base;
    document.operations[0].provides.as_mut().unwrap().value = CredentialLocator::SetCookie {
        name: String::new(),
    };
    assert!(document.validate().unwrap_err().iter().any(|error| {
        error
            .to_string()
            .contains("credential cookie name cannot be empty")
    }));
}

#[test]
fn validation_rejects_a_use_site_whose_type_cannot_cross_the_boundary() {
    use serde_json::json;

    struct Mismatch {
        name: &'static str,
        edit: fn(&mut serde_json::Value),
        expected: &'static str,
    }

    let cases = [
        Mismatch {
            name: "an output type as a request body",
            edit: |raw| {
                raw["operations"][0]["body"] =
                    json!({"ty": {"ref": "User"}, "content": "json", "required": true});
            },
            expected: "operations[0].body.ty: the request body needs a type of the input or both direction, but User is declared output",
        },
        Mismatch {
            name: "an output type as a path parameter",
            edit: |raw| {
                raw["operations"][1]["path"][1] = json!({"param": "cursor", "ty": {"ref": "User"}});
            },
            expected: "operations[1].path[1].ty: the path parameter needs a type of the input or both direction, but User is declared output",
        },
        Mismatch {
            name: "an output type as a query",
            edit: |raw| {
                raw["types"]["Filter"] = json!({
                    "kind": "struct", "name": ["filter"], "direction": "output",
                    "openness": "reject",
                    "fields": [{"wire": "q", "ident": ["q"], "ty": {"kind": "string"}, "presence": "optional"}]
                });
                raw["operations"][1]["query"] = json!({"ref": "Filter"});
            },
            expected: "operations[1].query: the query needs a type of the input or both direction, but Filter is declared output",
        },
        Mismatch {
            name: "an output type as a request header",
            edit: |raw| {
                raw["operations"][1]["headers"][0]["ty"] = json!({"ref": "User"});
            },
            expected: "operations[1].headers[0].ty: the request header needs a type of the input or both direction, but User is declared output",
        },
        Mismatch {
            name: "an input type as a response body",
            edit: |raw| {
                raw["operations"][0]["responses"][0]["body"] = json!({"ref": "LoginRequest"});
            },
            expected: "operations[0].responses[0].body: the response body needs a type of the output or both direction, but LoginRequest is declared input",
        },
        Mismatch {
            name: "an input type as a response header",
            edit: |raw| {
                raw["operations"][0]["responses"][0]["headers"] = json!([{
                    "wire": "X-Login", "ty": {"ref": "LoginRequest"}, "presence": "required"
                }]);
            },
            expected: "operations[0].responses[0].headers[0].ty: the response header needs a type of the output or both direction, but LoginRequest is declared input",
        },
        Mismatch {
            name: "an input type as an error body",
            edit: |raw| {
                raw["operations"][0]["errors"][0]["body"] = json!({"ref": "LoginRequest"});
            },
            expected: "operations[0].errors[0].body: the error body needs a type of the output or both direction, but LoginRequest is declared input",
        },
        Mismatch {
            name: "an input type as an error header",
            edit: |raw| {
                raw["operations"][0]["errors"][0]["headers"] = json!([{
                    "wire": "X-Login", "ty": {"ref": "LoginRequest"}, "presence": "required"
                }]);
            },
            expected: "operations[0].errors[0].headers[0].ty: the error header needs a type of the output or both direction, but LoginRequest is declared input",
        },
        Mismatch {
            name: "an input type as a common error body",
            edit: |raw| {
                raw["common_errors"] = json!([{
                    "status": 429, "name": ["throttled"], "content": "json",
                    "body": {"ref": "LoginRequest"}
                }]);
                raw["operations"][0]["errors_inherit"] = json!(true);
            },
            expected: "common_errors[0].body: the error body needs a type of the output or both direction, but LoginRequest is declared input",
        },
        Mismatch {
            name: "an input type as a stream event",
            edit: |raw| {
                raw["operations"][1]["stream"]["events"] =
                    json!([{"event": "login", "data": {"ref": "LoginRequest"}}]);
            },
            expected: "operations[1].stream.events[0].data: the stream event needs a type of the output or both direction, but LoginRequest is declared input",
        },
        Mismatch {
            name: "an input type as a stream item",
            edit: |raw| {
                raw["operations"][1]["stream"] =
                    json!({"kind": "ndjson", "item": {"ref": "LoginRequest"}});
            },
            expected: "operations[1].stream.item: the stream item needs a type of the output or both direction, but LoginRequest is declared input",
        },
        Mismatch {
            name: "an input type reached through a newtype",
            edit: |raw| {
                raw["types"]["Envelope"] = json!({
                    "kind": "newtype", "name": ["envelope"], "direction": "both",
                    "inner": {"ref": "LoginRequest"}
                });
                raw["operations"][0]["responses"][0]["body"] = json!({"ref": "Envelope"});
            },
            expected: "operations[0].responses[0].body: the response body needs a type of the output or both direction, but LoginRequest is declared input",
        },
        Mismatch {
            name: "an output type reached through a struct field",
            edit: |raw| {
                raw["types"]["Envelope"] = json!({
                    "kind": "struct", "name": ["envelope"], "direction": "both",
                    "openness": "reject",
                    "fields": [{"wire": "user", "ident": ["user"], "ty": {"ref": "User"}, "presence": "required"}]
                });
                raw["operations"][0]["body"] =
                    json!({"ty": {"ref": "Envelope"}, "content": "json", "required": true});
            },
            expected: "operations[0].body.ty: the request body needs a type of the input or both direction, but User is declared output",
        },
        Mismatch {
            name: "an output type reached through a flattened member",
            edit: |raw| {
                raw["types"]["Envelope"] = json!({
                    "kind": "struct", "name": ["envelope"], "direction": "both",
                    "openness": "reject", "flattened": [{"ref": "User"}], "fields": []
                });
                raw["operations"][0]["body"] =
                    json!({"ty": {"ref": "Envelope"}, "content": "json", "required": true});
            },
            expected: "operations[0].body.ty: the request body needs a type of the input or both direction, but User is declared output",
        },
        Mismatch {
            name: "an output type reached through a typed openness value",
            edit: |raw| {
                raw["types"]["Envelope"] = json!({
                    "kind": "struct", "name": ["envelope"], "direction": "both",
                    "openness": {"preserve_typed": {"value": {"ref": "User"}}}, "fields": []
                });
                raw["operations"][0]["body"] =
                    json!({"ty": {"ref": "Envelope"}, "content": "json", "required": true});
            },
            expected: "operations[0].body.ty: the request body needs a type of the input or both direction, but User is declared output",
        },
        Mismatch {
            name: "an input type reached through an enum payload",
            edit: |raw| {
                raw["types"]["Envelope"] = json!({
                    "kind": "enum", "name": ["envelope"], "direction": "both",
                    "repr": {"kind": "external"},
                    "variants": [{"wire": "login", "ident": ["login"], "payload": {"kind": "newtype", "ty": {"ref": "LoginRequest"}}}]
                });
                raw["operations"][0]["responses"][0]["body"] = json!({"ref": "Envelope"});
            },
            expected: "operations[0].responses[0].body: the response body needs a type of the output or both direction, but LoginRequest is declared input",
        },
        Mismatch {
            name: "an output type in a field the request side never reads",
            edit: |raw| {
                raw["types"]["Envelope"] = json!({
                    "kind": "struct", "name": ["envelope"], "direction": "both",
                    "openness": "reject",
                    "fields": [{"wire": "user", "ident": ["user"], "ty": {"ref": "User"}, "presence": "optional", "access": "read_only"}]
                });
                raw["operations"][0]["body"] =
                    json!({"ty": {"ref": "Envelope"}, "content": "json", "required": true});
            },
            expected: "",
        },
    ];
    for Mismatch {
        name,
        edit,
        expected,
    } in cases
    {
        let mut raw: serde_json::Value =
            serde_json::from_str(include_str!("../../../../spec/fixtures/login.json")).unwrap();
        edit(&mut raw);
        let document: Document = serde_json::from_value(raw)
            .unwrap_or_else(|error| panic!("{name}: the edit is not a document: {error}"));
        let outcome = document.validate();
        if expected.is_empty() {
            outcome.unwrap_or_else(|errors| {
                panic!(
                    "{name}: {}",
                    errors
                        .iter()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                        .join("\n")
                )
            });
            continue;
        }
        let errors = outcome
            .err()
            .unwrap_or_else(|| panic!("{name}: the document was accepted"))
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(errors.contains(expected), "{name}: {errors}");
    }
}
