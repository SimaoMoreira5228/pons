use std::collections::BTreeMap;

use pons_core::TypeCx;
use pons_ir::{
    Direction, Doc, EnumRepr, Field, FieldDefault, InlineType, IntegerWire, ObjectOpenness,
    OpaqueWire, Presence, TypeKind, TypeRef, Variant, VariantPayload,
};
use specta::Types;
use specta::datatype::{
    Attributes, DataType, Enum, Fields, NamedDataType, NamedReference, NamedReferenceType,
    Primitive, Reference, Struct, Variant as SpectaVariant,
};

use crate::attributes;
use crate::naming::{rename, words};

pub(crate) fn lower_root(
    cx: &mut TypeCx,
    types: &Types,
    root: &DataType,
    direction: Direction,
    root_type: &str,
) -> TypeRef {
    match root {
        DataType::Reference(reference) => lower_reference(cx, types, reference, direction),
        DataType::Struct(_) | DataType::Enum(_) => {
            let base = root_type.to_owned();
            let name = words(root_name(root_type));
            cx.named_as(
                direction_id(&base, direction),
                Some(base),
                name,
                direction,
                None,
                |cx| lower_type_kind(cx, types, root, direction),
            )
        }
        other => lower_data_type(cx, types, other, direction),
    }
}

fn root_name(root_type: &str) -> &str {
    root_type
        .rsplit("::")
        .next()
        .unwrap_or(root_type)
        .split('<')
        .next()
        .unwrap_or(root_type)
}

fn lower_reference(
    cx: &mut TypeCx,
    types: &Types,
    reference: &Reference,
    direction: Direction,
) -> TypeRef {
    match reference {
        Reference::Named(named) => match &named.inner {
            NamedReferenceType::Inline { dt, .. } => lower_data_type(cx, types, dt, direction),
            NamedReferenceType::Reference { .. } | NamedReferenceType::Recursive(_) => {
                lower_named(cx, types, named, direction)
            }
        },
        Reference::Opaque(reference) => opaque(
            cx,
            format!("specta:opaque:{}", reference.type_name()),
            format!(
                "specta opaque reference `{}` has no IR schema",
                reference.type_name()
            ),
        ),
    }
}

fn lower_named(
    cx: &mut TypeCx,
    types: &Types,
    reference: &NamedReference,
    direction: Direction,
) -> TypeRef {
    let Some(named) = types.get(reference) else {
        return opaque(
            cx,
            "specta:unresolved",
            "specta reference does not resolve to a named definition",
        );
    };
    let base = named_base(named);
    let id = direction_id(&base, direction);
    let name = words(&named.name);
    let doc = doc(&named.docs, named.deprecated.is_some());
    cx.named_as(id, Some(base), name, direction, doc, |cx| {
        match named.ty.as_ref() {
            Some(data) => lower_type_kind(cx, types, data, direction),
            None => {
                cx.push_type_diagnostic(
                    "PONS-W202",
                    format!("specta type `{}` has no exported definition", named.name),
                );
                TypeKind::Newtype {
                    inner: TypeRef::Inline(InlineType::Opaque {
                        codec: "specta:inline".to_owned(),
                        wire: OpaqueWire::Unknown,
                        ext: BTreeMap::new(),
                    }),
                }
            }
        }
    })
}

fn lower_type_kind(
    cx: &mut TypeCx,
    types: &Types,
    data: &DataType,
    direction: Direction,
) -> TypeKind {
    match data {
        DataType::Struct(data) => lower_struct_kind(cx, types, data, direction),
        DataType::Enum(data) => lower_enum_kind(cx, types, data, direction),
        other => TypeKind::Newtype {
            inner: lower_data_type(cx, types, other, direction),
        },
    }
}

fn lower_struct_kind(
    cx: &mut TypeCx,
    types: &Types,
    data: &Struct,
    direction: Direction,
) -> TypeKind {
    match &data.fields {
        Fields::Named(named) => TypeKind::Struct {
            fields: lower_named_fields(
                cx,
                types,
                named,
                direction,
                attributes::container_rename_all(&data.attributes, direction),
            ),
            flattened: Vec::new(),
            openness: openness(&data.attributes),
        },
        Fields::Unnamed(unnamed) => {
            let items = unnamed
                .fields
                .iter()
                .filter(|field| !attributes::field_skipped(field, direction))
                .filter_map(|field| field.ty.as_ref())
                .map(|ty| lower_data_type(cx, types, ty, direction))
                .collect::<Vec<_>>();
            TypeKind::Newtype {
                inner: match items.len() {
                    0 => TypeRef::Inline(InlineType::Null),
                    1 => items.into_iter().next().expect("one item"),
                    _ => TypeRef::Inline(InlineType::Tuple { items }),
                },
            }
        }
        Fields::Unit => TypeKind::Newtype {
            inner: TypeRef::Inline(InlineType::Null),
        },
    }
}

