use pons_ir::{AuthScheme, Document, OAuthFlow, OAuthFlowKind};

#[test]
fn validation_rejects_empty_auth_alternatives() {
    let mut document: Document =
        serde_json::from_str(include_str!("../../../../spec/fixtures/login.json")).unwrap();
    document.operations[0].auth.alternatives = vec![vec![]];
    let errors = document.validate().unwrap_err();
    assert!(errors.iter().any(|error| {
        error
            .to_string()
            .contains("auth alternative cannot be empty")
    }));
}

#[test]
fn validation_rejects_invalid_oauth_flow_sets() {
    let base: Document =
        serde_json::from_str(include_str!("../../../../spec/fixtures/login.json")).unwrap();
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
