use pons_core::TypeCx;
use pons_ir::{Direction, Field, InlineType, TypeDef, TypeKind, TypeRef};
use pons_json_schema::{RootType, lower};

mod all_of;
mod definitions;
mod discriminator;
mod inline_all_of;
mod opaque_composition;
mod scalars;

fn definition<'cx>(cx: &'cx TypeCx, reference: &TypeRef) -> &'cx TypeDef {
    let TypeRef::Named(named) = reference else {
        panic!()
    };
    &cx.definitions()[&named.id]
}

fn lower_root(
    schema: serde_json::Value,
    id: &str,
    name: &str,
    direction: Direction,
) -> (TypeCx, TypeRef) {
    let mut cx = TypeCx::new();
    let reference = lower(
        &mut cx,
        schema,
        RootType {
            id,
            name,
            origin: Some(id),
            direction,
        },
    );
    (cx, reference)
}

fn opaque_codec<'cx>(cx: &'cx TypeCx, reference: &TypeRef) -> &'cx str {
    let TypeKind::Newtype { inner } = &definition(cx, reference).definition else {
        panic!()
    };
    let TypeRef::Inline(InlineType::Opaque { codec, .. }) = inner else {
        panic!()
    };
    codec
}

fn has_code(cx: &TypeCx, reference: &TypeRef, code: &str) -> bool {
    definition(cx, reference)
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == code)
}

fn field<'a>(fields: &'a [Field], wire: &str) -> &'a Field {
    fields
        .iter()
        .find(|field| field.wire == wire)
        .unwrap_or_else(|| panic!("no field named {wire}"))
}
