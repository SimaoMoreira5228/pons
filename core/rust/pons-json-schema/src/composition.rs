use std::collections::{BTreeMap, BTreeSet};

use pons_core::TypeCx;
use pons_ir::{
    EnumRepr, Field, InlineType, ObjectOpenness, TypeKind, TypeRef, Variant, VariantPayload,
};
use serde_json::{Map, Value};

use crate::convert::{
    Context, UNEXPRESSIBLE, definition_name, lower_field, lower_ref, opaque, words,
};

const OWN_OBJECT_KEYS: [&str; 20] = [
    "$ref",
    "additionalProperties",
    "anyOf",
    "dependencies",
    "dependentRequired",
    "dependentSchemas",
    "discriminator",
    "else",
    "enum",
    "if",
    "maxProperties",
    "minProperties",
    "not",
    "oneOf",
    "patternProperties",
    "properties",
    "propertyNames",
    "required",
    "then",
    "unevaluatedProperties",
];

pub(crate) fn named(
    cx: &mut TypeCx,
    context: &Context<'_>,
    object: &Map<String, Value>,
) -> Option<TypeKind> {
    if object.contains_key("discriminator") {
        return Some(discriminated(cx, context, object));
    }
    object
        .contains_key("allOf")
        .then(|| all_of(cx, context, object))
}

pub(crate) fn inline_all_of(
    cx: &mut TypeCx,
    context: &Context<'_>,
    object: &Map<String, Value>,
) -> TypeRef {
    match merged_object(cx, context, object) {
        Ok((fields, openness)) => TypeRef::Inline(InlineType::Struct {
            fields,
            flattened: Vec::new(),
            openness: Box::new(openness),
        }),
        Err(message) => opaque(cx, "json-schema:allOf", message),
    }
}

fn discriminated(cx: &mut TypeCx, context: &Context<'_>, object: &Map<String, Value>) -> TypeKind {
    match discriminated_variants(cx, context, object) {
        Ok((tag, variants)) => TypeKind::Enum {
            repr: EnumRepr::Internal { tag },
            variants,
        },
        Err(message) => TypeKind::Newtype {
            inner: opaque(cx, "json-schema:discriminator", message),
        },
    }
}

fn discriminated_variants(
    cx: &mut TypeCx,
    context: &Context<'_>,
    object: &Map<String, Value>,
) -> Result<(String, Vec<Variant>), String> {
    let discriminator = object
        .get("discriminator")
        .and_then(Value::as_object)
        .ok_or_else(|| "the discriminator must be an object".to_owned())?;
    let tag = discriminator
        .get("propertyName")
        .and_then(Value::as_str)
        .filter(|tag| !tag.is_empty())
        .ok_or_else(|| "the discriminator needs a non-empty string propertyName".to_owned())?;
    let branches = ["oneOf", "anyOf"]
        .iter()
        .find_map(|keyword| object.get(*keyword))
        .ok_or_else(|| "a discriminator without oneOf or anyOf selects nothing".to_owned())?;
    let branches = branches
        .as_array()
        .ok_or_else(|| "the discriminated union must be an array of schemas".to_owned())?;

    for branch in branches {
        resolve_branch(context, branch)?;
    }

    let mut variants = Vec::new();
    match discriminator.get("mapping") {
        Some(Value::Object(mapping)) => {
            for (wire, target) in mapping {
                let reference = target.as_str().ok_or_else(|| {
                    format!("the discriminator mapping for \"{wire}\" must be a $ref string")
                })?;
                let branch = object_for_reference(context, reference).ok_or_else(|| {
                    format!(
                        "the discriminator mapping for \"{wire}\" does not point at an object schema"
                    )
                })?;
                variants.push(variant(cx, context, wire, branch, tag)?);
            }
        }
        Some(_) => return Err("the discriminator mapping must be an object".to_owned()),
        None => {
            for branch in branches {
                let resolved = resolve_branch(context, branch)?;
                let reference = branch
                    .as_object()
                    .and_then(|branch| branch.get("$ref"))
                    .and_then(Value::as_str);
                let wire = match reference {
                    Some(reference) => definition_name(reference)
                        .ok_or_else(|| {
                            format!(
                                "the reference \"{reference}\" does not point into this schema's definitions"
                            )
                        })?
                        .to_owned(),
                    None => branch
                        .as_object()
                        .and_then(|branch| branch.get("title"))
                        .and_then(Value::as_str)
                        .ok_or_else(|| {
                            "a discriminated union without a mapping needs a $ref or a title on each inline branch"
                                .to_owned()
                        })?
                        .to_owned(),
                };
                variants.push(variant(cx, context, &wire, resolved, tag)?);
            }
        }
    }
    if variants.is_empty() {
        return Err("the discriminated union has no variants".to_owned());
    }
    Ok((tag.to_owned(), variants))
}

