mod support;

use pons_ir::TypeKind;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use support::{fixture, refuses};

#[test]
fn a_query_parameter_and_a_request_header_cannot_name_an_api_key_carrier() {
    let with_carrier = |location: &str, name: &str, collision: &str| {
        let mut document = fixture();
        document.auth_schemes.remove("tenant");
        document.auth_schemes.insert(
            "bearer_key".into(),
            serde_json::from_value(json!({
                "kind": "api_key", "in": location, "name": name
            }))
            .unwrap(),
        );
        document.operations[1].auth.alternatives = vec![vec!["bearer_key".into()]];
        document.operations[1].headers[0].wire = collision.into();
        if location == "query" {
            let TypeKind::Struct { fields, .. } =
                &mut document.types.get_mut("EventQuery").unwrap().definition
            else {
                panic!("EventQuery is not a struct")
            };
            fields[0].wire = collision.into();
        }
        document
    };

    refuses(
        &with_carrier("header", "X-Tenant", "x-tenant"),
        "the request header x-tenant is also the bearer_key apiKey carrier name, so the credential overwrites it",
    );
    refuses(
        &with_carrier("query", "limit", "limit"),
        "the query field \"limit\" is also the bearer_key apiKey carrier name, so the credential overwrites it",
    );
    for accepted in [
        ("header", "X-Other", "X-Tenant"),
        ("query", "cursor", "limit"),
        ("cookie", "limit", "limit"),
        ("query", "Limit", "limit"),
    ] {
        with_carrier(accepted.0, accepted.1, accepted.2)
            .validate()
            .unwrap();
    }
}

#[test]
fn a_credential_carrier_collides_only_where_an_auth_group_names_its_scheme() {
    let with_scheme = |id: &str, scheme: Value, group: Option<usize>| {
        let mut document = fixture();
        document
            .auth_schemes
            .insert(id.into(), serde_json::from_value(scheme).unwrap());
        if let Some(index) = group {
            document.operations[index].auth.alternatives = vec![vec![id.into()]];
        }
        document
    };
    let header = |name: &str| json!({"kind": "api_key", "in": "header", "name": name});

    with_scheme("unused", header("X-Request-ID"), None)
        .validate()
        .unwrap();
    with_scheme("unused", header("X-Request-ID"), Some(0))
        .validate()
        .unwrap();
    refuses(
        &with_scheme("wanted", header("X-Request-ID"), Some(1)),
        "the request header X-Request-ID is also the wanted apiKey carrier name",
    );

    let same_carrier = |alternatives: Vec<Vec<&str>>| {
        let mut document = with_scheme("key", header("x-tenant"), None);
        document.auth_schemes.insert(
            "spare".into(),
            serde_json::from_value(header("X-Other")).unwrap(),
        );
        document.operations[1].auth.alternatives = alternatives
            .into_iter()
            .map(|group| group.into_iter().map(str::to_owned).collect())
            .collect();
        document
    };
    refuses(
        &same_carrier(vec![vec!["tenant", "key"]]),
        "the key and tenant schemes of one auth alternative both write the Header carrier \"x-tenant\"",
    );
    same_carrier(vec![vec!["tenant"], vec!["key"]])
        .validate()
        .unwrap();
    same_carrier(vec![vec!["tenant", "spare"]])
        .validate()
        .unwrap();

    let mut cookies = with_scheme(
        "sid",
        json!({"kind": "cookie_session", "cookie": "sid"}),
        None,
    );
    cookies.auth_schemes.insert(
        "key".into(),
        serde_json::from_value(json!({"kind": "api_key", "in": "cookie", "name": "sid"})).unwrap(),
    );
    cookies.operations[1].auth.alternatives = vec![vec!["key".into(), "sid".into()]];
    refuses(
        &cookies,
        "the sid and key schemes of one auth alternative both write the Cookie carrier \"sid\"",
    );
}

#[test]
fn an_operation_scope_the_flows_do_not_declare_is_refused() {
    let oauth = |scopes: Value| {
        let mut document = fixture();
        document.auth_schemes.insert(
            "oauth".into(),
            serde_json::from_value(json!({
                "kind": "o_auth2",
                "flows": [{
                    "kind": "client_credentials",
                    "token_url": "https://auth.example.test/token",
                    "scopes": scopes
                }]
            }))
            .unwrap(),
        );
        document.operations[1].auth.alternatives = vec![vec!["oauth".into()]];
        document.operations[1].auth.scopes =
            BTreeMap::from([("oauth".into(), vec!["read".into()])]);
        document
    };

    oauth(json!({"read": "Read", "write": "Write"}))
        .validate()
        .unwrap();
    refuses(
        &oauth(json!({"write": "Write"})),
        "the OAuth2 scheme oauth does not declare the read scope",
    );
}

#[test]
fn an_oauth_flow_url_must_be_an_absolute_https_url() {
    let flow = |kind: &str, url: &str| {
        let mut document = fixture();
        document.auth_schemes.insert(
            "oauth".into(),
            serde_json::from_value(json!({
                "kind": "o_auth2",
                "flows": [{ "kind": kind, "authorization_url": url, "token_url": url }]
            }))
            .unwrap(),
        );
        document
    };

    for url in [
        "https://auth.example.test/authorize",
        "https://auth.example.test/authorize?audience=api",
        "https://user@auth.example.test/authorize",
    ] {
        flow("authorization_code", url).validate().unwrap();
    }
    for url in [
        "/authorize",
        "http://auth.example.test/authorize",
        "https://",
        "",
    ] {
        refuses(
            &flow("authorization_code", url),
            "OAuth2 authorization URL for oauth must be an absolute HTTPS URL",
        );
        refuses(
            &flow("client_credentials", url),
            "OAuth2 token URL for oauth must be an absolute HTTPS URL",
        );
    }
}

#[test]
fn scopes_are_legal_on_every_auth_scheme() {
    let with_scopes = |schemes: Value, scopes: &[&str]| {
        let mut document = fixture();
        document
            .auth_schemes
            .extend(serde_json::from_value::<BTreeMap<String, _>>(schemes).unwrap());
        document.operations[1].auth.alternatives = vec![vec!["tenant".into()]];
        document.operations[1].auth.scopes = BTreeMap::from([(
            "tenant".to_owned(),
            scopes.iter().map(|scope| (*scope).to_owned()).collect(),
        )]);
        document
    };
    let oauth = |scopes: Value| {
        json!({
            "tenant": {
                "kind": "o_auth2",
                "flows": [{
                    "kind": "client_credentials",
                    "token_url": "https://auth.example.test/token",
                    "scopes": scopes
                }]
            }
        })
    };

    for scheme in [
        json!({"tenant": {"kind": "bearer", "format": "jwt"}}),
        json!({"tenant": {"kind": "api_key", "in": "header", "name": "X-Tenant"}}),
        json!({"tenant": {"kind": "custom", "description": "per tenant"}}),
        oauth(json!({"read": "Read"})),
    ] {
        with_scopes(scheme, &["read"]).validate().unwrap();
    }

    refuses(
        &with_scopes(oauth(json!({"write": "Write"})), &["read"]),
        "the OAuth2 scheme tenant does not declare the read scope",
    );
    refuses(
        &with_scopes(
            json!({"tenant": {"kind": "bearer", "format": "jwt"}}),
            &["read", "read"],
        ),
        "auth scheme tenant repeats scope read",
    );
    refuses(
        &with_scopes(json!({"tenant": {"kind": "bearer", "format": "jwt"}}), &[]),
        "auth scheme tenant has an empty scope list",
    );
}
