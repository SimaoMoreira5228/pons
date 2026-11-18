use super::*;

#[test]
fn classifies_added_fields_and_output_enum_variants() {
    let old: Document = serde_json::from_value(fixture()).unwrap();
    let mut changed = fixture();
    changed["types"]["LoginRequest"]["fields"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::json!({
            "wire": "remember",
            "ident": ["remember"],
            "ty": {"kind": "bool"},
            "presence": "optional"
        }));
    assert_eq!(
        level(&old, changed, "types.LoginRequest.fields.remember"),
        ChangeLevel::NonBreaking
    );

    let mut changed = fixture();
    changed["types"]["LoginError"]["variants"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::json!({
            "wire": "expired",
            "ident": ["expired"],
            "payload": {"kind": "unit"}
        }));
    let new: Document = serde_json::from_value(changed).unwrap();
    let diff = old.diff(&new);
    assert!(diff.has_breaking_changes());
    assert_eq!(
        diff.changes
            .iter()
            .find(|change| change.path == "types.LoginError.variants.expired")
            .unwrap()
            .level,
        ChangeLevel::Breaking
    );
}

#[test]
fn object_openness_changes_are_breaking() {
    let old: Document = serde_json::from_value(fixture()).unwrap();
    let mut changed = fixture();
    changed["types"]["User"]["openness"] = serde_json::json!("reject");
    assert_eq!(level(&old, changed, "types.User"), ChangeLevel::Breaking);
}

#[test]
fn type_name_changes_are_breaking() {
    let old: Document = serde_json::from_value(fixture()).unwrap();
    let mut new = old.clone();
    new.types.get_mut("User").unwrap().name = vec!["account".into()];
    let change = old
        .diff(&new)
        .changes
        .into_iter()
        .find(|change| change.path == "types.User")
        .unwrap();
    assert_eq!(change.level, ChangeLevel::Breaking);
    assert_eq!(change.detail, "type name changed");
}

#[test]
fn idempotency_changes_are_breaking() {
    let old: Document = serde_json::from_value(fixture()).unwrap();
    let mut new = old.clone();
    new.operations[0].semantics.idempotent = true;
    let change = old
        .diff(&new)
        .changes
        .into_iter()
        .find(|change| change.path == "operations.auth.login.semantics")
        .unwrap();
    assert_eq!(change.level, ChangeLevel::Breaking);
    assert_eq!(change.detail, "operation idempotency changed");
}

#[test]
fn route_changes_with_stable_operation_identity_are_nonbreaking() {
    let old: Document = serde_json::from_value(fixture()).unwrap();
    let mut changed = fixture();
    changed["operations"][0]["path"][1]["lit"] = "sign-in".into();
    assert_eq!(
        level(&old, changed, "operations.auth.login"),
        ChangeLevel::NonBreaking
    );
}

#[test]
fn path_parameter_type_changes_are_breaking() {
    let old: Document = serde_json::from_value(fixture()).unwrap();
    let mut changed = fixture();
    changed["operations"][1]["path"][1]["ty"] =
        serde_json::json!({"kind": "int", "bits": 32, "signed": true, "wire": "number"});
    let new: Document = serde_json::from_value(changed).unwrap();
    let diff = old.diff(&new);
    assert!(diff.changes.iter().any(|change| {
        change.path == "operations.events.watch.path[0]" && change.level == ChangeLevel::Breaking
    }));
    assert!(!diff.changes.iter().any(|change| {
        change.path == "operations.events.watch" && change.detail == "route URL changed"
    }));
}

