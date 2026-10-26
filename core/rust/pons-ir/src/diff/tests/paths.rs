use super::*;

#[test]
fn error_change_paths_are_injective() {
    let old: Document = serde_json::from_value(fixture()).unwrap();

    let mut statuses = old.clone();
    statuses.operations[0].errors[0].doc = Some(doc(Some("Login failed"), None));
    let mut unavailable = statuses.operations[0].errors[0].clone();
    unavailable.status = crate::ErrorStatus::Class(5);
    statuses.operations[0].errors.push(unavailable);
    statuses.validate().unwrap();

    let mut paths = old
        .diff(&statuses)
        .changes
        .iter()
        .map(|change| change.path.clone())
        .collect::<Vec<_>>();
    paths.sort();
    assert_eq!(
        paths,
        vec![
            "operations.auth.login.errors.5xx.login_error".to_owned(),
            "operations.auth.login.errors.login_error".to_owned(),
        ]
    );

    let mut names = old.clone();
    let mut parts = names.operations[0].errors[0].clone();
    parts.name = vec!["a".into(), "b".into()];
    let mut joined = parts.clone();
    joined.status = crate::ErrorStatus::Class(5);
    joined.name = vec!["a_b".into()];
    names.operations[0].errors = vec![parts, joined];
    names.validate().unwrap();

    let mut paths = old
        .diff(&names)
        .changes
        .iter()
        .map(|change| change.path.clone())
        .collect::<Vec<_>>();
    paths.sort();
    assert_eq!(
        paths,
        vec![
            "operations.auth.login.errors.5xx.a%5Fb".to_owned(),
            "operations.auth.login.errors.a_b".to_owned(),
            "operations.auth.login.errors.login_error".to_owned(),
        ]
    );
}

#[test]
fn change_paths_stay_unique_across_the_lists_that_carry_them() {
    let record = |id: &str, wires: &[&str]| {
        serde_json::from_value(serde_json::json!({
            "kind": "struct",
            "name": [id],
            "direction": "output",
            "openness": "reject",
            "fields": wires.iter().map(|wire| serde_json::json!({
                "wire": wire,
                "ident": [wire],
                "ty": {"kind": "string"},
                "presence": "optional"
            })).collect::<Vec<_>>()
        }))
        .unwrap()
    };

    let mut old: Document = serde_json::from_value(fixture()).unwrap();
    old.types
        .insert("A".into(), record("A", &["x", "B.fields.c"]));
    old.types
        .insert("A.fields.B".into(), record("A_fields_B", &["x", "c"]));
    old.operations[0].errors[0].name = vec!["x".into()];
    let mut nested = old.operations[0].clone();
    nested.id = vec![
        vec!["auth".into()],
        vec!["login".into()],
        vec!["errors".into()],
        vec!["x".into()],
    ];
    nested.path = vec![
        crate::PathSegment::Literal {
            lit: "errors".into(),
        },
        crate::PathSegment::Param {
            param: "x".into(),
            ty: TypeRef::Inline(InlineType::Bool),
        },
    ];
    old.operations.push(nested);
    for id in ["a", "a.flows"] {
        old.auth_schemes.insert(
            id.into(),
            AuthScheme::OAuth2 {
                flows: vec![OAuthFlow {
                    kind: crate::OAuthFlowKind::Password,
                    authorization_url: None,
                    token_url: Some("https://identity.example.test/token".into()),
                    scopes: BTreeMap::new(),
                    ext: BTreeMap::new(),
                }],
            },
        );
    }
    old.validate().unwrap();

    let mut new = old.clone();
    for (id, wire) in [("A", "B.fields.c"), ("A.fields.B", "c")] {
        let TypeKind::Struct { fields, .. } = &mut new.types.get_mut(id).unwrap().definition else {
            unreachable!()
        };
        fields.retain(|field| field.wire != wire);
    }
    new.operations[0].errors.clear();
    new.operations.pop();
    for id in ["a", "a.flows"] {
        let AuthScheme::OAuth2 { flows } = new.auth_schemes.get_mut(id).unwrap() else {
            unreachable!()
        };
        flows[0].token_url = Some("https://identity.example.test/v2/token".into());
    }
    new.validate().unwrap();

    let changes = old.diff(&new).changes;
    let mut paths = changes
        .iter()
        .map(|change| change.path.as_str())
        .collect::<Vec<_>>();
    let count = paths.len();
    paths.sort_unstable();
    paths.dedup();
    assert_eq!(paths.len(), count, "{changes:#?}");
    for (path, detail) in [
        ("types.A.fields.B.fields.c", "field removed"),
        ("types.A%2Efields%2EB.fields.c", "field removed"),
        ("operations.auth.login.errors.x", "operation removed"),
        ("operations.auth.login.errors.4xx.x", "error removed"),
        ("auth_schemes.a.flows.Password", "OAuth2 token URL changed"),
        (
            "auth_schemes.a.flows.flows.Password",
            "OAuth2 token URL changed",
        ),
    ] {
        assert!(
            changes
                .iter()
                .any(|change| change.path == path && change.detail == detail),
            "no {detail:?} reported at {path} in {changes:#?}"
        );
    }
}

#[test]
fn a_key_with_a_dot_prints_readable_and_escapes_only_on_a_collision() {
    let record = |id: &str, wires: &[&str]| {
        serde_json::from_value(serde_json::json!({
            "kind": "struct",
            "name": [id],
            "direction": "output",
            "openness": "reject",
            "fields": wires.iter().map(|wire| serde_json::json!({
                "wire": wire,
                "ident": [wire],
                "ty": {"kind": "string"},
                "presence": "optional"
            })).collect::<Vec<_>>()
        }))
        .unwrap()
    };
    let mut old: Document = serde_json::from_value(fixture()).unwrap();
    old.types
        .insert("A".into(), record("A", &["%b.fields.c", "f_o.fields.c"]));
    old.types
        .insert("A.fields.%b".into(), record("A_fields_percent_b", &["c"]));
    old.types
        .insert("A.fields.f_o".into(), record("A_fields_f_o", &["c"]));

    let mut new = old.clone();
    new.types.remove("A.fields.f_o");
    old.validate().unwrap();
    new.validate().unwrap();
    assert_eq!(
        change_at(&old, &new, "types.A.fields.f_o").detail,
        "type removed"
    );

    let mut new = old.clone();
    for (id, wire) in [
        ("A", "%b.fields.c"),
        ("A", "f_o.fields.c"),
        ("A.fields.%b", "c"),
        ("A.fields.f_o", "c"),
    ] {
        let TypeKind::Struct { fields, .. } = &mut new.types.get_mut(id).unwrap().definition else {
            unreachable!()
        };
        fields.retain(|field| field.wire != wire);
    }
    old.validate().unwrap();
    new.validate().unwrap();
    let removed = old
        .diff(&new)
        .changes
        .into_iter()
        .filter(|change| change.detail == "field removed")
        .map(|change| change.path)
        .collect::<Vec<_>>();
    assert_eq!(
        removed,
        [
            "types.A.fields.%b.fields.c",
            "types.A.fields.f_o.fields.c",
            "types.A%2Efields%2E%25b.fields.c",
            "types.A%2Efields%2Ef%5Fo.fields.c",
        ]
    );
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
