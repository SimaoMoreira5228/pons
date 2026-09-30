use std::collections::{BTreeMap, BTreeSet};

use pons_ir::{
    Access, Direction, Document, EnumRepr, Field, InlineType, IntegerWire, ObjectOpenness, TypeDef,
    TypeKind, TypeRef, Variant, VariantPayload,
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
    Ok(output)
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
            openness,
        } => {
            let flattened_types = flattened_field_types(flattened, document, names, mode)?;
            let fields = fields
                .iter()
                .filter(|field| field_is_visible(field, definition.direction))
                .collect::<Vec<_>>();
            let mut known_types = fields
                .iter()
                .map(|field| local_type_ref(&field.ty, document, names, mode))
                .collect::<Result<Vec<_>, _>>()?;
            known_types.extend(flattened_types);
            let mut members = fields
                .iter()
                .map(|field| field_type(field, definition.direction, document, names, mode))
                .collect::<Result<Vec<_>, _>>()?;
            match openness {
                ObjectOpenness::Reject => {}
                ObjectOpenness::PreserveTyped { value } => {
                    let extra =
                        local_type_ref(openness_value(value, document), document, names, mode)?;
                    if known_types.iter().any(|ty| *ty != extra) {
                        return Err(EmitError(
                            "TypeScript cannot represent typed object openness when direct or flattened fields have different types"
                                .into(),
                        ));
                    }
                    let index_type = if fields.iter().any(|field| {
                        field.presence == pons_ir::Presence::Optional
                            || matches!(
                                (definition.direction, field.access),
                                (Direction::Both, Some(Access::ReadOnly | Access::WriteOnly))
                            )
                    }) {
                        format!("{extra} | undefined")
                    } else {
                        extra
                    };
                    members.push(format!("[key: string]: {index_type}"));
                }
                ObjectOpenness::PreserveOpaque => {
                    members.push("[key: string]: unknown".into());
                }
            }
            let mut ty = format!("{{ {} }}", members.join("; "));
            for flat in flattened {
                ty = format!("{ty} & {}", local_type_ref(flat, document, names, mode)?);
            }
            Ok(ty)
        }
        TypeKind::Enum { repr, variants } => {
            enum_type(definition.direction, repr, variants, document, names, mode)
        }
        TypeKind::Newtype { inner } => local_type_ref(inner, document, names, mode),
    }
}

fn openness_value<'a>(ty: &'a TypeRef, document: &'a Document) -> &'a TypeRef {
    let mut current = ty;
    let mut visited = BTreeSet::new();
    loop {
        match current {
            TypeRef::Inline(InlineType::Map { value, .. }) => return value,
            TypeRef::Named(named) if visited.insert(named.id.clone()) => {
                let Some(TypeKind::Newtype { inner }) = document
                    .types
                    .get(&named.id)
                    .map(|definition| &definition.definition)
                else {
                    return current;
                };
                current = inner;
            }
            _ => return current,
        }
    }
}

fn field_is_visible(field: &Field, direction: Direction) -> bool {
    !matches!(
        (direction, field.access),
        (Direction::Input, Some(Access::ReadOnly)) | (Direction::Output, Some(Access::WriteOnly))
    )
}

fn flattened_field_types(
    references: &[TypeRef],
    document: &Document,
    names: &BTreeMap<String, String>,
    mode: EmitMode,
) -> Result<Vec<String>, EmitError> {
    let mut result = Vec::new();
    let mut visited = BTreeSet::new();
    let mut visiting = Vec::new();
    for reference in references {
        collect_flattened_field_types(
            reference,
            document,
            names,
            mode,
            &mut visited,
            &mut visiting,
            &mut result,
        )?;
    }
    Ok(result)
}

fn collect_flattened_field_types(
    reference: &TypeRef,
    document: &Document,
    names: &BTreeMap<String, String>,
    mode: EmitMode,
    visited: &mut BTreeSet<String>,
    visiting: &mut Vec<String>,
    result: &mut Vec<String>,
) -> Result<(), EmitError> {
    let TypeRef::Named(named) = reference else {
        if matches!(reference, TypeRef::Inline(InlineType::Opaque { .. })) {
            return Ok(());
        }
        return Err(not_a_struct_member(reference));
    };
    if named.variant.is_some() {
        return Err(not_a_struct_member(reference));
    }
    if visiting.contains(&named.id) {
        return Err(EmitError(format!(
            "the type {} cannot be inlined because flattened members form a cycle",
            named.id
        )));
    }
    if !visited.insert(named.id.clone()) {
        return Ok(());
    }
    let definition = document
        .types
        .get(&named.id)
        .ok_or_else(|| EmitError(format!("unknown type {}", named.id)))?;
    match &definition.definition {
        TypeKind::Struct {
            fields,
            flattened,
            openness,
        } => {
            visiting.push(named.id.clone());
            for field in fields
                .iter()
                .filter(|field| field_is_visible(field, definition.direction))
            {
                result.push(local_type_ref(&field.ty, document, names, mode)?);
            }
            if let ObjectOpenness::PreserveTyped { value } = openness {
                result.push(local_type_ref(
                    openness_value(value, document),
                    document,
                    names,
                    mode,
                )?);
            }
            for flattened in flattened {
                collect_flattened_field_types(
                    flattened, document, names, mode, visited, visiting, result,
                )?;
            }
            visiting.pop();
        }
        TypeKind::Newtype { inner } => {
            collect_flattened_field_types(inner, document, names, mode, visited, visiting, result)?
        }
        TypeKind::Enum { .. } => return Err(not_a_struct_member(reference)),
    }
    Ok(())
}

