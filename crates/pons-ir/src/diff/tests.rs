use super::*;

fn fixture() -> serde_json::Value {
    serde_json::from_str(include_str!("../../../../fixtures/login.json")).unwrap()
}

fn level(old: &Document, changed: serde_json::Value, path: &str) -> ChangeLevel {
    let new: Document = serde_json::from_value(changed).unwrap();
    old.diff(&new)
        .changes
        .into_iter()
        .find(|change| change.path == path)
        .unwrap()
        .level
}

fn change_at(old: &Document, new: &Document, path: &str) -> Change {
    old.diff(new)
        .changes
        .into_iter()
        .find(|change| change.path == path)
        .unwrap_or_else(|| panic!("no change reported at {path}"))
}

#[test]
fn equivalent_orderings_have_no_diff() {
    let mut old: Document = serde_json::from_value(fixture()).unwrap();
    let event = old
        .operations
        .iter_mut()
        .find(|operation| operation.stream.is_some())
        .unwrap();
    event.auth.alternatives = vec![vec!["session".into()], vec!["tenant".into()]];
    let mut second_header = event.responses[0].headers[0].clone();
    second_header.wire = "X-Second".into();
    event.responses[0].headers.push(second_header);
    let flow = |kind| crate::OAuthFlow {
        kind,
        authorization_url: None,
        token_url: Some("https://identity.example.test/token".into()),
        scopes: Default::default(),
        ext: Default::default(),
    };
    old.auth_schemes.insert(
        "oauth".into(),
        crate::AuthScheme::OAuth2 {
            flows: vec![
                flow(crate::OAuthFlowKind::Password),
                flow(crate::OAuthFlowKind::ClientCredentials),
            ],
        },
    );

    let mut new = old.clone();
    new.operations.reverse();
    let event = &mut new.operations[0];
    event.auth.alternatives.reverse();
    event.responses[0].headers.reverse();
    let crate::AuthScheme::OAuth2 { flows } = new.auth_schemes.get_mut("oauth").unwrap() else {
        unreachable!()
    };
    flows.reverse();
    old.validate().unwrap();
    new.validate().unwrap();

    assert_eq!(old.diff(&new), Diff::default());
}

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
fn change_paths_keep_readable_operation_ids() {
    let old: Document = serde_json::from_value(fixture()).unwrap();
    let mut changed = fixture();
    changed["operations"][0]["id"] = serde_json::json!([["auth"], ["sign_in"]]);
    changed["operations"][1]["id"] = serde_json::json!([["events.v2"], ["watch_all"]]);
    let new: Document = serde_json::from_value(changed).unwrap();
    let diff = old.diff(&new);
    for change in &diff.changes {
        assert!(
            !change.path.contains('%'),
            "change path {} escapes the operation id",
            change.path
        );
    }
    assert!(diff.changes.iter().any(|change| {
        change.path == "operations.auth.sign_in" && change.detail == "operation added"
    }));
    assert!(diff.changes.iter().any(|change| {
        change.path == "operations.events.v2.watch_all" && change.detail == "operation added"
    }));
    assert_eq!(crate::operation_path(&[vec!["a.b".into()]]), "a%2Eb");
}

#[test]
fn diff_operation_paths_are_injective() {
    let mut old: Document = serde_json::from_value(fixture()).unwrap();
    old.operations[0].id = vec![vec!["a".into(), "b".into()]];
    let mut collision = old.operations[0].clone();
    collision.id = vec![vec!["a_b".into()]];
    old.operations.push(collision);
    old.validate().unwrap();

    let mut new = old.clone();
    new.operations.clear();
    new.validate().unwrap();

    let paths = old
        .diff(&new)
        .changes
        .into_iter()
        .filter(|change| change.detail == "operation removed")
        .map(|change| change.path)
        .collect::<Vec<_>>();
    assert!(paths.contains(&"operations.a_b".into()));
    assert!(paths.contains(&"operations.a%5Fb".into()));
}

