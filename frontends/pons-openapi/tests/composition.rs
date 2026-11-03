use pons_ir::{
    Document, EnumRepr, Field, FieldFormat, InlineType, NamedRef, ObjectOpenness, Presence,
    TypeKind, TypeRef, VariantPayload, operation_path,
};

const FIXTURE: &str = include_str!("../testdata/composition.json");

fn document() -> Document {
    pons_openapi::import(FIXTURE.as_bytes()).expect("the composition fixture imports")
}

fn operation<'a>(document: &'a Document, label: &str) -> &'a pons_ir::Operation {
    document
        .operations
        .iter()
        .find(|operation| operation_path(&operation.id) == label)
        .unwrap_or_else(|| panic!("no operation {label}"))
}

fn structure(definition: &TypeKind) -> (&[Field], &ObjectOpenness) {
    match definition {
        TypeKind::Struct {
            fields, openness, ..
        } => (fields, openness),
        other => panic!("expected a struct, found {other:?}"),
    }
}

fn field<'a>(fields: &'a [Field], wire: &str) -> &'a Field {
    fields
        .iter()
        .find(|field| field.wire == wire)
        .unwrap_or_else(|| panic!("no field {wire} in {fields:#?}"))
}

fn wires(fields: &[Field]) -> Vec<&str> {
    let mut wires = fields
        .iter()
        .map(|field| field.wire.as_str())
        .collect::<Vec<_>>();
    wires.sort_unstable();
    wires
}

#[test]
fn imports_merged_all_of_objects_and_discriminated_unions() {
    let document = document();
    document.validate().expect("the imported document is valid");

    let create = operation(&document, "users.create");

    let response = create
        .responses
        .first()
        .expect("the create operation responds");
    assert_eq!(
        response.body,
        Some(TypeRef::Named(NamedRef {
            id: "openapi::User#output".into(),
            variant: None,
        }))
    );

    let body = create
        .body
        .as_ref()
        .expect("the create operation takes a body");
    assert_eq!(
        body.ty,
        TypeRef::Named(NamedRef {
            id: "openapi::Payment#input".into(),
            variant: None,
        })
    );

    let user = &document.types["openapi::User#output"];
    let (fields, openness) = structure(&user.definition);
    assert_eq!(openness, &ObjectOpenness::Reject);
    assert_eq!(wires(fields), ["email", "id", "name"]);

    assert_eq!(field(fields, "id").presence, Presence::Required);
    assert_eq!(
        field(fields, "id").ty,
        TypeRef::Inline(InlineType::String {
            format: Some(FieldFormat::Uuid),
            min_length: None,
            max_length: None,
            pattern: None,
        })
    );
    assert_eq!(field(fields, "email").presence, Presence::Required);
    assert_eq!(
        field(fields, "email").ty,
        TypeRef::Inline(InlineType::String {
            format: Some(FieldFormat::Email),
            min_length: None,
            max_length: None,
            pattern: None,
        })
    );
    assert_eq!(field(fields, "name").presence, Presence::Optional);

    let payment = &document.types["openapi::Payment#input"];
    let TypeKind::Enum { repr, variants } = &payment.definition else {
        panic!(
            "expected an enum, found {:?}; diagnostics: {:#?}",
            payment.definition, payment.diagnostics
        );
    };
    assert_eq!(repr, &EnumRepr::Internal { tag: "kind".into() });

    let mut variant_wires = variants
        .iter()
        .map(|variant| variant.wire.as_str())
        .collect::<Vec<_>>();
    variant_wires.sort_unstable();
    assert_eq!(variant_wires, ["bank_transfer", "card"]);

    for variant in variants {
        let VariantPayload::Struct { fields } = &variant.payload else {
            panic!(
                "expected an object payload for {}, found {:?}",
                variant.wire, variant.payload
            );
        };
        assert!(
            !fields.iter().any(|field| field.wire == "kind"),
            "the discriminator is carried by the tag, not by {}'s payload",
            variant.wire
        );
    }

    let card = variants
        .iter()
        .find(|variant| variant.wire == "card")
        .expect("the card variant is present");
    let VariantPayload::Struct { fields } = &card.payload else {
        panic!("expected an object payload, found {:?}", card.payload);
    };
    assert_eq!(wires(fields), ["expiry", "number"]);
    assert_eq!(field(fields, "number").presence, Presence::Required);
    assert_eq!(field(fields, "expiry").presence, Presence::Optional);
}
