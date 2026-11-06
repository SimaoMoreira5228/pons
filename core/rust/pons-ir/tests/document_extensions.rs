mod support;

use pons_ir::{Diagnostic, Document, InlineType, OpaqueWire, TypeKind, TypeRef};
use std::collections::BTreeMap;
use support::{fixture, refuses, rejections};

#[test]
fn extension_keys_carry_a_reverse_dns_namespace_and_a_version() {
    let with_key = |key: &str| {
        let mut document = fixture();
        document
            .ext
            .insert(key.to_owned(), serde_json::json!({"kept": true}));
        document
    };

    for key in [
        "com.example.cursor/v1",
        "test.example.source/v1",
        "a.b-c.d-e/v0",
        "x.y1.z1/v12",
        "com.pons.alias/v1",
        "com.pons.open_variant/v1",
    ] {
        with_key(key).validate().unwrap_or_else(|errors| {
            panic!("{key} should be accepted: {errors:?}");
        });
    }

    for (key, message) in [
        ("source.note", "is not shaped reverse-dns.namespace/vN"),
        ("com.example/v1", "is not shaped reverse-dns.namespace/vN"),
        (
            "com.example.cursor",
            "is not shaped reverse-dns.namespace/vN",
        ),
        (
            "com.example.cursor/extra/v1",
            "is not shaped reverse-dns.namespace/vN",
        ),
        (
            "com.example.cursor/v",
            "is not shaped reverse-dns.namespace/vN",
        ),
        (
            "com.example.cursor/v01",
            "is not shaped reverse-dns.namespace/vN",
        ),
        (
            "com.example.cursor/1",
            "is not shaped reverse-dns.namespace/vN",
        ),
        ("com.example./v1", "is not shaped reverse-dns.namespace/vN"),
        ("comexample/v1", "is not shaped reverse-dns.namespace/vN"),
        (
            "Com.Example/cursor/v1",
            "is not shaped reverse-dns.namespace/vN",
        ),
        (
            "com.example.Cursor/v1",
            "is not shaped reverse-dns.namespace/vN",
        ),
        (
            "com.example.curs-or/v1/x",
            "is not shaped reverse-dns.namespace/vN",
        ),
        (
            "x-pons.internal/cursor/v1",
            "starts with \"x-pons\", which this project reserves for itself",
        ),
        (
            "x-pons",
            "starts with \"x-pons\", which this project reserves for itself",
        ),
    ] {
        refuses(&with_key(key), message);
    }
}

#[test]
fn every_object_carrying_an_extension_names_its_owner() {
    let mut document = fixture();
    let bad = |key: &str| BTreeMap::from([(key.to_owned(), serde_json::json!(1))]);
    document.ext = bad("note");
    document.info.ext = bad("note");
    document.types.get_mut("User").unwrap().ext = bad("note");
    document.auth_schemes.insert(
        "custom".into(),
        pons_ir::AuthScheme::Custom {
            description: "d".into(),
            adapter: None,
            ext: bad("note"),
        },
    );
    document.operations[0].ext = bad("note");
    document.operations[0].auth.ext = bad("note");
    document.operations[0].body.as_mut().unwrap().ext = bad("note");
    document.operations[0].provides.as_mut().unwrap().ext = bad("note");
    document.operations[0].errors[0].ext = bad("note");
    document.operations[0].responses[0].ext = bad("note");
    document.operations[1].headers[0].ext = bad("note");
    document
        .common_errors
        .push(document.operations[0].errors[0].clone());
    document.diagnostics.push(Diagnostic {
        severity: pons_ir::DiagnosticSeverity::Info,
        code: "C".into(),
        message: "m".into(),
        origin: None,
        fidelity: None,
        capability: None,
        ext: bad("note"),
    });

    let found = rejections(&document);
    for owner in [
        "document:",
        "info:",
        "type User:",
        "auth scheme custom:",
        "operations[0]:",
        "operations[0].auth:",
        "operations[0].body:",
        "operations[0].provides:",
        "operations[0].errors[0]:",
        "operations[0].responses[0]:",
        "operations[1].headers[0]:",
        "common_errors[0]:",
        "document diagnostics[0]:",
    ] {
        assert!(
            found
                .iter()
                .any(|error| error.starts_with(&format!("{owner} extension key \"note\""))),
            "{owner} did not report the key: {found:?}"
        );
    }
}

