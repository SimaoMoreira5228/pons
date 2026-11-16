use pons_ir::{
    Direction, InlineType, NamedRef, ObjectOpenness, Presence, TypeKind, TypeRef, VariantPayload,
};

use super::{document, structure};

#[test]
fn lowers_components_per_direction() {
    let document = document();
    let order = &document.types["openapi::Order#output"];
    assert_eq!(order.name, ["order"]);
    assert_eq!(order.direction, Direction::Output);
    let (fields, openness) = structure(&order.definition);
    assert_eq!(openness, &ObjectOpenness::Reject);

    let customer = fields
        .iter()
        .find(|field| field.wire == "customer")
        .unwrap();
    assert_eq!(
        customer.ty,
        TypeRef::Named(NamedRef {
            id: "openapi::Customer#output".into(),
            variant: None,
        })
    );

    let note = fields.iter().find(|field| field.wire == "note").unwrap();
    assert_eq!(note.presence, Presence::Optional);
    let TypeRef::Inline(InlineType::Nullable { inner }) = &note.ty else {
        panic!("expected a nullable type, found {:?}", note.ty);
    };
    assert!(matches!(
        inner.as_ref(),
        TypeRef::Inline(InlineType::String { .. })
    ));

    let reference = fields
        .iter()
        .find(|field| field.wire == "reference")
        .unwrap();
    assert_eq!(reference.presence, Presence::Optional);
    assert!(matches!(
        reference.ty,
        TypeRef::Inline(InlineType::String { .. })
    ));

    let payment = fields.iter().find(|field| field.wire == "payment").unwrap();
    let TypeRef::Inline(InlineType::Opaque { codec, .. }) = &payment.ty else {
        panic!("expected an opaque type, found {:?}", payment.ty);
    };
    assert_eq!(codec, "json-schema:oneOf");

    let status = &document.types["openapi::Status#output"];
    let TypeKind::Enum { repr, variants } = &status.definition else {
        panic!("expected an enum, found {:?}", status.definition);
    };
    assert_eq!(repr, &pons_ir::EnumRepr::String);
    assert!(
        variants
            .iter()
            .all(|variant| matches!(variant.payload, VariantPayload::Unit))
    );
    let wires = variants
        .iter()
        .map(|variant| variant.wire.as_str())
        .collect::<Vec<_>>();
    assert_eq!(wires, ["cancelled", "pending", "shipped"]);
}