fn variant(
    cx: &mut TypeCx,
    context: &Context<'_>,
    wire: &str,
    object: &Map<String, Value>,
    tag: &str,
) -> Result<Variant, String> {
    let required = required_of(object);
    let mut fields = Vec::new();
    if let Some(properties) = object.get("properties").and_then(Value::as_object) {
        for (name, schema) in properties {
            if name == tag {
                if required.contains(name.as_str()) && contradicts_string(schema) {
                    return Err(format!(
                        "variant \"{wire}\" declares the discriminator tag \"{tag}\" as a required non-string field"
                    ));
                }
                continue;
            }
            fields.push(lower_field(
                cx,
                context,
                name,
                schema,
                required.contains(name.as_str()),
            ));
        }
    }
    Ok(Variant {
        wire: wire.to_owned(),
        ident: words(wire),
        payload: VariantPayload::Struct { fields },
        doc: None,
        diagnostics: Vec::new(),
        ext: BTreeMap::new(),
    })
}

fn contradicts_string(schema: &Value) -> bool {
    let Some(object) = schema.as_object() else {
        return false;
    };
    if let Some(ty) = object.get("type")
        && ty.as_str() != Some("string")
    {
        return true;
    }
    if let Some(constant) = object.get("const") {
        return !constant.is_string();
    }
    object
        .get("enum")
        .and_then(Value::as_array)
        .is_some_and(|values| !values.iter().all(Value::is_string))
}

fn all_of(cx: &mut TypeCx, context: &Context<'_>, object: &Map<String, Value>) -> TypeKind {
    match merged_object(cx, context, object) {
        Ok((fields, openness)) => TypeKind::Struct {
            fields,
            flattened: Vec::new(),
            openness,
        },
        Err(message) => TypeKind::Newtype {
            inner: opaque(cx, "json-schema:allOf", message),
        },
    }
}

fn merged_object(
    cx: &mut TypeCx,
    context: &Context<'_>,
    object: &Map<String, Value>,
) -> Result<(Vec<Field>, ObjectOpenness), String> {
    if let Some(keyword) = OWN_OBJECT_KEYS
        .iter()
        .find(|keyword| object.contains_key(**keyword))
    {
        return Err(format!(
            "allOf beside \"{keyword}\" would drop that constraint, so the composition stays opaque"
        ));
    }
    if let Some(ty) = object.get("type")
        && ty.as_str() != Some("object")
    {
        return Err(
            "allOf beside a non-object type is a union, not an object composition".to_owned(),
        );
    }
    let branches = object
        .get("allOf")
        .and_then(Value::as_array)
        .ok_or_else(|| "allOf must be an array of schemas".to_owned())?;

    let mut properties: Vec<(String, Value)> = Vec::new();
    let mut positions: BTreeMap<String, usize> = BTreeMap::new();
    let mut required = BTreeSet::new();
    let mut additional = AdditionalProperties::default();
    for branch in branches {
        let branch = object_schema(context, branch).ok_or_else(|| {
            "every allOf branch must be an object schema or a $ref to one".to_owned()
        })?;
        if let Some(branch_properties) = branch.get("properties").and_then(Value::as_object) {
            for (wire, property) in branch_properties {
                if let Some(index) = positions.get(wire) {
                    if properties[*index].1 != *property {
                        return Err(format!(
                            "allOf branches disagree on property \"{wire}\", so merging would have to choose one"
                        ));
                    }
                } else {
                    positions.insert(wire.clone(), properties.len());
                    properties.push((wire.clone(), property.clone()));
                }
            }
        }
        required.extend(required_of(branch).into_iter().map(str::to_owned));
        additional.merge(branch)?;
    }

    let mut fields = Vec::with_capacity(properties.len());
    for (wire, schema) in &properties {
        fields.push(lower_field(
            cx,
            context,
            wire,
            schema,
            required.contains(wire),
        ));
    }
    Ok((fields, additional.finish(cx, context)))
}