#[test]
fn opaque_metadata_changes_are_breaking() {
    let old: Document = serde_json::from_value(fixture()).unwrap();
    let mut changed = fixture();
    changed["types"]["User"]["fields"][0]["ty"] = serde_json::json!({
        "kind": "opaque",
        "codec": "example",
        "wire": "json"
    });
    let new: Document = serde_json::from_value(changed).unwrap();
    let diff = old.diff(&new);
    assert!(diff.changes.iter().any(|change| {
        change.path == "types.User.fields.id" && change.level == ChangeLevel::Breaking
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
    media.operations[0].body.as_mut().unwrap().media_type =
        Some("application/vnd.acme+json".into());
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
    new.info
        .ext
        .insert("source.note".into(), serde_json::json!({"kept": false}));
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
fn opaque_type_metadata_is_reported_as_a_warning() {
    let old: Document = serde_json::from_value(fixture()).unwrap();
    let mut changed = fixture();
    changed["types"]["User"]["ext"]["source.type"] = serde_json::json!("dropped");
    assert_eq!(level(&old, changed, "types.User"), ChangeLevel::Warning);

    let mut changed = fixture();
    changed["types"]["User"]["fields"][0]["ext"]["source.field"] = serde_json::json!(2);
    assert_eq!(
        level(&old, changed, "types.User.fields.id"),
        ChangeLevel::Warning
    );

    let mut changed = fixture();
    changed["types"]["LoginError"]["variants"][1]["ext"]["source.variant"] =
        serde_json::json!(true);
    assert_eq!(
        level(&old, changed, "types.LoginError.variants.locked"),
        ChangeLevel::Warning
    );
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
fn field_default_changes_follow_input_direction() {
    let old: Document = serde_json::from_value(fixture()).unwrap();
    let mut input = fixture();
    input["types"]["LoginRequest"]["fields"][0]["default"] =
        serde_json::json!("someone@example.test");
    assert_eq!(
        level(&old, input, "types.LoginRequest.fields.email"),
        ChangeLevel::Breaking
    );

    let mut output = fixture();
    output["types"]["User"]["fields"][1]["default"] = serde_json::json!("anonymous");
    assert_eq!(
        level(&old, output, "types.User.fields.nickname"),
        ChangeLevel::NonBreaking
    );
}

#[test]
fn both_direction_presence_changes_remain_breaking() {
    let mut old = fixture();
    old["types"]["User"]["direction"] = serde_json::json!("both");
    old["types"]["User"]["fields"][1]["presence"] = serde_json::json!("required");
    let old: Document = serde_json::from_value(old).unwrap();
    let mut changed = serde_json::to_value(&old).unwrap();
    changed["types"]["User"]["fields"][1]["presence"] = serde_json::json!("optional");
    let new: Document = serde_json::from_value(changed).unwrap();
    assert_eq!(
        change_at(&old, &new, "types.User.fields.nickname").level,
        ChangeLevel::Breaking
    );
}

#[test]
fn operation_safety_and_pagination_are_classified() {
    let old: Document = serde_json::from_value(fixture()).unwrap();
    let mut safe = old.clone();
    safe.operations[0].semantics.safe = true;
    let change = change_at(&old, &safe, "operations.auth.login.semantics");
    assert_eq!(change.level, ChangeLevel::Breaking);
    assert_eq!(change.detail, "operation safety changed");

    let mut paginated = old.clone();
    paginated.operations[0].semantics.pagination = Some(serde_json::json!({"cursor": true}));
    let change = change_at(
        &old,
        &paginated,
        "operations.auth.login.semantics.pagination",
    );
    assert_eq!(change.level, ChangeLevel::Warning);
    assert_eq!(change.detail, "operation pagination changed");
}

#[test]
fn operation_metadata_changes_are_reported_as_a_warning() {
    let old: Document = serde_json::from_value(fixture()).unwrap();
    let mut new = old.clone();
    new.operations[0]
        .ext
        .insert("source.operation".into(), serde_json::json!("dropped"));
    let change = change_at(&old, &new, "operations.auth.login.ext");
    assert_eq!(change.level, ChangeLevel::Warning);
    assert_eq!(change.detail, "operation metadata changed");
}

fn doc(summary: Option<&str>, deprecated: Option<bool>) -> Doc {
    Doc {
        summary: summary.map(str::to_owned),
        description: None,
        deprecated,
        tags: Vec::new(),
    }
}

#[test]
fn auth_scheme_documentation_changes_are_not_breaking() {
    let old: Document = serde_json::from_value(fixture()).unwrap();
    let mut reformatted = old.clone();
    reformatted.auth_schemes.insert(
        "session".into(),
        AuthScheme::Bearer {
            format: Some("opaque".into()),
        },
    );
    let change = change_at(&old, &reformatted, "auth_schemes.session");
    assert_eq!(change.level, ChangeLevel::Info);
    assert_eq!(change.detail, "bearer token format documentation changed");
    assert!(!old.diff(&reformatted).has_breaking_changes());

    let described = {
        let mut document = old.clone();
        document.auth_schemes.insert(
            "custom".into(),
            AuthScheme::Custom {
                description: "first".into(),
                ext: BTreeMap::new(),
            },
        );
        document
    };
    let mut reworded = described.clone();
    reworded.auth_schemes.insert(
        "custom".into(),
        AuthScheme::Custom {
            description: "second".into(),
            ext: BTreeMap::new(),
        },
    );
    let change = change_at(&described, &reworded, "auth_schemes.custom");
    assert_eq!(change.level, ChangeLevel::Info);
    assert_eq!(change.detail, "custom scheme description changed");

    let mut extended = described.clone();
    let AuthScheme::Custom { ext, .. } = extended.auth_schemes.get_mut("custom").unwrap() else {
        unreachable!()
    };
    *ext = BTreeMap::from([("source.scheme".into(), serde_json::json!(1))]);
    let change = change_at(&described, &extended, "auth_schemes.custom");
    assert_eq!(change.level, ChangeLevel::Warning);
    assert_eq!(change.detail, "custom scheme metadata changed");
    assert!(!described.diff(&extended).has_breaking_changes());
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
        .insert("source.flow".into(), serde_json::json!(true));
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
fn response_metadata_changes_follow_metadata_levels() {
    let old: Document = serde_json::from_value(fixture()).unwrap();
    let mut extended = old.clone();
    extended.operations[0].responses[0]
        .ext
        .insert("source.response".into(), serde_json::json!("dropped"));
    let change = change_at(&old, &extended, "operations.auth.login.responses.200");
    assert_eq!(change.level, ChangeLevel::Warning);
    assert_eq!(change.detail, "response metadata changed");
    assert!(!old.diff(&extended).has_breaking_changes());

    let mut retyped = old.clone();
    retyped.operations[0].responses[0].body = Some(TypeRef::Named(crate::NamedRef {
        id: "LoginRequest".into(),
        variant: None,
    }));
    let change = change_at(&old, &retyped, "operations.auth.login.responses.200");
    assert_eq!(change.level, ChangeLevel::Breaking);
    assert_eq!(change.detail, "response changed");
}

#[test]
fn error_documentation_and_metadata_follow_metadata_levels() {
    let old: Document = serde_json::from_value(fixture()).unwrap();
    const PATH: &str = "operations.auth.login.errors.login_error";

    let mut summarized = old.clone();
    summarized.operations[0].errors[0].doc = Some(doc(Some("Login failed"), None));
    let change = change_at(&old, &summarized, PATH);
    assert_eq!(change.level, ChangeLevel::Info);
    assert_eq!(change.detail, "error documentation changed");
    assert!(!old.diff(&summarized).has_breaking_changes());

    let mut deprecated = summarized.clone();
    deprecated.operations[0].errors[0].doc = Some(doc(Some("Login failed"), Some(true)));
    let change = change_at(&summarized, &deprecated, PATH);
    assert_eq!(change.level, ChangeLevel::Warning);
    assert_eq!(change.detail, "error deprecation changed");
    assert!(!summarized.diff(&deprecated).has_breaking_changes());

    let mut extended = old.clone();
    extended.operations[0].errors[0]
        .ext
        .insert("source.error".into(), serde_json::json!(1));
    let change = change_at(&old, &extended, PATH);
    assert_eq!(change.level, ChangeLevel::Warning);
    assert_eq!(change.detail, "error metadata changed");
    assert!(!old.diff(&extended).has_breaking_changes());

    let mut retyped = old.clone();
    retyped.operations[0].errors[0].body = Some(TypeRef::Named(crate::NamedRef {
        id: "LoginRequest".into(),
        variant: None,
    }));
    let change = change_at(&old, &retyped, PATH);
    assert_eq!(change.level, ChangeLevel::Breaking);
    assert_eq!(change.detail, "error definition changed");
}

#[test]
fn request_header_metadata_changes_are_reported() {
    let old: Document = serde_json::from_value(fixture()).unwrap();
    let mut extended = old.clone();
    extended.operations[1].headers[0]
        .ext
        .insert("source.header".into(), serde_json::json!(1));
    let change = change_at(
        &old,
        &extended,
        "operations.events.watch.headers.X-Request-ID",
    );
    assert_eq!(change.level, ChangeLevel::Warning);
    assert_eq!(change.detail, "request header metadata changed");
    assert!(!old.diff(&extended).has_breaking_changes());

    let mut retyped = old.clone();
    retyped.operations[1].headers[0].ty = TypeRef::Inline(InlineType::Int {
        bits: 32,
        signed: true,
        wire: crate::IntegerWire::Number,
        minimum: None,
        maximum: None,
        multiple_of: None,
    });
    retyped.operations[1].headers[0]
        .ext
        .insert("source.header".into(), serde_json::json!(1));
    let path = "operations.events.watch.headers.X-Request-ID";
    let diff = old.diff(&retyped);
    assert!(diff.has_breaking_changes());
    assert!(
        diff.changes.iter().any(|change| {
            change.path == path && change.detail == "request header type changed"
        })
    );
    assert!(diff.changes.iter().any(|change| {
        change.path == path && change.detail == "request header metadata changed"
    }));
}

#[test]
fn operation_documentation_and_deprecation_are_reported() {
    let old: Document = serde_json::from_value(fixture()).unwrap();
    let mut documented = old.clone();
    documented.operations[0].doc = Some(doc(Some("Sign in"), None));
    let change = change_at(&old, &documented, "operations.auth.login.doc");
    assert_eq!(change.level, ChangeLevel::Info);
    assert_eq!(change.detail, "operation documentation changed");
    assert!(!old.diff(&documented).has_breaking_changes());

    let mut deprecated = documented.clone();
    deprecated.operations[0].doc = Some(doc(Some("Sign in"), Some(true)));
    let change = change_at(&documented, &deprecated, "operations.auth.login.doc");
    assert_eq!(change.level, ChangeLevel::Warning);
    assert_eq!(change.detail, "operation deprecation changed");
    assert!(!documented.diff(&deprecated).has_breaking_changes());

    let mut unannotated = documented.clone();
    unannotated.operations[0].doc = None;
    let change = change_at(&documented, &unannotated, "operations.auth.login.doc");
    assert_eq!(change.level, ChangeLevel::Info);
    assert_eq!(change.detail, "operation documentation changed");
}
