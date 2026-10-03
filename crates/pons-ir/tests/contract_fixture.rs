use pons_ir::{
    AuthScheme, ContentKind, CredentialLocator, Diagnostic, DiagnosticSeverity, Document, EnumRepr,
    ErrorStatus, InlineType, Method, OAuthFlow, OAuthFlowKind, Presence, ResponseHeader,
    ResponseStatus, StreamKind, TypeKind, TypeRef, VariantPayload,
};
use std::collections::BTreeMap;

#[test]
fn consumer_can_load_validate_and_canonicalize_the_contract_fixture() {
    let document: Document =
        serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
    document.validate().unwrap();

    let canonical = document.to_canonical_json().unwrap();
    let decoded: Document = serde_json::from_str(&canonical).unwrap();
    decoded.validate().unwrap();
    assert_eq!(document, decoded);
}

#[test]
fn validation_rejects_empty_auth_alternatives() {
    let mut document: Document =
        serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
    document.operations[0].auth.alternatives = vec![vec![]];
    let errors = document.validate().unwrap_err();
    assert!(errors.iter().any(|error| {
        error
            .to_string()
            .contains("auth alternative cannot be empty")
    }));
}

#[test]
fn field_error_diagnostics_block_validation() {
    let mut document: Document =
        serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
    let user = document.types.get_mut("User").unwrap();
    let TypeKind::Struct { fields, .. } = &mut user.definition else {
        unreachable!()
    };
    fields[0].diagnostics.push(Diagnostic {
        severity: DiagnosticSeverity::Error,
        code: "FIELD_ERROR".into(),
        message: "field is blocked".into(),
        origin: None,
        ext: BTreeMap::new(),
    });

    assert!(
        document
            .validate()
            .unwrap_err()
            .iter()
            .any(|error| error.to_string().contains("FIELD_ERROR"))
    );
}

#[test]
fn variant_error_diagnostics_block_validation() {
    let mut document: Document =
        serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
    let definition = document.types.get_mut("LoginError").unwrap();
    let TypeKind::Enum { variants, .. } = &mut definition.definition else {
        unreachable!()
    };
    variants[0].diagnostics.push(Diagnostic {
        severity: DiagnosticSeverity::Error,
        code: "VARIANT_ERROR".into(),
        message: "variant is blocked".into(),
        origin: None,
        ext: BTreeMap::new(),
    });

    let errors = document
        .validate()
        .unwrap_err()
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    assert!(
        errors.iter().any(|error| error.contains("VARIANT_ERROR")),
        "{errors:?}"
    );
    assert!(
        errors
            .iter()
            .any(|error| error.contains("type LoginError.variants.invalid diagnostics"))
    );

    let TypeKind::Enum { variants, .. } =
        &mut document.types.get_mut("LoginError").unwrap().definition
    else {
        unreachable!()
    };
    variants[0].diagnostics[0].severity = DiagnosticSeverity::Warning;
    document.validate().unwrap();
}

