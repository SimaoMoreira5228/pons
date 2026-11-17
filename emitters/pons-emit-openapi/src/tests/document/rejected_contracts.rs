use pons_ir::{AuthScheme, OAuthFlow, OAuthFlowKind, PathSegment};

use super::super::{fixture, string};

#[test]
fn rejects_operation_id_collisions() {
    let mut document = fixture();
    document.operations[0].id = vec![vec!["same".into()], vec!["part".into()]];
    document.operations[1].id = vec![vec!["same.part".into()]];
    assert!(
        crate::emit(&document)
            .unwrap_err()
            .0
            .contains("duplicate OpenAPI operationId same.part")
    );
}

#[test]
fn rejects_auth_and_stream_contracts_openapi_cannot_describe() {
    let mut document = fixture();
    document.auth_schemes.insert(
        "tenant".into(),
        AuthScheme::Custom {
            description: "custom auth".into(),
            adapter: None,
            ext: Default::default(),
        },
    );
    assert!(
        crate::emit(&document)
            .unwrap_err()
            .0
            .contains("custom auth scheme")
    );

    let mut document = fixture();
    document
        .operations
        .iter_mut()
        .find(|operation| operation.stream.is_some())
        .unwrap()
        .responses
        .clear();
    assert!(
        crate::emit(&document)
            .unwrap_err()
            .0
            .contains("1 stream descriptions and 0 stream responses")
    );
}

#[test]
fn rejects_ambiguous_paths_headers_and_oauth_urls() {
    let mut document = fixture();
    document.operations[0].path[0] = PathSegment::Literal {
        lit: "auth/{login}".into(),
    };
    assert!(
        crate::emit(&document)
            .unwrap_err()
            .0
            .contains("template syntax")
    );

    let mut document = fixture();
    document.operations[0].path = vec![
        PathSegment::Literal {
            lit: "events".into(),
        },
        PathSegment::Param {
            param: "id".into(),
            ty: string(),
        },
    ];
    document.operations[1].path = vec![
        PathSegment::Literal {
            lit: "events".into(),
        },
        PathSegment::Param {
            param: "name".into(),
            ty: string(),
        },
    ];
    assert!(
        crate::emit(&document)
            .unwrap_err()
            .0
            .contains("OpenAPI route pattern /events/{} uses conflicting parameter names")
    );

    let mut document = fixture();
    document.operations[1].path = vec![PathSegment::Param {
        param: "bad/name".into(),
        ty: string(),
    }];
    assert!(
        crate::emit(&document)
            .unwrap_err()
            .0
            .contains("not a valid OpenAPI path parameter name")
    );

    let mut document = fixture();
    document.operations[1].headers[0].wire = "Accept".into();
    assert!(
        crate::emit(&document)
            .unwrap_err()
            .0
            .contains("request header Accept is owned by the request body or the security model")
    );

    let mut document = fixture();
    document.auth_schemes.insert(
        "oauth".into(),
        AuthScheme::OAuth2 {
            flows: vec![OAuthFlow {
                kind: OAuthFlowKind::ClientCredentials,
                authorization_url: None,
                token_url: Some("http://identity.example.test/token".into()),
                scopes: Default::default(),
                ext: Default::default(),
            }],
        },
    );
    assert!(
        crate::emit(&document)
            .unwrap_err()
            .0
            .contains("OAuth2 token URL for oauth must be an absolute HTTPS URL")
    );

    let mut document = fixture();
    document.auth_schemes.insert(
        "oauth".into(),
        AuthScheme::OAuth2 {
            flows: vec![OAuthFlow {
                kind: OAuthFlowKind::ClientCredentials,
                authorization_url: None,
                token_url: Some("https://?query".into()),
                scopes: Default::default(),
                ext: Default::default(),
            }],
        },
    );
    assert!(
        crate::emit(&document)
            .unwrap_err()
            .0
            .contains("OAuth2 token URL for oauth must be an absolute HTTPS URL")
    );
}