#[derive(Default)]
struct AdditionalProperties {
    reject: bool,
    schemas: Vec<Value>,
}

impl AdditionalProperties {
    fn merge(&mut self, branch: &Map<String, Value>) -> Result<(), String> {
        match branch.get("additionalProperties") {
            None | Some(Value::Bool(true)) => {}
            Some(Value::Bool(false)) => self.reject = true,
            Some(value @ Value::Object(_)) => {
                if !self.schemas.contains(value) {
                    self.schemas.push(value.clone());
                }
            }
            Some(_) => {
                return Err(
                    "additionalProperties must be a schema or a boolean in every allOf branch"
                        .to_owned(),
                );
            }
        }
        Ok(())
    }

    fn finish(self, cx: &mut TypeCx, context: &Context<'_>) -> ObjectOpenness {
        if self.reject {
            return ObjectOpenness::Reject;
        }
        let Some(schema) = self.schemas.first() else {
            return ObjectOpenness::PreserveOpaque;
        };
        if self.schemas.len() > 1 {
            cx.push_type_diagnostic(
                UNEXPRESSIBLE,
                "allOf branches disagree on additionalProperties, so unknown properties keep an untyped value",
            );
            return ObjectOpenness::PreserveOpaque;
        }
        ObjectOpenness::PreserveTyped {
            value: lower_ref(cx, context, schema),
        }
    }
}

fn resolve_branch<'a>(
    context: &'a Context<'_>,
    branch: &'a Value,
) -> Result<&'a Map<String, Value>, String> {
    object_schema(context, branch).ok_or_else(|| {
        "every discriminated union branch must be an object schema or a $ref to one".to_owned()
    })
}

fn object_for_reference<'a>(
    context: &'a Context<'_>,
    reference: &str,
) -> Option<&'a Map<String, Value>> {
    object_schema(context, context.defs.get(definition_name(reference)?)?)
}

fn object_schema<'a>(
    context: &'a Context<'_>,
    schema: &'a Value,
) -> Option<&'a Map<String, Value>> {
    let mut current = schema;
    let mut seen = BTreeSet::new();
    loop {
        let object = current.as_object()?;
        if let Some(reference) = object.get("$ref").and_then(Value::as_str) {
            let name = definition_name(reference)?;
            if !seen.insert(name) {
                return None;
            }
            current = context.defs.get(name)?;
            continue;
        }
        return is_object_schema(object).then_some(object);
    }
}

fn is_object_schema(object: &Map<String, Value>) -> bool {
    if ["oneOf", "anyOf", "allOf", "discriminator", "enum"]
        .iter()
        .any(|keyword| object.contains_key(*keyword))
    {
        return false;
    }
    match object.get("type") {
        Some(Value::String(name)) => name == "object",
        Some(_) => false,
        None => object.contains_key("properties") || object.contains_key("additionalProperties"),
    }
}

fn required_of(object: &Map<String, Value>) -> BTreeSet<&str> {
    object
        .get("required")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect()
}
