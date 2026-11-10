use std::collections::{BTreeMap, BTreeSet};

use pons_ir::{
    Access, Direction, Document, EnumRepr, InlineType, IntegerWire, ObjectOpenness, TypeKind,
    TypeRef, VariantPayload,
};

use crate::render::{object_type, string};
use crate::{DateMode, EmitError, EmitMode, Int64, TsOptions};

use super::definition::payload_type;
use super::field::{field_is_visible, field_type};
use super::flatten::{flattened_field_types, openness_value};

pub(super) fn local_type_ref(
    reference: &TypeRef,
    document: &Document,
    names: &BTreeMap<String, String>,
    options: &TsOptions,
) -> Result<String, EmitError> {
    type_ref_with_namespace(
        reference,
        document,
        names,
        options,
        false,
        &mut BTreeSet::new(),
    )
}

pub(super) fn type_ref_with_namespace(
    reference: &TypeRef,
    document: &Document,
    names: &BTreeMap<String, String>,
    options: &TsOptions,
    qualify: bool,
    narrowed: &mut BTreeSet<String>,
) -> Result<String, EmitError> {
    match reference {
        TypeRef::Named(reference) => {
            let name = names
                .get(&reference.id)
                .ok_or_else(|| EmitError(format!("unknown type {}", reference.id)))?;
            let name = if qualify {
                format!("Types.{name}")
            } else {
                name.clone()
            };
            let Some(variant) = &reference.variant else {
                return Ok(name);
            };
            let definition = document.types.get(&reference.id);
            if let Some(TypeKind::Enum {
                repr: EnumRepr::Untagged,
                variants,
            }) = definition.map(|definition| &definition.definition)
            {
                if !narrowed.insert(format!("{}\u{1}{variant}", reference.id)) {
                    return Ok(name);
                }
                let Some(entry) = variants.iter().find(|entry| entry.wire == *variant) else {
                    return Ok(name);
                };
                let direction =
                    definition.map_or(Direction::Both, |definition| definition.direction);
                return payload_type(
                    &entry.payload,
                    direction,
                    document,
                    names,
                    options,
                    qualify,
                    narrowed,
                );
            }
            let filter = match definition.map(|definition| &definition.definition) {
                Some(TypeKind::Enum {
                    repr: EnumRepr::String,
                    ..
                }) => string(variant),
                Some(TypeKind::Enum {
                    repr: EnumRepr::External,
                    variants,
                }) => {
                    if variants.iter().any(|entry| {
                        entry.wire == *variant && matches!(&entry.payload, VariantPayload::Unit)
                    }) {
                        string(variant)
                    } else {
                        format!("{{ {}: unknown }}", string(variant))
                    }
                }
                Some(TypeKind::Enum {
                    repr: EnumRepr::Internal { tag },
                    ..
                })
                | Some(TypeKind::Enum {
                    repr: EnumRepr::Adjacent { tag, .. },
                    ..
                }) => format!("{{ {}: {} }}", string(tag), string(variant)),
                _ => return Ok(name),
            };
            Ok(format!("Extract<{name}, {filter}>"))
        }
        TypeRef::Inline(inline) => match inline {
            InlineType::Bool => Ok("boolean".into()),
            InlineType::Int { bits, wire, .. } => match wire {
                IntegerWire::String => Ok("string".into()),
                IntegerWire::Number if *bits > 53 => Ok(match options.int64 {
                    Int64::String => "string",
                    Int64::Bigint if options.mode == EmitMode::Strict => "bigint",
                    _ => "number",
                }
                .into()),
                IntegerWire::Number => Ok("number".into()),
            },
            InlineType::Float { .. } => Ok("number".into()),
            InlineType::Decimal { wire } => Ok(match wire {
                pons_ir::DecimalWire::Number => "number",
                pons_ir::DecimalWire::String => "string",
            }
            .into()),
            InlineType::String { format, .. } => Ok(match format {
                Some(pons_ir::FieldFormat::DateTime) if options.date_mode == DateMode::Date => {
                    "Date"
                }
                Some(pons_ir::FieldFormat::Date) if options.date_mode == DateMode::Date => {
                    "DateOnly"
                }
                _ => "string",
            }
            .into()),
            InlineType::Bytes { .. } => Ok("Uint8Array".into()),
            InlineType::Null => Ok("null".into()),
            InlineType::Any => Ok("unknown".into()),
            InlineType::Opaque { .. } => Ok("unknown".into()),
            InlineType::Nullable { inner } => Ok(format!(
                "{} | null",
                type_ref_with_namespace(inner, document, names, options, qualify, narrowed)?
            )),
            InlineType::Array { item, .. } => Ok(format!(
                "Array<{}>",
                type_ref_with_namespace(item, document, names, options, qualify, narrowed)?
            )),
            InlineType::Map { value, .. } => Ok(format!(
                "Record<string, {}>",
                type_ref_with_namespace(value, document, names, options, qualify, narrowed)?
            )),
            InlineType::Tuple { items } => Ok(format!(
                "[{}]",
                items
                    .iter()
                    .map(|item| {
                        type_ref_with_namespace(item, document, names, options, qualify, narrowed)
                    })
                    .collect::<Result<Vec<_>, _>>()?
                    .join(", ")
            )),
            InlineType::Struct {
                fields,
                flattened,
                openness,
            } => {
                let flattened_types = flattened_field_types(flattened, document, names, options)?;
                let visible = fields
                    .iter()
                    .filter(|field| field_is_visible(field, Direction::Both))
                    .collect::<Vec<_>>();
                let mut known_types = visible
                    .iter()
                    .map(|field| local_type_ref(&field.ty, document, names, options))
                    .collect::<Result<Vec<_>, _>>()?;
                known_types.extend(flattened_types);
                let mut members = visible
                    .iter()
                    .map(|field| {
                        field_type(
                            field,
                            Direction::Both,
                            document,
                            names,
                            options,
                            qualify,
                            narrowed,
                        )
                        .map(|member| (field.doc.as_ref(), member))
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                match openness.as_ref() {
                    ObjectOpenness::Reject => {}
                    ObjectOpenness::PreserveTyped { value } => {
                        let extra = local_type_ref(
                            openness_value(value, document),
                            document,
                            names,
                            options,
                        )?;
                        if known_types.iter().any(|ty| *ty != extra) {
                            return Err(EmitError(
                                "TypeScript cannot represent typed object openness when direct or flattened fields have different types"
                                    .into(),
                            ));
                        }
                        let index_type = if visible.iter().any(|field| {
                            field.presence == pons_ir::Presence::Optional
                                || matches!(
                                    field.access,
                                    Some(Access::ReadOnly | Access::WriteOnly)
                                )
                        }) {
                            format!("{extra} | undefined")
                        } else {
                            extra
                        };
                        members.push((None, format!("[key: string]: {index_type}")));
                    }
                    ObjectOpenness::PreserveOpaque => {
                        members.push((None, String::from("[key: string]: unknown")));
                    }
                }
                let mut ty = object_type(&members);
                for flat in flattened {
                    ty = format!("{ty} & {}", local_type_ref(flat, document, names, options)?);
                }
                Ok(ty)
            }
        },
    }
}