fn lower_enum_kind(cx: &mut TypeCx, types: &Types, data: &Enum, direction: Direction) -> TypeKind {
    let repr = enum_repr(data);
    let fields_rule = attributes::fields_rename_all(&data.attributes, direction);
    let variants = data
        .variants
        .iter()
        .filter(|(_, variant)| !attributes::variant_skipped(variant, direction))
        .map(|(name, variant)| {
            lower_variant(
                cx,
                types,
                name,
                variant,
                &data.attributes,
                fields_rule,
                direction,
            )
        })
        .collect();
    TypeKind::Enum { repr, variants }
}

fn lower_variant(
    cx: &mut TypeCx,
    types: &Types,
    name: &str,
    variant: &SpectaVariant,
    container: &Attributes,
    fields_rule: Option<&str>,
    direction: Direction,
) -> Variant {
    let wire = attributes::variant_rename(&variant.attributes, direction)
        .map(str::to_owned)
        .or_else(|| {
            attributes::container_rename_all(container, direction).map(|rule| rename(name, rule))
        })
        .unwrap_or_else(|| name.to_owned());
    let rule = attributes::variant_rename_all(&variant.attributes, direction).or(fields_rule);
    let payload = match &variant.fields {
        Fields::Unit => VariantPayload::Unit,
        Fields::Unnamed(unnamed) => {
            let items = unnamed
                .fields
                .iter()
                .filter(|field| !attributes::field_skipped(field, direction))
                .filter_map(|field| field.ty.as_ref())
                .map(|ty| lower_data_type(cx, types, ty, direction))
                .collect::<Vec<_>>();
            match items.len() {
                0 => VariantPayload::Unit,
                1 => VariantPayload::Newtype {
                    ty: items.into_iter().next().expect("one item"),
                },
                _ => VariantPayload::Tuple { items },
            }
        }
        Fields::Named(named) => VariantPayload::Struct {
            fields: lower_named_fields(cx, types, named, direction, rule),
        },
    };
    Variant {
        ident: words(&wire),
        wire,
        payload,
        doc: doc(&variant.docs, variant.deprecated.is_some()),
        diagnostics: Vec::new(),
        ext: BTreeMap::new(),
    }
}

fn lower_named_fields(
    cx: &mut TypeCx,
    types: &Types,
    named: &specta::datatype::NamedFields,
    direction: Direction,
    rule: Option<&str>,
) -> Vec<Field> {
    named
        .fields
        .iter()
        .filter(|(_, field)| !attributes::field_skipped(field, direction))
        .map(|(name, field)| lower_field(cx, types, name, field, direction, rule))
        .collect()
}

fn lower_field(
    cx: &mut TypeCx,
    types: &Types,
    name: &str,
    field: &specta::datatype::Field,
    direction: Direction,
    rule: Option<&str>,
) -> Field {
    let wire = attributes::field_rename(&field.attributes, direction)
        .map(str::to_owned)
        .or_else(|| rule.map(|rule| rename(name, rule)))
        .unwrap_or_else(|| name.to_owned());
    let ty = field
        .ty
        .as_ref()
        .map(|ty| lower_data_type(cx, types, ty, direction))
        .unwrap_or_else(|| {
            opaque(
                cx,
                "specta:skipped-field",
                format!("specta field `{name}` has no type"),
            )
        });
    Field {
        ident: words(&wire),
        wire,
        ty,
        presence: if attributes::field_optional(field, direction) {
            Presence::Optional
        } else {
            Presence::Required
        },
        default: FieldDefault::Unspecified,
        access: None,
        doc: doc(&field.docs, field.deprecated.is_some()),
        diagnostics: Vec::new(),
        ext: BTreeMap::new(),
    }
}

fn lower_data_type(
    cx: &mut TypeCx,
    types: &Types,
    data: &DataType,
    direction: Direction,
) -> TypeRef {
    match data {
        DataType::Primitive(primitive) => lower_primitive(cx, primitive),
        DataType::List(list) => TypeRef::Inline(InlineType::Array {
            item: Box::new(lower_data_type(cx, types, &list.ty, direction)),
            unique: list.unique.then_some(true),
        }),
        DataType::Map(map) => TypeRef::Inline(InlineType::Map {
            key: Box::new(lower_data_type(cx, types, map.key_ty(), direction)),
            value: Box::new(lower_data_type(cx, types, map.value_ty(), direction)),
        }),
        DataType::Tuple(tuple) => TypeRef::Inline(InlineType::Tuple {
            items: tuple
                .elements
                .iter()
                .map(|element| lower_data_type(cx, types, element, direction))
                .collect(),
        }),
        DataType::Nullable(inner) => TypeRef::Inline(InlineType::Nullable {
            inner: Box::new(lower_data_type(cx, types, inner, direction)),
        }),
        DataType::Reference(reference) => lower_reference(cx, types, reference, direction),
        DataType::Struct(_) => opaque(
            cx,
            "specta:inline-struct",
            "specta inline struct has no named definition to reference",
        ),
        DataType::Enum(_) => opaque(
            cx,
            "specta:inline-enum",
            "specta inline enum has no named definition to reference",
        ),
        DataType::Intersection(_) => opaque(
            cx,
            "specta:intersection",
            "specta intersection has no IR equivalent",
        ),
        DataType::Generic(generic) => opaque(
            cx,
            format!("specta:generic:{}", generic.name()),
            format!("specta generic `{}` was not instantiated", generic.name()),
        ),
    }
}

