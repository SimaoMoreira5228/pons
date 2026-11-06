mod support;

use pons_ir::{
    ContentKind, Direction, Document, Field, FieldDefault, InlineType, ObjectOpenness, OpaqueWire,
    Presence, TypeDef, TypeKind, TypeRef,
};
use std::collections::BTreeMap;
use support::named;

fn string() -> TypeRef {
    TypeRef::Inline(InlineType::String {
        format: None,
        min_length: None,
        max_length: None,
        pattern: None,
    })
}

fn array(item: TypeRef) -> TypeRef {
    TypeRef::Inline(InlineType::Array {
        item: Box::new(item),
        unique: None,
    })
}

fn nullable(inner: TypeRef) -> TypeRef {
    TypeRef::Inline(InlineType::Nullable {
        inner: Box::new(inner),
    })
}

fn map(value: TypeRef) -> TypeRef {
    TypeRef::Inline(InlineType::Map {
        key: Box::new(string()),
        value: Box::new(value),
    })
}

fn named_type(id: &'static str, definition: TypeKind) -> (&'static str, TypeDef) {
    (
        id,
        TypeDef {
            name: vec![id.into()],
            direction: Direction::Both,
            definition,
            origin: None,
            doc: None,
            diagnostics: Vec::new(),
            ext: BTreeMap::new(),
        },
    )
}

fn profile() -> (&'static str, TypeDef) {
    named_type(
        "Profile",
        TypeKind::Struct {
            fields: Vec::new(),
            flattened: Vec::new(),
            openness: ObjectOpenness::Reject,
        },
    )
}

fn form_document(fields: Vec<(&str, TypeRef)>, extra: Vec<(&str, TypeDef)>) -> Document {
    form_document_with(Presence::Required, fields, extra)
}

fn form_document_with(
    presence: Presence,
    fields: Vec<(&str, TypeRef)>,
    extra: Vec<(&str, TypeDef)>,
) -> Document {
    let mut document: Document =
        serde_json::from_str(include_str!("../../../../spec/fixtures/login.json")).unwrap();
    for (id, definition) in extra {
        document.types.insert(id.into(), definition);
    }
    document.types.insert(
        "FormBody".into(),
        TypeDef {
            name: vec!["form".into(), "body".into()],
            direction: Direction::Input,
            definition: TypeKind::Struct {
                fields: fields
                    .into_iter()
                    .map(|(wire, ty)| Field {
                        wire: wire.into(),
                        ident: vec![wire.into()],
                        ty,
                        presence,
                        default: FieldDefault::Unspecified,
                        access: None,
                        doc: None,
                        diagnostics: Vec::new(),
                        ext: BTreeMap::new(),
                    })
                    .collect(),
                flattened: Vec::new(),
                openness: ObjectOpenness::Reject,
            },
            origin: None,
            doc: None,
            diagnostics: Vec::new(),
            ext: BTreeMap::new(),
        },
    );
    let body = document.operations[0].body.as_mut().unwrap();
    body.content = ContentKind::Form;
    body.ty = named("FormBody");
    document
}

fn form_errors(document: &Document) -> Vec<String> {
    document
        .validate()
        .unwrap_err()
        .iter()
        .map(ToString::to_string)
        .collect()
}

fn refuses_form_field(document: &Document, wire: &str) {
    let errors = form_errors(document);
    let blamed = format!("form body field {wire:?}");
    assert!(
        errors.iter().any(|error| error.contains(&blamed)),
        "a form body value the carrier cannot spell is blamed by its own field: {errors:?}"
    );
}

fn refuses_form(document: &Document, message: &str) {
    let errors = form_errors(document);
    assert!(
        errors.iter().any(|error| error.contains(message)),
        "{message:?} missing from {errors:?}"
    );
}

#[test]
fn a_form_body_is_a_record_of_flat_values_and_nothing_else() {
    for (fields, extra, body) in [
        (vec![("email", string())], vec![], None),
        (
            vec![("email", string())],
            vec![named_type(
                "Wrapped",
                TypeKind::Newtype {
                    inner: named("FormBody"),
                },
            )],
            Some(named("Wrapped")),
        ),
    ] {
        let mut document = form_document(fields, extra);
        if let Some(body) = body {
            document.operations[0].body.as_mut().unwrap().ty = body;
        }
        document.validate().unwrap();
    }

    let mut opaque = form_document(vec![("email", string())], vec![]);
    opaque.operations[0].body.as_mut().unwrap().ty = TypeRef::Inline(InlineType::Opaque {
        codec: "axum.form".into(),
        wire: OpaqueWire::Unknown,
        ext: Default::default(),
    });
    refuses_form(
        &opaque,
        "a form body must be a struct or a transparent newtype over a struct, because a urlencoded form writes one key per field",
    );

    for (wire, fields, extra) in [
        (
            "profile",
            vec![("profile", named("Profile"))],
            vec![profile()],
        ),
        ("profile", vec![("profile", map(string()))], vec![]),
        (
            "profile",
            vec![("profile", named("WrappedProfile"))],
            vec![
                profile(),
                named_type(
                    "WrappedProfile",
                    TypeKind::Newtype {
                        inner: named("Profile"),
                    },
                ),
            ],
        ),
    ] {
        refuses_form_field(&form_document(fields, extra), wire);
    }

    refuses_form(
        &form_document(vec![("email", nullable(string()))], vec![]),
        "form body field \"email\" is required and nullable; a urlencoded form field has no encoding for a present null",
    );
    form_document_with(
        Presence::Optional,
        vec![("email", nullable(string()))],
        vec![],
    )
    .validate()
    .unwrap();
}

#[test]
fn a_form_body_carries_one_array_level_and_no_more() {
    form_document(vec![("tags", array(string()))], vec![])
        .validate()
        .unwrap();
    form_document_with(
        Presence::Optional,
        vec![("tags", nullable(array(string())))],
        vec![],
    )
    .validate()
    .unwrap();

    for (wire, fields, extra) in [
        (
            "matrix",
            vec![("matrix", array(array(string())))],
            Vec::new(),
        ),
        (
            "profiles",
            vec![("profiles", array(named("Profile")))],
            vec![profile()],
        ),
    ] {
        refuses_form_field(&form_document(fields, extra), wire);
    }
    refuses_form(
        &form_document(vec![("tags", nullable(array(string())))], vec![]),
        "form body field \"tags\" is required and nullable",
    );
}
