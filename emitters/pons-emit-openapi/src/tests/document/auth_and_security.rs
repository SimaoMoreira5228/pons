use std::collections::BTreeMap;

use pons_ir::{AuthScheme, OAuthFlow, OAuthFlowKind};
use serde_json::{Value, json};

use super::super::fixture;

#[test]
fn emits_auth_scopes_only_for_oauth2_schemes() {
    let mut document = fixture();
    document.auth_schemes.insert(
        "oauth".into(),
        AuthScheme::OAuth2 {
            flows: vec![OAuthFlow {
                kind: OAuthFlowKind::AuthorizationCode,
                authorization_url: Some("https://auth.test/authorize".into()),
                token_url: Some("https://auth.test/token".into()),
                scopes: BTreeMap::from([("events:read".into(), "Read events".into())]),
                ext: Default::default(),
            }],
        },
    );
    let operation = document
        .operations
        .iter_mut()
        .find(|operation| operation.id == vec![vec!["events"], vec!["watch"]])
        .unwrap();
    operation.auth.alternatives = vec![vec!["oauth".into()], vec!["tenant".into()]];
    operation.auth.scopes = BTreeMap::from([
        ("oauth".into(), vec!["events:read".into()]),
        ("tenant".into(), vec!["ignored".into()]),
    ]);
    let output: Value = serde_json::from_str(&crate::emit(&document).unwrap()).unwrap();
    let security = &output["paths"]["/events/{cursor}"]["get"]["security"];
    assert_eq!(security[0]["Oauth"][0], "events:read");
    assert_eq!(security[1]["Tenant"], json!([]));
}