#[test]
fn new_errors_are_breaking() {
    let old: Document = serde_json::from_value(fixture()).unwrap();
    let mut operation = old.clone();
    let mut operation_error = operation.operations[0].errors[0].clone();
    operation_error.status = crate::ErrorStatus::Exact(418);
    operation_error.name = vec!["teapot".into()];
    operation.operations[0].errors.push(operation_error);
    let diff = old.diff(&operation);
    assert!(diff.has_breaking_changes());
    assert!(diff.changes.iter().any(|change| {
        change.path == "operations.auth.login.errors.teapot"
            && change.level == ChangeLevel::Breaking
    }));

    let mut common = old.clone();
    let mut common_error = common.operations[0].errors[0].clone();
    common_error.status = crate::ErrorStatus::Exact(503);
    common_error.name = vec!["unavailable".into()];
    common.common_errors.push(common_error);
    let diff = old.diff(&common);
    assert!(diff.has_breaking_changes());
    assert!(diff.changes.iter().any(|change| {
        change.path == "common_errors.errors.unavailable" && change.level == ChangeLevel::Breaking
    }));
}

#[test]
fn added_success_response_statuses_are_breaking() {
    let old: Document = serde_json::from_value(fixture()).unwrap();
    let mut created = old.clone();
    let mut accepted = created.operations[0].responses[0].clone();
    accepted.status = crate::ResponseStatus::Exact(201);
    created.operations[0].responses.push(accepted);
    created.validate().unwrap();

    let diff = old.diff(&created);
    assert!(diff.has_breaking_changes());
    let change = change_at(&old, &created, "operations.auth.login.responses.201");
    assert_eq!(change.level, ChangeLevel::Breaking);
    assert_eq!(change.detail, "response status added");
}

#[test]
fn request_body_requiredness_is_classified() {
    let old: Document = serde_json::from_value(fixture()).unwrap();
    let mut optional = old.clone();
    optional.operations[0].body.as_mut().unwrap().required = false;
    let change = old
        .diff(&optional)
        .changes
        .into_iter()
        .find(|change| change.path == "operations.auth.login.body")
        .unwrap();
    assert_eq!(change.level, ChangeLevel::NonBreaking);
    assert_eq!(change.detail, "request body became optional");

    let mut required = optional.clone();
    required.operations[0].body.as_mut().unwrap().required = true;
    assert_eq!(
        optional
            .diff(&required)
            .changes
            .into_iter()
            .find(|change| change.path == "operations.auth.login.body")
            .unwrap()
            .level,
        ChangeLevel::Breaking
    );

    let mut media = old.clone();
    media.operations[0].body.as_mut().unwrap().media = Some(crate::MediaDescriptor::from_declared(
        "application/vnd.acme+json",
    ));
    assert_eq!(
        old.diff(&media)
            .changes
            .into_iter()
            .find(|change| change.path == "operations.auth.login.body")
            .unwrap()
            .level,
        ChangeLevel::Breaking
    );
}

#[test]
fn document_info_changes_are_classified() {
    let old: Document = serde_json::from_value(fixture()).unwrap();
    let mut new = old.clone();
    new.info.title = "Renamed".into();
    new.info.version = "2".into();
    new.info.description = Some("now with words".into());
    new.info.servers = vec!["https://api.example.test/v2".into()];
    new.info.ext.insert(
        "test.example.source/v1".into(),
        serde_json::json!({"kept": false}),
    );
    new.ext.insert(
        "com.example.cursor/v1".into(),
        serde_json::json!({"page": 2}),
    );

    let diff = old.diff(&new);
    for (path, level, detail) in [
        ("info.title", ChangeLevel::Info, "API title changed"),
        (
            "info.version",
            ChangeLevel::NonBreaking,
            "API version changed",
        ),
        (
            "info.description",
            ChangeLevel::Info,
            "API description changed",
        ),
        (
            "info.servers",
            ChangeLevel::NonBreaking,
            "API servers changed",
        ),
        (
            "info.ext",
            ChangeLevel::Warning,
            "document metadata changed",
        ),
        ("ext", ChangeLevel::Warning, "document metadata changed"),
    ] {
        let change = diff
            .changes
            .iter()
            .find(|change| change.path == path)
            .unwrap_or_else(|| panic!("no change reported at {path}"));
        assert_eq!((change.level, change.detail.as_str()), (level, detail));
    }
}

