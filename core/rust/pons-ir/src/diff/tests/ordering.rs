use super::*;

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
fn flattened_reordering_has_no_diff() {
    let member = |id: &str| {
        serde_json::json!({
            "kind": "struct",
            "name": [id],
            "direction": "output",
            "openness": "reject",
            "fields": [{
                "wire": id,
                "ident": [id],
                "ty": {"kind": "string"},
                "presence": "optional"
            }]
        })
    };
    let mut old: Document = serde_json::from_value(fixture()).unwrap();
    for id in ["Meta", "Trace"] {
        old.types
            .insert(id.into(), serde_json::from_value(member(id)).unwrap());
    }
    let TypeKind::Struct { flattened, .. } = &mut old.types.get_mut("User").unwrap().definition
    else {
        unreachable!()
    };
    *flattened = vec![
        TypeRef::Named(crate::NamedRef {
            id: "Meta".into(),
            variant: None,
        }),
        TypeRef::Named(crate::NamedRef {
            id: "Trace".into(),
            variant: None,
        }),
    ];

    let mut new = old.clone();
    let TypeKind::Struct { flattened, .. } = &mut new.types.get_mut("User").unwrap().definition
    else {
        unreachable!()
    };
    flattened.reverse();
    old.validate().unwrap();
    new.validate().unwrap();

    assert_eq!(old.diff(&new), Diff::default());
}

#[test]
fn worst_reports_the_highest_level() {
    let old: Document = serde_json::from_value(fixture()).unwrap();
    assert_eq!(old.diff(&old).worst(), None);

    let mut warned = old.clone();
    warned.operations[0]
        .ext
        .insert("test.example.source/v1".into(), serde_json::json!(1));
    let diff = old.diff(&warned);
    assert_eq!(diff.worst(), Some(ChangeLevel::Warning));
    assert!(!diff.has_breaking_changes());

    let mut broken = warned;
    broken.operations[0].semantics.idempotent = true;
    let diff = old.diff(&broken);
    assert_eq!(diff.worst(), Some(ChangeLevel::Breaking));
    assert!(diff.has_breaking_changes());
}
