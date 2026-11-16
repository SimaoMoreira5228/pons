use pons_ir::{InlineType, NamedRef, TypeRef};

use super::{document_30, structure};

#[test]
fn translates_a_30_document() {
    let document = document_30();
    document
        .validate()
        .expect("the imported 3.0 document is valid");

    let order = &document.types["openapi::Order#output"];
    let (fields, _) = structure(&order.definition);

    let note = fields.iter().find(|field| field.wire == "note").unwrap();
    let TypeRef::Inline(InlineType::Nullable { inner }) = &note.ty else {
        panic!("expected a nullable note, found {:?}", note.ty);
    };
    assert!(matches!(
        inner.as_ref(),
        TypeRef::Inline(InlineType::String { .. })
    ));

    let customer = fields
        .iter()
        .find(|field| field.wire == "customer")
        .unwrap();
    let TypeRef::Inline(InlineType::Nullable { inner }) = &customer.ty else {
        panic!("expected a nullable customer, found {:?}", customer.ty);
    };
    assert_eq!(
        inner.as_ref(),
        &TypeRef::Named(NamedRef {
            id: "openapi::Customer#output".into(),
            variant: None,
        })
    );

    let quantity = fields
        .iter()
        .find(|field| field.wire == "quantity")
        .unwrap();
    let TypeRef::Inline(InlineType::Int {
        bits,
        signed,
        minimum,
        ..
    }) = &quantity.ty
    else {
        panic!("expected an integer quantity, found {:?}", quantity.ty);
    };
    assert_eq!(*bits, 32);
    assert!(signed);
    assert_eq!(minimum.as_ref(), Some(&serde_json::json!(2)));

    let total = fields.iter().find(|field| field.wire == "total").unwrap();
    let TypeRef::Inline(InlineType::Float {
        bits,
        minimum,
        exclusive_minimum,
        ..
    }) = &total.ty
    else {
        panic!("expected a float total, found {:?}", total.ty);
    };
    assert_eq!(*bits, 64);
    assert_eq!(minimum.as_ref(), None);
    assert_eq!(exclusive_minimum.as_ref(), Some(&serde_json::json!(0)));

    let attachment = fields
        .iter()
        .find(|field| field.wire == "attachment")
        .unwrap();
    let TypeRef::Inline(InlineType::Opaque { codec, .. }) = &attachment.ty else {
        panic!("expected an opaque attachment, found {:?}", attachment.ty);
    };
    assert_eq!(codec, "json-schema:type:file");

    let diagnostics = &document.diagnostics;
    let warned = |needle: &str| {
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "PONS-W205" && diagnostic.message.contains(needle))
    };
    assert!(warned("properties.attachment"), "{diagnostics:#?}");
}