#[test]
fn enum_variant_identifier_changes_are_breaking() {
    let old: Document = serde_json::from_value(fixture()).unwrap();
    let mut new = old.clone();
    let TypeKind::Enum { variants, .. } = &mut new.types.get_mut("LoginError").unwrap().definition
    else {
        unreachable!()
    };
    variants[1].ident = vec!["locked".into(), "out".into()];
    let change = change_at(&old, &new, "types.LoginError.variants.locked");
    assert_eq!(change.level, ChangeLevel::Breaking);
    assert_eq!(change.detail, "enum variant identifier changed");
}

#[test]
fn operation_safety_is_classified() {
    let old: Document = serde_json::from_value(fixture()).unwrap();
    let mut safe = old.clone();
    safe.operations[0].semantics.safe = true;
    let change = change_at(&old, &safe, "operations.auth.login.semantics");
    assert_eq!(change.level, ChangeLevel::Breaking);
    assert_eq!(change.detail, "operation safety changed");
}

#[test]
fn auth_scheme_credential_carriers_stay_breaking() {
    let old: Document = serde_json::from_value(fixture()).unwrap();
    for (scheme, detail) in [
        (
            AuthScheme::ApiKey {
                location: crate::ApiKeyLocation::Header,
                name: "X-Account".into(),
            },
            "API key name changed",
        ),
        (
            AuthScheme::ApiKey {
                location: crate::ApiKeyLocation::Query,
                name: "X-Tenant".into(),
            },
            "API key location changed",
        ),
        (AuthScheme::Basic, "authentication scheme kind changed"),
    ] {
        let mut new = old.clone();
        new.auth_schemes.insert("tenant".into(), scheme.clone());
        new.auth_schemes.insert("session".into(), scheme);
        let path = if detail.starts_with("API key") {
            "auth_schemes.tenant"
        } else {
            "auth_schemes.session"
        };
        let change = change_at(&old, &new, path);
        assert_eq!(change.level, ChangeLevel::Breaking, "{detail}");
        assert_eq!(change.detail, detail);
    }

    let with_cookie = {
        let mut document = old.clone();
        document.auth_schemes.insert(
            "browser".into(),
            AuthScheme::CookieSession {
                cookie: "sid".into(),
            },
        );
        document
    };
    let mut renamed = with_cookie.clone();
    renamed.auth_schemes.insert(
        "browser".into(),
        AuthScheme::CookieSession {
            cookie: "session_id".into(),
        },
    );
    let change = change_at(&with_cookie, &renamed, "auth_schemes.browser");
    assert_eq!(change.level, ChangeLevel::Breaking);
    assert_eq!(change.detail, "session cookie name changed");

    let mut removed = with_cookie.clone();
    removed.auth_schemes.remove("browser");
    let change = change_at(&with_cookie, &removed, "auth_schemes.browser");
    assert_eq!(change.level, ChangeLevel::Breaking);
    assert_eq!(change.detail, "authentication scheme removed");
}

