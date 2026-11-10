use pons_core::{Schema, TypeCx};
use pons_ir::{Direction, InlineType, IntegerWire, OpaqueWire, TypeRef};

pub trait HeaderSpec {
    type Value: Schema + std::str::FromStr;
    const NAME: &'static str;
}

pub(crate) fn header_schema<T: Schema>(
    cx: &mut TypeCx,
    direction: Direction,
    name: &str,
) -> TypeRef {
    let ty = T::schema(cx, direction);
    match ty {
        TypeRef::Inline(InlineType::Int {
            bits,
            signed,
            minimum,
            maximum,
            multiple_of,
            ..
        }) => TypeRef::Inline(InlineType::Int {
            bits,
            signed,
            wire: IntegerWire::String,
            minimum,
            maximum,
            multiple_of,
        }),
        TypeRef::Inline(
            InlineType::Array { .. } | InlineType::Map { .. } | InlineType::Tuple { .. },
        ) => {
            cx.push_error(
                "PONS-E102",
                format!(
                    "header {name} carries a JSON value; a header value is a text token, so it is described as opaque text"
                ),
            );
            TypeRef::Inline(InlineType::Opaque {
                codec: "axum.header".into(),
                wire: OpaqueWire::Text,
                ext: Default::default(),
            })
        }
        ty => ty,
    }
}
