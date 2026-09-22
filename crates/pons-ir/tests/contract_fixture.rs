use pons_ir::{
    CredentialLocator, Document, EnumRepr, ErrorStatus, Method, StreamKind, TypeKind,
    VariantPayload,
};

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
fn diff_treats_header_names_case_insensitively() {
    let old: Document =
        serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
    let mut new = old.clone();
    let operation = new
        .operations
        .iter_mut()
        .find(|operation| operation.stream.is_some())
        .unwrap();
    operation.headers[0].wire = "x-request-id".into();
    operation.responses[0].headers[0].wire = "x-trace-id".into();

    assert!(old.diff(&new).changes.is_empty());
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
fn validation_rejects_bodies_on_bodyless_http_responses() {
    let base: Document =
        serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
    for status in [100, 204, 205, 304] {
        let mut document = base.clone();
        document.operations[0].responses[0].status = status;
        assert!(
            document
                .validate()
                .unwrap_err()
                .iter()
                .any(|error| error.to_string().contains("cannot carry a body"))
        );
    }

    let mut document = base.clone();
    document.operations[1].responses[0].status = 204;
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
