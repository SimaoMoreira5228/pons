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
        if let TypeKind::Newtype { inner } = &definition.definition
            && newtype_cycle(id, inner, document)
        {
            return Err(EmitError(format!(
                "the type {id} cannot be resolved because newtypes form a cycle"
            )));
        }
        let name = &names[id];
        output.push_str(&format!(
            "export type {name} = {};\n\n",
            definition_type(definition, document, names, mode)?
        ));
    }
    Ok(output)
}

fn newtype_cycle(id: &str, inner: &TypeRef, document: &Document) -> bool {
    reenters(id, inner, document, &mut BTreeSet::new())
}

fn reenters(
    id: &str,
    reference: &TypeRef,
    document: &Document,
    seen: &mut BTreeSet<(String, Option<String>)>,
) -> bool {
    match reference {
        TypeRef::Named(named) => {
            if named.id == id {
                return true;
            }
            if !seen.insert((named.id.clone(), named.variant.clone())) {
                return false;
            }
            let Some(definition) = document
                .types
                .get(&named.id)
                .map(|definition| &definition.definition)
            else {
                return false;
            };
            let next: Vec<&TypeRef> = match definition {
                TypeKind::Newtype { inner } => vec![inner],
                TypeKind::Enum { variants, .. } => match named
                    .variant
                    .as_deref()
                    .and_then(|variant| variants.iter().find(|entry| entry.wire == variant))
                {
                    Some(Variant {
                        payload: VariantPayload::Newtype { ty },
                        ..
                    }) => vec![ty],
                    _ => return false,
                },
                TypeKind::Struct { .. } => return false,
            };
            next.into_iter()
                .any(|reference| reenters(id, reference, document, seen))
        }
        TypeRef::Inline(InlineType::Nullable { inner }) => reenters(id, inner, document, seen),
        TypeRef::Inline(InlineType::Array { item, .. }) => reenters(id, item, document, seen),
        TypeRef::Inline(InlineType::Map { key, value }) => {
            reenters(id, key, document, seen) || reenters(id, value, document, seen)
        }
        TypeRef::Inline(InlineType::Tuple { items }) => {
            items.iter().any(|item| reenters(id, item, document, seen))
        }
        _ => false,
    }
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
            if let ObjectOpenness::PreserveTyped { value } = openness {
                let extra = local_type_ref(openness_value(value, document), document, names, mode)?;
                if known_types.iter().any(|ty| *ty != extra) {
                    return Err(EmitError(
                        "TypeScript cannot represent typed object openness when direct or flattened fields have different types"
                            .into(),
                    ));
                }
            }
            if let Some(carrier) = flattened_opaque(flattened, document) {
                return local_type_ref(carrier, document, names, mode);
            }
            let mut members = fields
                .iter()
                .map(|field| {
                    field_type(
                        field,
                        definition.direction,
                        document,
                        names,
                        mode,
                        false,
                        &mut BTreeSet::new(),
                    )
                })
                .collect::<Result<Vec<_>, _>>()?;
            match openness {
                ObjectOpenness::Reject => {}
                ObjectOpenness::PreserveTyped { value } => {
                    let extra =
                        local_type_ref(openness_value(value, document), document, names, mode)?;
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
    visiting.push(named.id.clone());
    match &definition.definition {
        TypeKind::Struct {
            fields,
            flattened,
            openness,
        } => {
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
        }
        TypeKind::Newtype { inner } => {
            collect_flattened_field_types(inner, document, names, mode, visited, visiting, result)?
        }
        TypeKind::Enum { .. } => return Err(not_a_struct_member(reference)),
    }
    visiting.pop();
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

fn flattened_opaque<'a>(references: &'a [TypeRef], document: &Document) -> Option<&'a TypeRef> {
    let mut visited = BTreeSet::new();
    references
        .iter()
        .find(|reference| resolves_to_opaque(reference, document, &mut visited))
}

fn resolves_to_opaque(
    reference: &TypeRef,
    document: &Document,
    visited: &mut BTreeSet<String>,
) -> bool {
    let TypeRef::Named(named) = reference else {
        return matches!(reference, TypeRef::Inline(InlineType::Opaque { .. }));
    };
    if named.variant.is_some() || !visited.insert(named.id.clone()) {
        return false;
    }
    let Some(definition) = document
        .types
        .get(&named.id)
        .map(|definition| &definition.definition)
    else {
        return false;
    };
    match definition {
        TypeKind::Newtype { inner } => resolves_to_opaque(inner, document, visited),
        TypeKind::Struct { flattened, .. } => flattened
            .iter()
            .any(|reference| resolves_to_opaque(reference, document, visited)),
        TypeKind::Enum { .. } => false,
    }
}

fn field_type(
    field: &Field,
    direction: Direction,
    document: &Document,
    names: &BTreeMap<String, String>,
    mode: EmitMode,
    qualify: bool,
    narrowed: &mut BTreeSet<String>,
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
        type_ref_with_namespace(&field.ty, document, names, mode, qualify, narrowed)?
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
        let mut narrowed = BTreeSet::new();
        let payload = payload_type(
            &variant.payload,
            direction,
            document,
            names,
            mode,
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
        types.push(ty);
    }
    if types.is_empty() {
        return Err(EmitError("an enum must have at least one variant".into()));
    }
    Ok(types.join(" | "))
}

fn payload_type(
    payload: &VariantPayload,
    direction: Direction,
    document: &Document,
    names: &BTreeMap<String, String>,
    mode: EmitMode,
    qualify: bool,
    narrowed: &mut BTreeSet<String>,
) -> Result<String, EmitError> {
    match payload {
        VariantPayload::Unit => Ok("null".into()),
        VariantPayload::Newtype { ty } => {
            type_ref_with_namespace(ty, document, names, mode, qualify, narrowed)
        }
        VariantPayload::Tuple { items } => Ok(format!(
            "[{}]",
            items
                .iter()
                .map(|item| type_ref_with_namespace(item, document, names, mode, qualify, narrowed))
                .collect::<Result<Vec<_>, _>>()?
                .join(", ")
        )),
        VariantPayload::Struct { fields } => {
            let mut members = fields
                .iter()
                .filter(|field| field_is_visible(field, direction))
                .map(|field| field_type(field, direction, document, names, mode, qualify, narrowed))
                .collect::<Result<Vec<_>, _>>()?;
            members.push("[key: string]: unknown".into());
            Ok(format!("{{ {} }}", members.join("; ")))
        }
    }
}

pub(super) fn type_ref(
    reference: &TypeRef,
    document: &Document,
    names: &BTreeMap<String, String>,
    mode: EmitMode,
) -> Result<String, EmitError> {
    type_ref_with_namespace(reference, document, names, mode, true, &mut BTreeSet::new())
}

fn local_type_ref(
    reference: &TypeRef,
    document: &Document,
    names: &BTreeMap<String, String>,
    mode: EmitMode,
) -> Result<String, EmitError> {
    type_ref_with_namespace(
        reference,
        document,
        names,
        mode,
        false,
        &mut BTreeSet::new(),
    )
}

fn type_ref_with_namespace(
    reference: &TypeRef,
    document: &Document,
    names: &BTreeMap<String, String>,
    mode: EmitMode,
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
                    mode,
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
                type_ref_with_namespace(inner, document, names, mode, qualify, narrowed)?
            )),
            InlineType::Array { item, .. } => Ok(format!(
                "Array<{}>",
                type_ref_with_namespace(item, document, names, mode, qualify, narrowed)?
            )),
            InlineType::Map { value, .. } => Ok(format!(
                "Record<string, {}>",
                type_ref_with_namespace(value, document, names, mode, qualify, narrowed)?
            )),
            InlineType::Tuple { items } => Ok(format!(
                "[{}]",
                items
                    .iter()
                    .map(|item| {
                        type_ref_with_namespace(item, document, names, mode, qualify, narrowed)
                    })
                    .collect::<Result<Vec<_>, _>>()?
                    .join(", ")
            )),
        },
    }
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    fn document(types: Value) -> pons_ir::Document {
        let mut raw: Value =
            serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
        for (id, definition) in types.as_object().unwrap() {
            raw["types"][id] = definition.clone();
        }
        serde_json::from_value(raw).unwrap()
    }

    fn newtype(name: &str, inner: &str) -> Value {
        json!({"kind":"newtype","name":[name],"direction":"both","inner":{"ref":inner}})
    }

    fn newtype_over(name: &[&str], inner: Value) -> Value {
        json!({"kind":"newtype","name":name,"direction":"both","inner":inner})
    }

    fn cycle_error(types: Value) -> String {
        crate::emit(&document(types))
            .map(|_| String::from("the document was accepted"))
            .unwrap_err()
            .0
    }

    fn untagged_variant(wire: &str, payload: Value) -> Value {
        json!({"wire": wire, "ident": [wire], "payload": payload})
    }

    fn assert_cycle(error: &str, members: &[&str]) {
        for member in members {
            let expected =
                format!("the type {member} cannot be resolved because newtypes form a cycle");
            if error.contains(&expected) {
                return;
            }
        }
        panic!("{error}");
    }

    #[test]
    fn rejects_a_self_referential_newtype() {
        let document = document(json!({"Loop": newtype("loop", "Loop")}));
        let error = crate::emit(&document).unwrap_err();
        assert!(
            error
                .0
                .contains("the type Loop cannot be resolved because newtypes form a cycle"),
            "{}",
            error.0
        );
    }

    #[test]
    fn rejects_a_cycle_of_newtypes() {
        let document = document(json!({"A": newtype("a", "B"), "B": newtype("b", "A")}));
        let error = crate::emit(&document).unwrap_err();
        for member in ["A", "B"] {
            let expected =
                format!("the type {member} cannot be resolved because newtypes form a cycle");
            if error.0.contains(&expected) {
                return;
            }
        }
        panic!("{}", error.0);
    }

    #[test]
    fn rejects_a_newtype_that_returns_to_itself_through_an_inline_arm() {
        assert_cycle(
            &cycle_error(json!({
                "Loop": newtype_over(&["loop"], json!({"kind": "nullable", "inner": {"ref": "Loop"}}))
            })),
            &["Loop"],
        );

        assert_cycle(
            &cycle_error(json!({
                "Loop": newtype_over(
                    &["loop"],
                    json!({"kind": "map", "key": {"kind": "string"}, "value": {"ref": "Loop"}})
                )
            })),
            &["Loop"],
        );
    }

    #[test]
    fn rejects_two_newtypes_that_return_to_one_another_through_an_inline_arm() {
        assert_cycle(
            &cycle_error(json!({
                "A": newtype_over(&["a"], json!({"kind": "nullable", "inner": {"ref": "B"}})),
                "B": newtype_over(&["b"], json!({"kind": "nullable", "inner": {"ref": "A"}}))
            })),
            &["A", "B"],
        );
    }

    #[test]
    fn rejects_a_newtype_that_reaches_itself_through_an_untagged_variant() {
        assert_cycle(
            &cycle_error(json!({
                "Loop": newtype_over(&["loop"], json!({"ref": "Kinds", "variant": "again"})),
                "Kinds": {
                    "kind": "enum", "name": ["kinds"], "direction": "both", "repr": {"kind": "untagged"},
                    "variants": [
                        untagged_variant("stop", json!({"kind": "unit"})),
                        untagged_variant(
                            "again",
                            json!({"kind": "newtype", "ty": {"kind": "nullable", "inner": {"ref": "Loop"}}})
                        )
                    ]
                }
            })),
            &["Loop", "Kinds"],
        );
    }

    #[test]
    fn rejects_a_cycle_that_closes_through_a_second_untagged_narrowing() {
        assert_cycle(
            &cycle_error(json!({
                "A": newtype_over(&["a"], json!({"ref": "Kinds", "variant": "left"})),
                "B": newtype_over(&["b"], json!({"ref": "Kinds", "variant": "right"})),
                "Kinds": {
                    "kind": "enum", "name": ["kinds"], "direction": "both", "repr": {"kind": "untagged"},
                    "variants": [
                        untagged_variant("left", json!({"kind": "newtype", "ty": {"ref": "B"}})),
                        untagged_variant("right", json!({"kind": "newtype", "ty": {"ref": "A"}}))
                    ]
                }
            })),
            &["A", "B"],
        );
    }

    #[test]
    fn rejects_a_cycle_that_closes_through_a_second_narrowing_in_the_other_order() {
        assert_cycle(
            &cycle_error(json!({
                "A": newtype_over(&["a"], json!({"ref": "Kinds", "variant": "right"})),
                "B": newtype_over(&["b"], json!({"ref": "Kinds", "variant": "left"})),
                "Kinds": {
                    "kind": "enum", "name": ["kinds"], "direction": "both", "repr": {"kind": "untagged"},
                    "variants": [
                        untagged_variant("left", json!({"kind": "newtype", "ty": {"ref": "A"}})),
                        untagged_variant("right", json!({"kind": "newtype", "ty": {"ref": "B"}}))
                    ]
                }
            })),
            &["A", "B"],
        );
    }

    #[test]
    fn a_narrowed_untagged_variant_that_names_itself_keeps_the_whole_enum() {
        let document = document(json!({
            "Tree": {
                "kind": "enum", "name": ["tree"], "direction": "both", "repr": {"kind": "untagged"},
                "variants": [
                    {"wire": "leaf", "ident": ["leaf"], "payload": {"kind": "unit"}},
                    {
                        "wire": "node",
                        "ident": ["node"],
                        "payload": {
                            "kind": "newtype",
                            "ty": {"kind": "array", "item": {"ref": "Tree", "variant": "node"}}
                        }
                    }
                ]
            },
            "Root": newtype_over(&["root"], json!({"ref": "Tree", "variant": "node"}))
        }));
        let types = crate::emit(&document)
            .unwrap()
            .into_iter()
            .find(|file| file.path == "types.ts")
            .unwrap()
            .content;
        assert!(types.contains("export type Root = Array<Tree>;"), "{types}");
    }

    #[test]
    fn emits_a_newtype_that_resolves_to_a_struct() {
        let document = document(json!({"Alias": newtype("alias", "User")}));
        let types = crate::emit(&document)
            .unwrap()
            .into_iter()
            .find(|file| file.path == "types.ts")
            .unwrap()
            .content;
        assert!(types.contains("export type Alias = User;"), "{types}");
    }
}
