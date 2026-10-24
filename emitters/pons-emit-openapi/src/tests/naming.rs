use std::collections::BTreeMap;

use pons_ir::{AuthScheme, Direction, NamedRef, TypeRef};
use serde_json::Value;

use super::{fixture, named_type};

#[test]
fn allocates_unique_schema_and_security_component_names() {
    let mut document = fixture();
    for (id, name) in [("A", "foo"), ("A2", "foo2"), ("Z", "foo")] {
        document.types.insert(id.into(), named_type(name));
    }
    document.auth_schemes.clear();
    for id in ["Tenant", "Tenant2", "session", "tenant"] {
        document
            .auth_schemes
            .insert(id.into(), AuthScheme::Bearer { format: None });
    }

    let output: Value = serde_json::from_str(&crate::emit(&document).unwrap()).unwrap();
    let schemas = &output["components"]["schemas"];
    assert!(schemas.get("Foo").is_some());
    assert!(schemas.get("Foo2").is_some());
    assert!(schemas.get("Foo3").is_some());
    let security = &output["components"]["securitySchemes"];
    assert!(security.get("Tenant").is_some());
    assert!(security.get("Tenant2").is_some());
    assert!(security.get("Tenant3").is_some(), "{security:#}");
}

fn carriers(definitions: &[(&str, &[&str], Direction, Option<&str>)]) -> pons_ir::Document {
    let mut document = fixture();
    for (id, name, direction, origin) in definitions {
        let mut definition = named_type("");
        definition.name = name.iter().map(|word| (*word).to_owned()).collect();
        definition.direction = *direction;
        definition.origin = origin.map(str::to_owned);
        document.types.insert((*id).into(), definition);
    }
    document
}

fn component_names(document: &pons_ir::Document) -> BTreeMap<String, String> {
    let output: Value = serde_json::from_str(&crate::emit(document).unwrap()).unwrap();
    output["components"]["schemas"]
        .as_object()
        .unwrap()
        .iter()
        .map(|(name, schema)| {
            (
                name.clone(),
                schema["x-pons"]["type"]["id"].as_str().unwrap().to_owned(),
            )
        })
        .collect()
}

#[test]
fn names_a_direction_pair_after_the_pair_and_never_after_the_id() {
    let mut document = carriers(&[
        (
            "router::Account",
            &["account"],
            Direction::Output,
            Some("router::Account"),
        ),
        (
            "storefront::AccountRequest",
            &["account", "request"],
            Direction::Input,
            Some("router::Account"),
        ),
    ]);
    let login = &mut document.operations[0];
    login.body.as_mut().unwrap().ty = TypeRef::Named(NamedRef {
        id: "storefront::AccountRequest".into(),
        variant: None,
    });
    login.responses[0].body = Some(TypeRef::Named(NamedRef {
        id: "router::Account".into(),
        variant: None,
    }));

    let output: Value = serde_json::from_str(&crate::emit(&document).unwrap()).unwrap();
    assert_eq!(
        output["paths"]["/auth/login"]["post"]["requestBody"]["content"]["application/json"]["schema"]
            ["$ref"],
        "#/components/schemas/AccountInput"
    );
    assert_eq!(
        output["paths"]["/auth/login"]["post"]["responses"]["200"]["content"]["application/json"]["schema"]
            ["$ref"],
        "#/components/schemas/Account"
    );

    let pair = component_names(&document);
    assert_eq!(pair["Account"], "router::Account");
    assert_eq!(
        pair["AccountInput"], "storefront::AccountRequest",
        "the request half takes the base display name the pair publishes"
    );
    assert!(
        pair.keys().all(|name| !name.contains('#')),
        "{pair:?} leaks an id"
    );

    let lone = component_names(&carriers(&[(
        "storefront::AccountRequest",
        &["account"],
        Direction::Input,
        Some("router::Account"),
    )]));
    assert_eq!(
        lone["AccountInput"], "storefront::AccountRequest",
        "pruning the response half leaves the request half the pair's name"
    );
}

#[test]
fn a_pair_that_declares_no_origin_keeps_its_own_display_name() {
    let names = component_names(&carriers(&[
        ("a::Account", &["account"], Direction::Output, None),
        ("b::Account", &["account"], Direction::Input, None),
    ]));
    assert_eq!(names["Account"], "a::Account");
    assert_eq!(names["Account2"], "b::Account");
    assert!(!names.contains_key("AccountInput"));
}

#[test]
fn a_suffixed_name_never_takes_the_name_another_type_publishes() {
    let names = component_names(&carriers(&[
        ("a::Q", &["q"], Direction::Output, Some("a::Q")),
        ("a::Q#input", &["q"], Direction::Input, Some("a::Q")),
        ("b::Q#input", &["q"], Direction::Input, Some("b::Q")),
        ("c::qInput", &["qInput"], Direction::Both, None),
    ]));
    assert_eq!(names["Q"], "a::Q");
    assert_eq!(names["QInput"], "c::qInput");
    assert_eq!(names["QInput2"], "a::Q#input");
    assert_eq!(names["QInput3"], "b::Q#input");
}
