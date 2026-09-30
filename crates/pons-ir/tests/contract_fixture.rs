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
                      maximum: Option<serde_json::Value>| {
        let definition = document.types.get_mut("User").unwrap();
        let TypeKind::Struct { fields, .. } = &mut definition.definition else {
            panic!()
        };
        let TypeRef::Inline(InlineType::Int {
            bits: field_bits,
            signed: field_signed,
            minimum: field_minimum,
            maximum: field_maximum,
            ..
        }) = &mut fields[0].ty
        else {
            panic!()
        };
        *field_bits = bits;
        *field_signed = signed;
        *field_minimum = minimum;
        *field_maximum = maximum;
    };
    for (bits, signed, minimum, maximum, message) in [
        (
            32,
            true,
            Some(serde_json::json!(2_147_483_648_i64)),
            None,
            "minimum is above its native range",
        ),
        (
            32,
            true,
            None,
            Some(serde_json::json!(-2_147_483_649_i64)),
            "maximum is below its native range",
        ),
        (
            32,
            false,
            Some(serde_json::json!(4_294_967_296_u64)),
            None,
            "minimum is above its native range",
        ),
        (
            32,
            false,
            None,
            Some(serde_json::json!(-1)),
            "maximum is below its native range",
        ),
        (
            128,
            true,
            None,
            Some(serde_json::json!(
                170_141_183_460_469_231_731_687_303_715_884_105_728.0_f64
            )),
            "maximum is above its native range",
        ),
        (
            128,
            false,
            None,
            Some(serde_json::json!(
                340_282_366_920_938_463_463_374_607_431_768_211_456.0_f64
            )),
            "maximum is above its native range",
        ),
    ] {
        let mut document = base.clone();
        set_bounds(&mut document, bits, signed, minimum, maximum);
        assert!(
            document
                .validate()
                .unwrap_err()
                .iter()
                .any(|error| error.to_string().contains(message))
        );
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