#[test]
fn an_opaque_value_carries_its_own_extensions() {
    let mut document = fixture();
    let TypeKind::Struct { fields, .. } = &mut document.types.get_mut("User").unwrap().definition
    else {
        panic!()
    };
    fields[0].ty = TypeRef::Inline(InlineType::Opaque {
        codec: "example".into(),
        wire: pons_ir::OpaqueWire::Json,
        ext: BTreeMap::from([("com.example.codec/v1".into(), serde_json::json!(true))]),
    });
    document.validate().unwrap();

    let raw = serde_json::to_value(&document).unwrap();
    assert_eq!(
        raw["types"]["User"]["fields"][0]["ty"]["ext"]["com.example.codec/v1"],
        serde_json::json!(true)
    );
    assert!(
        raw["types"]["User"]["fields"][0]["ty"]["ext"]
            .as_object()
            .is_some_and(|ext| !ext.is_empty())
    );

    let TypeKind::Struct { fields, .. } = &mut document.types.get_mut("User").unwrap().definition
    else {
        panic!()
    };
    let TypeRef::Inline(InlineType::Opaque { ext, .. }) = &mut fields[0].ty else {
        panic!()
    };
    ext.insert("note".into(), serde_json::json!(true));
    refuses(
        &document,
        "extension key \"note\" is not shaped reverse-dns.namespace/vN",
    );
}

#[test]
fn an_opaque_value_must_name_a_codec() {
    let mut document = fixture();
    let TypeKind::Struct { fields, .. } = &mut document.types.get_mut("User").unwrap().definition
    else {
        panic!()
    };
    fields[0].ty = TypeRef::Inline(InlineType::Opaque {
        codec: "   ".into(),
        wire: pons_ir::OpaqueWire::Json,
        ext: BTreeMap::new(),
    });
    refuses(&document, "opaque codec cannot be empty");
}

#[test]
fn opaque_values_require_a_codec_and_can_be_scanned() {
    let mut document: Document =
        serde_json::from_str(include_str!("../../../../spec/fixtures/login.json")).unwrap();
    let user = document.types.get_mut("User").unwrap();
    let TypeKind::Struct { fields, .. } = &mut user.definition else {
        panic!()
    };
    fields[0].ty = TypeRef::Inline(InlineType::Opaque {
        codec: String::new(),
        wire: OpaqueWire::Unknown,
        ext: Default::default(),
    });
    let errors = document.validate().unwrap_err();
    assert!(
        errors
            .iter()
            .any(|error| error.to_string().contains("opaque codec cannot be empty"))
    );
    assert!(document.has_opaque());
    let TypeKind::Struct { fields, .. } = &mut document.types.get_mut("User").unwrap().definition
    else {
        panic!()
    };
    fields[0].ty = TypeRef::Inline(InlineType::Opaque {
        codec: "example".into(),
        wire: OpaqueWire::Json,
        ext: Default::default(),
    });
    document.validate().unwrap();
}

#[test]
fn common_errors_only_root_opaque_when_an_operation_inherits_them() {
    let mut document: Document =
        serde_json::from_str(include_str!("../../../../spec/fixtures/login.json")).unwrap();
    let mut common = document.operations[0].errors[0].clone();
    common.name = vec!["common".into(), "error".into()];
    common.body = Some(TypeRef::Inline(InlineType::Opaque {
        codec: "example".into(),
        wire: OpaqueWire::Json,
        ext: Default::default(),
    }));
    document.common_errors.push(common);
    document.operations[0].errors.clear();
    document
        .operations
        .iter_mut()
        .for_each(|operation| operation.errors_inherit = false);
    document.validate().unwrap();
    assert!(document.has_opaque());
    assert!(!document.has_reachable_opaque());

    document.operations[0].errors_inherit = true;
    assert!(document.has_reachable_opaque());
}
