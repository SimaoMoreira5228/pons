use super::*;

#[test]
fn type_field_and_variant_documentation_is_compared() {
    let old: Document = serde_json::from_value(fixture()).unwrap();
    let mut deprecated = fixture();
    deprecated["types"]["User"]["doc"] = serde_json::json!({"deprecated": true});
    deprecated["types"]["User"]["fields"][0]["doc"] = serde_json::json!({"deprecated": true});
    deprecated["types"]["LoginError"]["variants"][0]["doc"] =
        serde_json::json!({"deprecated": true});
    let new: Document = serde_json::from_value(deprecated).unwrap();
    new.validate().unwrap();

    let diff = old.diff(&new);
    assert_eq!(diff.worst(), Some(ChangeLevel::Warning));
    for (path, label) in [
        ("types.User", "type"),
        ("types.User.fields.id", "field"),
        ("types.LoginError.variants.invalid", "enum variant"),
    ] {
        let change = diff
            .changes
            .iter()
            .find(|change| change.path == path)
            .unwrap_or_else(|| panic!("no change reported at {path}"));
        assert_eq!(change.level, ChangeLevel::Warning);
        assert_eq!(change.detail, format!("{label} deprecation changed"));
    }

    let mut summarized = fixture();
    summarized["types"]["User"]["doc"] = serde_json::json!({"summary": "an account"});
    let new: Document = serde_json::from_value(summarized).unwrap();
    let change = change_at(&old, &new, "types.User");
    assert_eq!(change.level, ChangeLevel::Info);
    assert_eq!(change.detail, "type documentation changed");
    assert!(!old.diff(&new).has_breaking_changes());
}

#[test]
fn request_body_auth_and_provision_metadata_are_warnings() {
    let old: Document = serde_json::from_value(fixture()).unwrap();
    let mut annotated = old.clone();
    annotated.operations[0]
        .body
        .as_mut()
        .unwrap()
        .ext
        .insert("test.example.source/v1".into(), serde_json::json!(1));
    annotated.operations[0]
        .auth
        .ext
        .insert("test.example.source/v1".into(), serde_json::json!(1));
    annotated.operations[0]
        .provides
        .as_mut()
        .unwrap()
        .ext
        .insert("test.example.source/v1".into(), serde_json::json!(1));

    let diff = old.diff(&annotated);
    assert_eq!(diff.worst(), Some(ChangeLevel::Warning));
    assert!(!diff.has_breaking_changes());
    for (path, detail) in [
        (
            "operations.auth.login.body",
            "request body metadata changed",
        ),
        (
            "operations.auth.login.auth",
            "authentication requirement metadata changed",
        ),
        (
            "operations.auth.login.provides",
            "credential provision metadata changed",
        ),
    ] {
        let change = diff
            .changes
            .iter()
            .find(|change| change.path == path)
            .unwrap_or_else(|| panic!("no change reported at {path}"));
        assert_eq!(change.level, ChangeLevel::Warning);
        assert_eq!(change.detail, detail);
    }

    let mut retyped = annotated;
    retyped.operations[0].body.as_mut().unwrap().media = Some(
        crate::MediaDescriptor::from_declared("application/vnd.acme+json"),
    );
    let diff = old.diff(&retyped);
    let body = diff
        .changes
        .iter()
        .filter(|change| change.path == "operations.auth.login.body")
        .collect::<Vec<_>>();
    assert!(
        body.iter()
            .any(|change| change.level == ChangeLevel::Breaking
                && change.detail == "request body changed")
    );
    assert!(
        body.iter()
            .any(|change| change.level == ChangeLevel::Warning
                && change.detail == "request body metadata changed")
    );
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
fn opaque_type_metadata_is_reported_as_a_warning() {
    let old: Document = serde_json::from_value(fixture()).unwrap();
    let mut changed = fixture();
    changed["types"]["User"]["ext"]["test.example.source/v1"] = serde_json::json!("dropped");
    assert_eq!(level(&old, changed, "types.User"), ChangeLevel::Warning);

    let mut changed = fixture();
    changed["types"]["User"]["fields"][0]["ext"]["test.example.source/v1"] = serde_json::json!(2);
    assert_eq!(
        level(&old, changed, "types.User.fields.id"),
        ChangeLevel::Warning
    );

    let mut changed = fixture();
    changed["types"]["LoginError"]["variants"][1]["ext"]["test.example.source/v1"] =
        serde_json::json!(true);
    assert_eq!(
        level(&old, changed, "types.LoginError.variants.locked"),
        ChangeLevel::Warning
    );
}

#[test]
fn operation_metadata_changes_are_reported_as_a_warning() {
    let old: Document = serde_json::from_value(fixture()).unwrap();
    let mut new = old.clone();
    new.operations[0].ext.insert(
        "test.example.source/v1".into(),
        serde_json::json!("dropped"),
    );
    let change = change_at(&old, &new, "operations.auth.login.ext");
    assert_eq!(change.level, ChangeLevel::Warning);
    assert_eq!(change.detail, "operation metadata changed");
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
                adapter: None,
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
            adapter: None,
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
    *ext = BTreeMap::from([("test.example.source/v1".into(), serde_json::json!(1))]);
    let change = change_at(&described, &extended, "auth_schemes.custom");
    assert_eq!(change.level, ChangeLevel::Warning);
    assert_eq!(change.detail, "custom scheme metadata changed");
    assert!(!described.diff(&extended).has_breaking_changes());
}

#[test]
fn response_metadata_changes_follow_metadata_levels() {
    let old: Document = serde_json::from_value(fixture()).unwrap();
    let mut extended = old.clone();
    extended.operations[0].responses[0].ext.insert(
        "test.example.source/v1".into(),
        serde_json::json!("dropped"),
    );
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
        .insert("test.example.source/v1".into(), serde_json::json!(1));
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
        .insert("test.example.source/v1".into(), serde_json::json!(1));
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
        .insert("test.example.source/v1".into(), serde_json::json!(1));
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