fn not_a_struct_member(ty: &TypeRef) -> EmitError {
    let name = match ty {
        TypeRef::Named(named) => match named.variant.as_deref() {
            Some(variant) => format!("the {variant} variant of the type {}", named.id),
            None => format!("the type {}", named.id),
        },
        TypeRef::Inline(InlineType::Map { .. }) => "an inline map".into(),
        TypeRef::Inline(_) => "an inline type".into(),
    };
    EmitError(format!(
        "{name} cannot be inlined into a struct because only struct members are representable"
    ))
}

fn field_type(
    field: &Field,
    direction: Direction,
    document: &Document,
    names: &BTreeMap<String, String>,
    mode: EmitMode,
) -> Result<String, EmitError> {
    let key = string(&field.wire);
    let access_specific = matches!(
        (direction, field.access),
        (Direction::Both, Some(Access::ReadOnly | Access::WriteOnly))
    );
    let presence = if field.presence == pons_ir::Presence::Optional || access_specific {
        "?"
    } else {
        ""
    };
    Ok(format!(
        "{key}{presence}: {}",
        local_type_ref(&field.ty, document, names, mode)?
    ))
}

fn is_object_type<'a>(mut ty: &'a TypeRef, document: &'a Document) -> bool {
    let mut visited = BTreeSet::new();
    loop {
        let TypeRef::Named(reference) = ty else {
            return false;
        };
        if !visited.insert(reference.id.clone()) {
            return false;
        }
        let Some(definition) = document.types.get(&reference.id) else {
            return false;
        };
        match &definition.definition {
            TypeKind::Struct { .. } => return true,
            TypeKind::Newtype { inner } => ty = inner,
            TypeKind::Enum { .. } => return false,
        }
    }
}

fn enum_type(
    direction: Direction,
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
            VariantPayload::Newtype { ty } => local_type_ref(ty, document, names, mode)?,
            VariantPayload::Tuple { items } => format!(
                "[{}]",
                items
                    .iter()
                    .map(|item| local_type_ref(item, document, names, mode))
                    .collect::<Result<Vec<_>, _>>()?
                    .join(", ")
            ),
            VariantPayload::Struct { fields } => format!(
                "{{ {} }}",
                fields
                    .iter()
                    .filter(|field| field_is_visible(field, direction))
                    .map(|field| field_type(field, direction, document, names, mode))
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
                    VariantPayload::Struct { .. } => format!("{{ {tag}: {wire} }} & {payload}"),
                    VariantPayload::Newtype { ty } if is_object_type(ty, document) => {
                        format!("{{ {tag}: {wire} }} & {payload}")
                    }
                    _ => {
                        return Err(EmitError(
                            "TypeScript cannot represent internally tagged newtype or tuple payloads"
                                .into(),
                        ));
                    }
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
    type_ref_with_namespace(reference, document, names, mode, true)
}

fn local_type_ref(
    reference: &TypeRef,
    document: &Document,
    names: &BTreeMap<String, String>,
    mode: EmitMode,
) -> Result<String, EmitError> {
    type_ref_with_namespace(reference, document, names, mode, false)
}

fn type_ref_with_namespace(
    reference: &TypeRef,
    document: &Document,
    names: &BTreeMap<String, String>,
    mode: EmitMode,
    qualify: bool,
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
                _ => return Ok(name),
            };
            Ok(format!("Extract<{name}, {filter}>"))
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
            InlineType::Opaque { .. } => Ok("unknown".into()),
            InlineType::Nullable { inner } => Ok(format!(
                "{} | null",
                type_ref_with_namespace(inner, document, names, mode, qualify)?
            )),
            InlineType::Array { item, .. } => Ok(format!(
                "Array<{}>",
                type_ref_with_namespace(item, document, names, mode, qualify)?
            )),
            InlineType::Map { value, .. } => Ok(format!(
                "Record<string, {}>",
                type_ref_with_namespace(value, document, names, mode, qualify)?
            )),
            InlineType::Tuple { items } => Ok(format!(
                "[{}]",
                items
                    .iter()
                    .map(|item| type_ref_with_namespace(item, document, names, mode, qualify))
                    .collect::<Result<Vec<_>, _>>()?
                    .join(", ")
            )),
        },
    }
}
