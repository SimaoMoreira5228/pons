use pons_ir::{ApiKeyLocation, AuthScheme, OAuthFlowKind};

use super::{FIXTURE, document, operation};

#[test]
fn maps_auth_alternatives_and_scopes() {
    let document = document();
    let get = operation(&document, "orders.get");
    assert_eq!(get.auth.alternatives, vec![vec!["bearerAuth"]]);
    assert!(get.auth.scopes.is_empty());
    assert!(!get.auth.optional);

    let cancel = operation(&document, "orders.cancel");
    assert_eq!(
        cancel.auth.alternatives,
        vec![vec!["bearerAuth"], vec!["oauth"]]
    );
    assert_eq!(cancel.auth.scopes["oauth"], ["orders:read"]);
    assert!(!cancel.auth.optional);

    let check = operation(&document, "orders.check");
    assert!(check.auth.alternatives.is_empty());
    assert!(!check.auth.optional);

    assert_eq!(
        document.auth_schemes["bearerAuth"],
        AuthScheme::Bearer {
            format: Some("JWT".into()),
        }
    );
    let AuthScheme::OAuth2 { flows } = &document.auth_schemes["oauth"] else {
        panic!("expected an oauth2 scheme");
    };
    assert_eq!(flows.len(), 1);
    assert_eq!(flows[0].kind, OAuthFlowKind::ClientCredentials);
    assert_eq!(
        flows[0].token_url.as_deref(),
        Some("https://auth.example.test/token")
    );
    assert_eq!(flows[0].scopes["orders:read"], "Read orders");
    assert_eq!(
        document.auth_schemes.get("missing"),
        None,
        "the fixture only declares the schemes it uses"
    );
}

#[test]
fn keeps_an_api_key_scheme_its_location() {
    let mut raw: serde_json::Value = serde_json::from_str(FIXTURE).unwrap();
    raw["components"]["securitySchemes"]["key"] = serde_json::json!({
        "type": "apiKey",
        "in": "header",
        "name": "X-API-Key"
    });
    raw["components"]["securitySchemes"]["alias"] = serde_json::json!({
        "$ref": "#/components/securitySchemes/key"
    });
    let document = pons_openapi::import(&serde_json::to_vec(&raw).unwrap()).unwrap();
    assert_eq!(
        document.auth_schemes["key"],
        AuthScheme::ApiKey {
            location: ApiKeyLocation::Header,
            name: "X-API-Key".into(),
        }
    );
    assert_eq!(document.auth_schemes["alias"], document.auth_schemes["key"]);
}