fn lower_primitive(cx: &mut TypeCx, primitive: &Primitive) -> TypeRef {
    let inline = match primitive {
        Primitive::i8 => integer(8, true),
        Primitive::i16 => integer(16, true),
        Primitive::i32 => integer(32, true),
        Primitive::i64 => integer(64, true),
        Primitive::i128 => integer(128, true),
        Primitive::isize => integer(64, true),
        Primitive::u8 => integer(8, false),
        Primitive::u16 => integer(16, false),
        Primitive::u32 => integer(32, false),
        Primitive::u64 => integer(64, false),
        Primitive::u128 => integer(128, false),
        Primitive::usize => integer(64, false),
        Primitive::f32 => InlineType::Float {
            bits: 32,
            minimum: None,
            maximum: None,
            exclusive_minimum: None,
            exclusive_maximum: None,
            multiple_of: None,
        },
        Primitive::f64 => InlineType::Float {
            bits: 64,
            minimum: None,
            maximum: None,
            exclusive_minimum: None,
            exclusive_maximum: None,
            multiple_of: None,
        },
        Primitive::bool => InlineType::Bool,
        Primitive::char => InlineType::String {
            format: None,
            min_length: Some(1),
            max_length: Some(1),
            pattern: None,
        },
        Primitive::str => string(),
        Primitive::f16 => {
            return opaque(
                cx,
                "specta:f16",
                "specta f16 has no IR float width of 16 bits",
            );
        }
        Primitive::f128 => {
            return opaque(
                cx,
                "specta:f128",
                "specta f128 has no IR float width of 128 bits",
            );
        }
    };
    TypeRef::Inline(inline)
}

fn enum_repr(data: &Enum) -> EnumRepr {
    if attributes::untagged(&data.attributes) {
        return EnumRepr::Untagged;
    }
    if let Some(tag) = attributes::tag(&data.attributes) {
        return match attributes::content(&data.attributes) {
            Some(content) => EnumRepr::Adjacent {
                tag: tag.to_owned(),
                content: content.to_owned(),
            },
            None => EnumRepr::Internal {
                tag: tag.to_owned(),
            },
        };
    }
    if data
        .variants
        .iter()
        .all(|(_, variant)| matches!(variant.fields, Fields::Unit))
    {
        EnumRepr::String
    } else {
        EnumRepr::External
    }
}

fn openness(attributes: &Attributes) -> ObjectOpenness {
    if attributes::deny_unknown_fields(attributes) {
        ObjectOpenness::Reject
    } else {
        ObjectOpenness::PreserveOpaque
    }
}

fn integer(bits: u16, signed: bool) -> InlineType {
    InlineType::Int {
        bits,
        signed,
        wire: IntegerWire::Number,
        minimum: None,
        maximum: None,
        multiple_of: None,
    }
}

fn string() -> InlineType {
    InlineType::String {
        format: None,
        min_length: None,
        max_length: None,
        pattern: None,
    }
}

fn doc(text: &str, deprecated: bool) -> Option<Doc> {
    let text = text.trim();
    if text.is_empty() && !deprecated {
        return None;
    }
    Some(Doc {
        summary: (!text.is_empty()).then(|| text.to_owned()),
        description: None,
        deprecated: deprecated.then_some(true),
        tags: Vec::new(),
    })
}

fn named_base(named: &NamedDataType) -> String {
    if named.module_path.is_empty() {
        named.name.to_string()
    } else {
        format!("{}::{}", named.module_path, named.name)
    }
}

fn direction_id(base: &str, direction: Direction) -> String {
    match direction {
        Direction::Both => base.to_owned(),
        Direction::Input => format!("{base}#input"),
        Direction::Output => format!("{base}#output"),
    }
}

fn opaque(cx: &mut TypeCx, codec: impl Into<String>, message: impl Into<String>) -> TypeRef {
    cx.push_type_diagnostic("PONS-W202", message);
    TypeRef::Inline(InlineType::Opaque {
        codec: codec.into(),
        wire: OpaqueWire::Unknown,
        ext: BTreeMap::new(),
    })
}