#[test]
fn oauth_flow_changes_are_classified() {
    let old = {
        let mut document: Document = serde_json::from_value(fixture()).unwrap();
        document.auth_schemes.insert(
            "oauth".into(),
            AuthScheme::OAuth2 {
                flows: vec![OAuthFlow {
                    kind: crate::OAuthFlowKind::ClientCredentials,
                    authorization_url: None,
                    token_url: Some("https://identity.example.test/token".into()),
                    scopes: BTreeMap::new(),
                    ext: BTreeMap::new(),
                }],
            },
        );
        document
    };
    const PATH: &str = "auth_schemes.oauth.flows.ClientCredentials";

    let mut retokenized = old.clone();
    let AuthScheme::OAuth2 { flows } = retokenized.auth_schemes.get_mut("oauth").unwrap() else {
        unreachable!()
    };
    flows[0].token_url = Some("https://identity.example.test/v2/token".into());
    let change = change_at(&old, &retokenized, PATH);
    assert_eq!(change.level, ChangeLevel::Breaking);
    assert_eq!(change.detail, "OAuth2 token URL changed");

    let mut documented = old.clone();
    let AuthScheme::OAuth2 { flows } = documented.auth_schemes.get_mut("oauth").unwrap() else {
        unreachable!()
    };
    flows[0]
        .scopes
        .insert("read".into(), "read everything".into());
    let change = change_at(&old, &documented, PATH);
    assert_eq!(change.level, ChangeLevel::Info);
    assert_eq!(change.detail, "OAuth2 scope documentation changed");
    assert!(!old.diff(&documented).has_breaking_changes());

    let mut extended = old.clone();
    let AuthScheme::OAuth2 { flows } = extended.auth_schemes.get_mut("oauth").unwrap() else {
        unreachable!()
    };
    flows[0]
        .ext
        .insert("test.example.source/v1".into(), serde_json::json!(true));
    let change = change_at(&old, &extended, PATH);
    assert_eq!(change.level, ChangeLevel::Warning);
    assert_eq!(change.detail, "OAuth2 flow metadata changed");

    let mut added = old.clone();
    let AuthScheme::OAuth2 { flows } = added.auth_schemes.get_mut("oauth").unwrap() else {
        unreachable!()
    };
    flows.push(OAuthFlow {
        kind: crate::OAuthFlowKind::Password,
        authorization_url: None,
        token_url: Some("https://identity.example.test/token".into()),
        scopes: BTreeMap::new(),
        ext: BTreeMap::new(),
    });
    let change = change_at(&old, &added, "auth_schemes.oauth.flows.Password");
    assert_eq!(change.level, ChangeLevel::NonBreaking);
    assert_eq!(change.detail, "OAuth2 flow added");

    let mut removed = added.clone();
    let AuthScheme::OAuth2 { flows } = removed.auth_schemes.get_mut("oauth").unwrap() else {
        unreachable!()
    };
    flows.retain(|flow| flow.kind != crate::OAuthFlowKind::Password);
    let change = change_at(&added, &removed, "auth_schemes.oauth.flows.Password");
    assert_eq!(change.level, ChangeLevel::Breaking);
    assert_eq!(change.detail, "OAuth2 flow removed");
}

#[test]
fn pagination_descriptors_are_classified_when_added_removed_or_rewritten() {
    let mut old: Document = serde_json::from_value(fixture()).unwrap();
    for operation in &mut old.operations {
        if operation.stream.is_some() {
            operation.stream = None;
            operation.responses[0].content = crate::ContentKind::Json;
            operation.responses[0].body =
                Some(serde_json::from_value(serde_json::json!({"ref": "User"})).unwrap());
        }
    }
    let mut added = old.clone();
    let operation = added
        .operations
        .iter_mut()
        .find(|operation| operation.query.is_some())
        .unwrap();
    operation.semantics.pagination = Some(
        serde_json::from_value(serde_json::json!({
            "request": {"kind": "cursor", "field": "limit"},
            "items": "/items",
            "next": "/next"
        }))
        .unwrap(),
    );
    added.validate().unwrap();

    let change = change_at(&old, &added, "operations.events.watch.semantics.pagination");
    assert_eq!(change.level, ChangeLevel::NonBreaking);
    assert_eq!(change.detail, "pagination descriptor added");

    let change = change_at(&added, &old, "operations.events.watch.semantics.pagination");
    assert_eq!(change.level, ChangeLevel::Breaking);
    assert_eq!(change.detail, "pagination descriptor removed");

    let mut rewritten = added.clone();
    let operation = rewritten
        .operations
        .iter_mut()
        .find(|operation| operation.query.is_some())
        .unwrap();
    operation.semantics.pagination.as_mut().unwrap().request =
        serde_json::from_value(serde_json::json!({"kind": "offset", "field": "limit"})).unwrap();
    let change = change_at(
        &added,
        &rewritten,
        "operations.events.watch.semantics.pagination",
    );
    assert_eq!(change.level, ChangeLevel::Breaking);
    assert_eq!(change.detail, "pagination request encoding changed");
}
