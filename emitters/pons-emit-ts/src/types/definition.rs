use std::collections::{BTreeMap, BTreeSet};

use pons_ir::{
    Access, Direction, Doc, Document, EnumRepr, ObjectOpenness, TypeDef, TypeKind, TypeRef,
    Variant, VariantPayload,
};

use crate::render::{object_type, string, union_type};
use crate::{EmitError, Newtypes, TsOptions};

use super::field::{field_is_visible, field_type};
use super::flatten::{flattened_field_types, flattened_opaque, openness_value};
use super::inline::{local_type_ref, type_ref_with_namespace};

pub(super) fn newtype_brand(
    definition: &TypeDef,
    name: &str,
    options: &TsOptions,
) -> Option<String> {
    if options.newtypes != Newtypes::Branded
        || !matches!(definition.definition, TypeKind::Newtype { .. })
    {
        return None;
    }
    let brand = format!("brand{name}");
    Some(format!("declare const {brand}: unique symbol;\n"))
}

pub(super) fn definition_type(
    id: &str,
    definition: &TypeDef,
    document: &Document,
    names: &BTreeMap<String, String>,
    options: &TsOptions,
) -> Result<String, EmitError> {
    match &definition.definition {
        TypeKind::Struct {
            fields,
            flattened,
            openness,
        } => {
            let flattened_types = flattened_field_types(flattened, document, names, options)?;
            let fields = fields
                .iter()
                .filter(|field| field_is_visible(field, definition.direction))
                .collect::<Vec<_>>();
            let mut known_types = fields
                .iter()
                .map(|field| local_type_ref(&field.ty, document, names, options))
                .collect::<Result<Vec<_>, _>>()?;
            known_types.extend(flattened_types);
            if let Some(carrier) = flattened_opaque(flattened, document) {
                return local_type_ref(carrier, document, names, options);
            }
            let mut members = fields
                .iter()
                .map(|field| {
                    field_type(
                        field,
                        definition.direction,
                        document,
                        names,
                        options,
                        false,
                        &mut BTreeSet::new(),
                    )
                    .map(|member| (field.doc.as_ref(), member))
                })
                .collect::<Result<Vec<_>, _>>()?;
            match openness {
                ObjectOpenness::Reject => {}
                ObjectOpenness::PreserveTyped { value } => {
                    let extra =
                        local_type_ref(openness_value(value, document), document, names, options)?;
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
        TypeKind::Enum { repr, variants } => enum_type(
            definition.direction,
            repr,
            variants,
            document,
            names,
            options,
        ),
        TypeKind::Newtype { inner } => {
            let inner = local_type_ref(inner, document, names, options)?;
            if options.newtypes != Newtypes::Branded {
                return Ok(inner);
            }
            Ok(format!(
                "{inner} & {{ readonly [brand{}]: never }}",
                names[id]
            ))
        }
    }
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
    options: &TsOptions,
) -> Result<String, EmitError> {
    let mut types: Vec<(Option<&Doc>, String)> = Vec::with_capacity(variants.len());
    for variant in variants {
        let mut narrowed = BTreeSet::new();
        let payload = payload_type(
            &variant.payload,
            direction,
            document,
            names,
            options,
            false,
            &mut narrowed,
        )?;
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
        types.push((variant.doc.as_ref(), ty));
    }
    if types.is_empty() {
        return Err(EmitError("an enum must have at least one variant".into()));
    }
    Ok(union_type(&types))
}

pub(super) fn payload_type(
    payload: &VariantPayload,
    direction: Direction,
    document: &Document,
    names: &BTreeMap<String, String>,
    options: &TsOptions,
    qualify: bool,
    narrowed: &mut BTreeSet<String>,
) -> Result<String, EmitError> {
    match payload {
        VariantPayload::Unit => Ok("null".into()),
        VariantPayload::Newtype { ty } => {
            type_ref_with_namespace(ty, document, names, options, qualify, narrowed)
        }
        VariantPayload::Tuple { items } => Ok(format!(
            "[{}]",
            items
                .iter()
                .map(|item| type_ref_with_namespace(
                    item, document, names, options, qualify, narrowed
                ))
                .collect::<Result<Vec<_>, _>>()?
                .join(", ")
        )),
        VariantPayload::Struct { fields } => {
            let mut members = fields
                .iter()
                .filter(|field| field_is_visible(field, direction))
                .map(|field| {
                    field_type(
                        field, direction, document, names, options, qualify, narrowed,
                    )
                    .map(|member| (field.doc.as_ref(), member))
                })
                .collect::<Result<Vec<_>, EmitError>>()?;
            members.push((None, String::from("[key: string]: unknown")));
            Ok(object_type(&members))
        }
    }
}