#[test]
fn diff_treats_header_names_case_insensitively() {
    let mut old: Document =
        serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
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
        serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
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
        serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
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
        serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
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
        serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
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
        serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
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
        serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
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
            serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
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
        serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
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
        serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
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
fn validation_rejects_duplicate_error_identities() {
    let base: Document =
        serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
    let mut document = base.clone();
    let error = document.operations[0].errors[0].clone();
    let mut duplicate = error.clone();
    duplicate.retryable = Some(true);
    document.common_errors.push(error);
    document.common_errors.push(duplicate);
    assert!(
        document
            .validate()
            .unwrap_err()
            .iter()
            .any(|error| error.to_string().contains("duplicate error"))
    );

    let mut document = base.clone();
    let error = document.operations[0].errors[0].clone();
    let mut duplicate = error.clone();
    duplicate.retryable = Some(true);
    document.operations[0].errors.push(duplicate);
    assert!(
        document
            .validate()
            .unwrap_err()
            .iter()
            .any(|error| error.to_string().contains("duplicate error"))
    );

    let mut document = base.clone();
    let error = document.operations[0].errors[0].clone();
    document.common_errors.push(error);
    assert!(
        document
            .validate()
            .unwrap_err()
            .iter()
            .any(|error| error.to_string().contains("is inherited"))
    );

    let mut document = base;
    let error = document.operations[0].errors[0].clone();
    document.common_errors.push(error);
    document.operations[0].errors_inherit = false;
    assert!(document.validate().is_ok());
}

#[test]
fn validation_rejects_duplicate_error_dispatch_slots() {
    let base: Document =
        serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
    let mut document = base.clone();
    let error = document.operations[0].errors[0].clone();
    let mut other = error.clone();
    other.name = vec!["other".into()];
    document.common_errors.push(error);
    document.common_errors.push(other);
    assert!(
        document
            .validate()
            .unwrap_err()
            .iter()
            .any(|error| error.to_string().contains("duplicate error dispatch slot"))
    );

    let mut document = base.clone();
    let error = document.operations[0].errors[0].clone();
    let mut other = error.clone();
    other.name = vec!["other".into()];
    document.operations[0].errors.push(other);
    assert!(
        document
            .validate()
            .unwrap_err()
            .iter()
            .any(|error| error.to_string().contains("duplicate error dispatch slot"))
    );

    let mut document = base.clone();
    let error = document.operations[0].errors[0].clone();
    document.common_errors.push(error);
    assert!(
        document
            .validate()
            .unwrap_err()
            .iter()
            .any(|error| error.to_string().contains("dispatch slot")
                && error.to_string().contains("inherited"))
    );

    let mut document = base;
    let mut exact = document.operations[0].errors[0].clone();
    exact.status = ErrorStatus::Exact(400);
    exact.name = vec!["exact".into()];
    document.operations[0].errors.push(exact);
    assert!(document.validate().is_ok());
}

#[test]
fn validation_refuses_an_operation_error_a_common_error_already_answers_for() {
    let base: Document =
        serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
    let with_common = |operation: ErrorStatus, common: ErrorStatus| {
        let mut document = base.clone();
        let rejected = &mut document.operations[0].errors[0];
        rejected.status = operation;
        rejected.name = vec!["request".into(), "rejected".into()];
        let mut inherited = rejected.clone();
        inherited.status = common;
        document.common_errors.push(inherited);
        document
    };
    let inherited = |document: &Document, shadowing: &str| {
        document.validate().unwrap_err().iter().any(|error| {
            let error = error.to_string();
            error.contains("error dispatch slot")
                && error.contains("is inherited")
                && error.contains(shadowing)
        })
    };

    assert!(inherited(
        &with_common(ErrorStatus::Class(4), ErrorStatus::Class(4)),
        "Class(4)"
    ));
    assert!(inherited(
        &with_common(ErrorStatus::Default, ErrorStatus::Default),
        "Default"
    ));

    with_common(ErrorStatus::Exact(404), ErrorStatus::Class(4))
        .validate()
        .unwrap();
    with_common(ErrorStatus::Exact(422), ErrorStatus::Default)
        .validate()
        .unwrap();
    with_common(ErrorStatus::Exact(422), ErrorStatus::Class(5))
        .validate()
        .unwrap();
    assert!(
        with_common(ErrorStatus::Exact(422), ErrorStatus::Exact(422))
            .validate()
            .unwrap_err()
            .iter()
            .any(|error| error.to_string().contains("is inherited"))
    );
}

#[test]
fn validation_refuses_two_common_errors_that_shadow_one_dispatch_slot() {
    let base: Document =
        serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
    let with_common = |first: ErrorStatus, second: ErrorStatus| {
        let mut document = base.clone();
        let template = document.operations[0].errors[0].clone();
        document.operations[0].errors.clear();
        for status in [first, second] {
            let mut common = template.clone();
            common.status = status;
            document.common_errors.push(common);
        }
        document
    };
    let shadowed = |document: &Document, shadowing: &str| {
        document.validate().unwrap_err().iter().any(|error| {
            let error = error.to_string();
            error.contains("error dispatch slot")
                && error.contains("is shadowed")
                && error.contains(shadowing)
        })
    };

    assert!(shadowed(
        &with_common(ErrorStatus::Class(4), ErrorStatus::Class(4)),
        "Class(4)"
    ));
    assert!(shadowed(
        &with_common(ErrorStatus::Default, ErrorStatus::Default),
        "Default"
    ));

    assert!(
        with_common(ErrorStatus::Exact(422), ErrorStatus::Exact(422))
            .validate()
            .unwrap_err()
            .iter()
            .any(|error| error.to_string().contains("duplicate error dispatch slot"))
    );
    for dispatchable in [
        with_common(ErrorStatus::Exact(404), ErrorStatus::Class(4)),
        with_common(ErrorStatus::Exact(422), ErrorStatus::Default),
        with_common(ErrorStatus::Exact(422), ErrorStatus::Class(5)),
    ] {
        dispatchable.validate().unwrap();
    }
}

#[test]
fn an_error_status_covers_the_statuses_a_runtime_can_answer_it_for() {
    assert!(ErrorStatus::Class(4).covers(&ErrorStatus::Exact(422)));
    assert!(ErrorStatus::Class(4).covers(&ErrorStatus::Class(4)));
    assert!(ErrorStatus::Default.covers(&ErrorStatus::Exact(418)));
    assert!(ErrorStatus::Default.covers(&ErrorStatus::Default));
    assert!(ErrorStatus::Exact(422).covers(&ErrorStatus::Exact(422)));
    assert!(!ErrorStatus::Exact(422).covers(&ErrorStatus::Class(4)));
    assert!(!ErrorStatus::Class(4).covers(&ErrorStatus::Exact(503)));
    assert!(!ErrorStatus::Class(4).covers(&ErrorStatus::Default));
}

#[test]
fn an_error_status_shadows_only_the_slots_it_answers_for_the_same() {
    assert!(ErrorStatus::Class(4).shadows(&ErrorStatus::Class(4)));
    assert!(ErrorStatus::Default.shadows(&ErrorStatus::Default));
    assert!(!ErrorStatus::Class(4).shadows(&ErrorStatus::Exact(404)));
    assert!(!ErrorStatus::Default.shadows(&ErrorStatus::Class(4)));
    assert!(!ErrorStatus::Default.shadows(&ErrorStatus::Exact(404)));
    assert!(!ErrorStatus::Exact(404).shadows(&ErrorStatus::Class(4)));
    assert!(!ErrorStatus::Exact(404).shadows(&ErrorStatus::Default));
    assert!(!ErrorStatus::Exact(404).shadows(&ErrorStatus::Exact(404)));
    assert!(!ErrorStatus::Class(4).shadows(&ErrorStatus::Class(5)));
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
                    "fields": [{"wire": "user", "ident": ["user"], "ty": {"ref": "User"}, "presence": "required", "access": "read_only"}]
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
            serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
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

#[test]
fn validation_rejects_exact_error_response_collisions() {
    let base: Document =
        serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
    let mut document = base.clone();
    document.operations[0].errors[0].status = ErrorStatus::Exact(200);
    assert!(document.validate().unwrap_err().iter().any(|error| {
        error
            .to_string()
            .contains("collides with a success response")
    }));

    let mut document = base.clone();
    let mut common = document.operations[0].errors[0].clone();
    common.status = ErrorStatus::Exact(200);
    document.common_errors.push(common);
    assert!(document.validate().unwrap_err().iter().any(|error| {
        error
            .to_string()
            .contains("collides with a success response")
    }));

    let mut document = base;
    document.operations[0].responses[0].status = ResponseStatus::Exact(400);
    document.operations[0].errors[0].status = ErrorStatus::Class(5);
    assert!(document.validate().is_ok());
}

#[test]
fn validation_rejects_overlapping_response_error_status_dispatch() {
    let base: Document =
        serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
    let has_collision = |document: &Document| {
        document.validate().unwrap_err().iter().any(|error| {
            error
                .to_string()
                .contains("collides with a success response")
        })
    };

    let mut document = base.clone();
    document.operations[0].responses[0].status = ResponseStatus::Class(4);
    document.operations[0].errors[0].status = ErrorStatus::Class(4);
    assert!(has_collision(&document));

    document = base.clone();
    document.operations[0].responses[0].status = ResponseStatus::Class(4);
    document.operations[0].errors[0].status = ErrorStatus::Exact(404);
    assert!(has_collision(&document));

    document = base.clone();
    document.operations[0].responses[0].status = ResponseStatus::Unknown;
    document.operations[0].errors[0].status = ErrorStatus::Class(2);
    assert!(has_collision(&document));

    document = base.clone();
    document.operations[0].responses[0].status = ResponseStatus::Default;
    document.operations[0].errors[0].status = ErrorStatus::Exact(201);
    assert!(has_collision(&document));

    document = base.clone();
    document.operations[0].responses[0].status = ResponseStatus::Class(4);
    let mut common = document.operations[0].errors[0].clone();
    common.name = vec!["common".into()];
    document.common_errors.push(common);
    assert!(has_collision(&document));

    document = base.clone();
    document.operations[0].responses[0].status = ResponseStatus::Unknown;
    document.operations[0].errors[0].status = ErrorStatus::Default;
    assert!(has_collision(&document));

    document = base.clone();
    document.operations[0].responses[0].status = ResponseStatus::Default;
    document.operations[0].errors[0].status = ErrorStatus::Default;
    assert!(has_collision(&document));

    document = base.clone();
    document.operations[0].responses[0].status = ResponseStatus::Default;
    document.operations[0].errors[0].status = ErrorStatus::Class(5);
    document.validate().unwrap();

    let mut disjoint = base;
    disjoint.operations[0].responses[0].status = ResponseStatus::Class(2);
    disjoint.operations[0].errors[0].status = ErrorStatus::Class(4);
    disjoint.validate().unwrap();
}

#[test]
fn validation_rejects_two_catch_all_success_responses() {
    let base: Document =
        serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
    let with_catch_alls = |first: ResponseStatus, second: ResponseStatus| {
        let mut document = base.clone();
        document.operations[0].responses[0].status = first;
        let mut other = document.operations[0].responses[0].clone();
        other.status = second;
        document.operations[0].responses.push(other);
        document
    };

    for document in [
        with_catch_alls(ResponseStatus::Default, ResponseStatus::Unknown),
        with_catch_alls(ResponseStatus::Unknown, ResponseStatus::Default),
    ] {
        assert!(document.validate().unwrap_err().iter().any(|error| {
            error
                .to_string()
                .contains("duplicate response status default")
        }));
    }

    for status in [ResponseStatus::Default, ResponseStatus::Unknown] {
        let mut document = base.clone();
        document.operations[0].responses[0].status = status;
        document.validate().unwrap();
    }
}

#[test]
fn validation_checks_64_bit_integer_bounds_exactly() {
    let base: Document =
        serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
    let mut document = base.clone();
    let definition = document.types.get_mut("User").unwrap();
    let TypeKind::Struct { fields, .. } = &mut definition.definition else {
        panic!()
    };
    let TypeRef::Inline(InlineType::Int {
        minimum, signed, ..
    }) = &mut fields[0].ty
    else {
        panic!()
    };
    *signed = true;
    *minimum = Some(serde_json::json!(9_223_372_036_854_775_808_u64));
    assert!(document.validate().unwrap_err().iter().any(|error| {
        error
            .to_string()
            .contains("minimum is above its native range")
    }));

    let mut document = base.clone();
    let definition = document.types.get_mut("User").unwrap();
    let TypeKind::Struct { fields, .. } = &mut definition.definition else {
        panic!()
    };
    let TypeRef::Inline(InlineType::Int {
        minimum,
        maximum,
        signed,
        ..
    }) = &mut fields[0].ty
    else {
        panic!()
    };
    *signed = true;
    *minimum = Some(serde_json::json!(9_223_372_036_854_775_808_u64));
    *maximum = Some(serde_json::json!(i64::MAX));
    assert!(
        document
            .validate()
            .unwrap_err()
            .iter()
            .any(|error| error.to_string().contains("minimum exceeds maximum"))
    );

    let mut document = base.clone();
    let definition = document.types.get_mut("User").unwrap();
    let TypeKind::Struct { fields, .. } = &mut definition.definition else {
        panic!()
    };
    let TypeRef::Inline(InlineType::Int {
        minimum, maximum, ..
    }) = &mut fields[0].ty
    else {
        panic!()
    };
    *minimum = Some(serde_json::json!(9_007_199_254_740_993_u64));
    *maximum = Some(serde_json::json!(9_007_199_254_740_992.0_f64));
    assert!(
        document
            .validate()
            .unwrap_err()
            .iter()
            .any(|error| error.to_string().contains("minimum exceeds maximum"))
    );

    let mut document = base;
    let definition = document.types.get_mut("User").unwrap();
    let TypeKind::Struct { fields, .. } = &mut definition.definition else {
        panic!()
    };
    let TypeRef::Inline(InlineType::Int { maximum, .. }) = &mut fields[0].ty else {
        panic!()
    };
    *maximum = Some(serde_json::json!(u64::MAX));
    assert!(document.validate().is_ok());
}

#[test]
fn validation_checks_integer_bounds_in_both_directions() {
    let base: Document =
        serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
    let set_bounds = |document: &mut Document,
                      bits: u16,
                      signed: bool,
                      minimum: Option<serde_json::Value>,
                      maximum: Option<serde_json::Value>,
                      multiple_of: Option<serde_json::Value>| {
        let definition = document.types.get_mut("User").unwrap();
        let TypeKind::Struct { fields, .. } = &mut definition.definition else {
            panic!()
        };
        let TypeRef::Inline(InlineType::Int {
            bits: field_bits,
            signed: field_signed,
            minimum: field_minimum,
            maximum: field_maximum,
            multiple_of: field_multiple_of,
            ..
        }) = &mut fields[0].ty
        else {
            panic!()
        };
        *field_bits = bits;
        *field_signed = signed;
        *field_minimum = minimum;
        *field_maximum = maximum;
        *field_multiple_of = multiple_of;
    };
    for (bits, signed, minimum, maximum, multiple_of, message) in [
        (
            32,
            true,
            Some(serde_json::json!(2_147_483_648_i64)),
            None,
            None,
            "minimum is above its native range",
        ),
        (
            32,
            true,
            None,
            Some(serde_json::json!(-2_147_483_649_i64)),
            None,
            "maximum is below its native range",
        ),
        (
            32,
            false,
            Some(serde_json::json!(4_294_967_296_u64)),
            None,
            None,
            "minimum is above its native range",
        ),
        (
            32,
            false,
            None,
            Some(serde_json::json!(-1)),
            None,
            "maximum is below its native range",
        ),
        (
            128,
            true,
            None,
            Some(serde_json::json!(
                170_141_183_460_469_231_731_687_303_715_884_105_728.0_f64
            )),
            None,
            "maximum is above its native range",
        ),
        (
            128,
            false,
            None,
            Some(serde_json::json!(
                340_282_366_920_938_463_463_374_607_431_768_211_456.0_f64
            )),
            None,
            "maximum is above its native range",
        ),
        (
            8,
            false,
            Some(serde_json::json!(1.5)),
            None,
            None,
            "minimum must be an integral value",
        ),
        (
            8,
            false,
            None,
            Some(serde_json::json!(255.5)),
            None,
            "maximum must be an integral value",
        ),
        (
            8,
            false,
            Some(serde_json::json!(0)),
            Some(serde_json::json!(255)),
            Some(serde_json::json!(0.5)),
            "multiple_of must be an integral value",
        ),
    ] {
        let mut document = base.clone();
        set_bounds(&mut document, bits, signed, minimum, maximum, multiple_of);
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

    for (minimum, maximum, multiple_of) in [
        (
            serde_json::json!(1.0),
            serde_json::json!(10.0),
            serde_json::json!(2.0),
        ),
        (
            serde_json::json!(0),
            serde_json::json!(255),
            serde_json::json!(1),
        ),
    ] {
        let mut document = base.clone();
        set_bounds(
            &mut document,
            8,
            false,
            Some(minimum),
            Some(maximum),
            Some(multiple_of),
        );
        assert!(document.validate().is_ok());
    }
}

#[test]
fn validation_rejects_invalid_oauth_flow_sets() {
    let base: Document =
        serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
    let flow = |kind| {
        let (authorization_url, token_url) = match kind {
            OAuthFlowKind::AuthorizationCode => (
                Some("https://identity.example.test/authorize".into()),
                Some("https://identity.example.test/token".into()),
            ),
            OAuthFlowKind::Implicit => {
                (Some("https://identity.example.test/authorize".into()), None)
            }
            OAuthFlowKind::Password | OAuthFlowKind::ClientCredentials => {
                (None, Some("https://identity.example.test/token".into()))
            }
        };
        OAuthFlow {
            kind,
            authorization_url,
            token_url,
            scopes: Default::default(),
            ext: Default::default(),
        }
    };

    let mut document = base.clone();
    document
        .auth_schemes
        .insert("oauth".into(), AuthScheme::OAuth2 { flows: Vec::new() });
    assert!(
        document
            .validate()
            .unwrap_err()
            .iter()
            .any(|error| error.to_string().contains("at least one flow"))
    );

    let mut document = base.clone();
    document.auth_schemes.insert(
        "oauth".into(),
        AuthScheme::OAuth2 {
            flows: vec![
                flow(OAuthFlowKind::AuthorizationCode),
                flow(OAuthFlowKind::AuthorizationCode),
            ],
        },
    );
    assert!(
        document
            .validate()
            .unwrap_err()
            .iter()
            .any(|error| error.to_string().contains("duplicate OAuth2 flow"))
    );

    for kind in [
        OAuthFlowKind::AuthorizationCode,
        OAuthFlowKind::Implicit,
        OAuthFlowKind::Password,
        OAuthFlowKind::ClientCredentials,
    ] {
        let mut document = base.clone();
        let mut invalid = flow(kind);
        match kind {
            OAuthFlowKind::AuthorizationCode => invalid.token_url = None,
            OAuthFlowKind::Implicit => {
                invalid.token_url = Some("https://identity.example.test/token".into())
            }
            OAuthFlowKind::Password => {
                invalid.authorization_url = Some("https://identity.example.test/authorize".into())
            }
            OAuthFlowKind::ClientCredentials => invalid.token_url = None,
        }
        document.auth_schemes.insert(
            "oauth".into(),
            AuthScheme::OAuth2 {
                flows: vec![invalid],
            },
        );
        assert!(
            document
                .validate()
                .unwrap_err()
                .iter()
                .any(|error| error.to_string().contains("invalid URL fields"))
        );
    }

    let mut document = base;
    document.auth_schemes.insert(
        "oauth".into(),
        AuthScheme::OAuth2 {
            flows: vec![
                flow(OAuthFlowKind::AuthorizationCode),
                flow(OAuthFlowKind::ClientCredentials),
            ],
        },
    );
    assert!(document.validate().is_ok());
}
