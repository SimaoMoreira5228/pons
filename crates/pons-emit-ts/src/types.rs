use std::collections::BTreeMap;

use pons_ir::{
    Document, EnumRepr, Field, InlineType, IntegerWire, TypeDef, TypeKind, TypeRef, Variant,
    VariantPayload,
};

use crate::{EmitError, EmitMode, string};

pub(super) fn emit(
    document: &Document,
    names: &BTreeMap<String, String>,
    mode: EmitMode,
) -> Result<String, EmitError> {
    let mut output = String::new();
    for (id, definition) in &document.types {
        let name = &names[id];
        output.push_str(&format!(
            "export type {name} = {};\n\n",
            definition_type(definition, document, names, mode)?
        ));
    }
    Ok(output.replace("Types.", ""))
}

fn definition_type(
    definition: &TypeDef,
    document: &Document,
    names: &BTreeMap<String, String>,
    mode: EmitMode,
) -> Result<String, EmitError> {
    match &definition.definition {
        TypeKind::Struct {
            fields,
            flattened,
            additional,
        } => {
            let mut members = fields
                .iter()
                .map(|field| field_type(field, document, names, mode))
                .collect::<Result<Vec<_>, _>>()?;
            if let Some(additional) = additional {
                members.push(format!(
                    "[key: string]: {}",
                    type_ref(additional, document, names, mode)?
                ));
            }
            let mut ty = format!("{{ {} }}", members.join("; "));
            for flat in flattened {
                ty = format!("{ty} & {}", type_ref(flat, document, names, mode)?);
            }
            Ok(ty)
        }
        TypeKind::Enum { repr, variants } => enum_type(repr, variants, document, names, mode),
        TypeKind::Newtype { inner } => type_ref(inner, document, names, mode),
    }
}

fn field_type(
    field: &Field,
    document: &Document,
    names: &BTreeMap<String, String>,
    mode: EmitMode,
) -> Result<String, EmitError> {
    let key = string(&field.wire);
    let presence = if field.presence == pons_ir::Presence::Optional {
        "?"
    } else {
        ""
    };
    Ok(format!(
        "{key}{presence}: {}",
        type_ref(&field.ty, document, names, mode)?
    ))
}

fn enum_type(
    repr: &EnumRepr,
    variants: &[Variant],
    document: &Document,
    names: &BTreeMap<String, String>,
    mode: EmitMode,
) -> Result<String, EmitError> {
    let mut types = Vec::with_capacity(variants.len());
    for variant in variants {
        let payload = match &variant.payload {
            VariantPayload::Unit => "null".into(),
            VariantPayload::Newtype { ty } => type_ref(ty, document, names, mode)?,
            VariantPayload::Tuple { items } => format!(
                "[{}]",
                items
                    .iter()
                    .map(|item| type_ref(item, document, names, mode))
                    .collect::<Result<Vec<_>, _>>()?
                    .join(", ")
            ),
            VariantPayload::Struct { fields } => format!(
                "{{ {} }}",
                fields
                    .iter()
                    .map(|field| field_type(field, document, names, mode))
                    .collect::<Result<Vec<_>, _>>()?
                    .join("; ")
            ),
        };
        let wire = string(&variant.wire);
        let ty = match repr {
            EnumRepr::String => wire,
            EnumRepr::External => match &variant.payload {
                VariantPayload::Unit => wire,
                _ => format!("{{ {wire}: {payload} }}"),
            },
            EnumRepr::Internal { tag } => {
                let tag = string(tag);
                match &variant.payload {
                    VariantPayload::Unit => format!("{{ {tag}: {wire} }}"),
                    _ => format!("{{ {tag}: {wire} }} & {payload}"),
                }
            }
            EnumRepr::Adjacent { tag, content } => {
                let tag_key = string(tag);
                let tag_field = format!("{tag_key}: {wire}");
                if matches!(&variant.payload, VariantPayload::Unit) {
                    format!("{{ {tag_field} }}")
                } else {
                    format!("{{ {tag_field}; {}: {payload} }}", string(content))
                }
            }
            EnumRepr::Untagged => payload,
        };
        types.push(ty);
    }
    if types.is_empty() {
        return Err(EmitError("an enum must have at least one variant".into()));
    }
    Ok(types.join(" | "))
}

pub(super) fn type_ref(
    reference: &TypeRef,
    document: &Document,
    names: &BTreeMap<String, String>,
    mode: EmitMode,
) -> Result<String, EmitError> {
    match reference {
        TypeRef::Named(reference) => {
            let name = names
                .get(&reference.id)
                .ok_or_else(|| EmitError(format!("unknown type {}", reference.id)))?;
            let Some(variant) = &reference.variant else {
                return Ok(format!("Types.{name}"));
            };
            let filter = match document
                .types
                .get(&reference.id)
                .map(|definition| &definition.definition)
            {
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
                _ => return Ok(format!("Types.{name}")),
            };
            Ok(format!("Extract<Types.{name}, {filter}>"))
        }
        TypeRef::Inline(inline) => match inline {
            InlineType::Bool => Ok("boolean".into()),
            InlineType::Int { bits, wire, .. } => match wire {
                IntegerWire::String => Ok("string".into()),
                IntegerWire::Number if *bits > 53 && mode == EmitMode::Strict => {
                    Ok("bigint".into())
                }
                IntegerWire::Number => Ok("number".into()),
            },
            InlineType::Float { .. } => Ok("number".into()),
            InlineType::Decimal { wire } => Ok(match wire {
                pons_ir::DecimalWire::Number => "number",
                pons_ir::DecimalWire::String => "string",
            }
            .into()),
            InlineType::String { .. } => Ok("string".into()),
            InlineType::Bytes { .. } => Ok("Uint8Array".into()),
            InlineType::Null => Ok("null".into()),
            InlineType::Any => Ok("unknown".into()),
            InlineType::Nullable { inner } => Ok(format!(
                "{} | null",
                type_ref(inner, document, names, mode)?
            )),
            InlineType::Array { item, .. } => {
                Ok(format!("Array<{}>", type_ref(item, document, names, mode)?))
            }
            InlineType::Map { value, .. } => Ok(format!(
                "Record<string, {}>",
                type_ref(value, document, names, mode)?
            )),
            InlineType::Tuple { items } => Ok(format!(
                "[{}]",
                items
                    .iter()
                    .map(|item| type_ref(item, document, names, mode))
                    .collect::<Result<Vec<_>, _>>()?
                    .join(", ")
            )),
        },
    }
}
