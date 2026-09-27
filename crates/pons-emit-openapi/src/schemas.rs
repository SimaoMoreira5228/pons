use pons_ir::{
    Access, ByteEncoding, DecimalWire, EnumRepr, FieldDefault, InlineType, IntegerWire,
    ObjectOpenness, Presence, TypeDef, TypeKind, TypeRef, Variant, VariantPayload,
};
use serde_json::{Map, Value, json};

use crate::{Builder, EmitError};

fn reference(name: &str) -> Value {
    json!({"$ref": format!("#/components/schemas/{}", name.replace('~', "~0").replace('/', "~1"))})
}

fn add_doc(schema: &mut Value, doc: Option<&pons_ir::Doc>) {
    let Some(doc) = doc else { return };
    if let Some(description) = &doc.description {
        schema["description"] = json!(description);
    }
    if let Some(summary) = &doc.summary {
        schema["title"] = json!(summary);
    }
    if let Some(deprecated) = doc.deprecated {
        schema["deprecated"] = json!(deprecated);
    }
}

fn add_extensions(schema: &mut Value, key: &str, metadata: Value) {
    let extensions = schema
        .as_object_mut()
        .unwrap()
        .entry("x-pons")
        .or_insert_with(|| json!({}));
    extensions[key] = metadata;
}

fn integer_bounds(bits: u16, signed: bool) -> Option<(Value, Value)> {
    if signed {
        if bits == 64 {
            return Some((json!(i64::MIN), json!(i64::MAX)));
        }
        if bits < 64 {
            let edge = 1_i64 << (bits - 1);
            return Some((json!(-edge), json!(edge - 1)));
        }
    } else if bits == 64 {
        return Some((json!(0), json!(u64::MAX)));
    } else if bits < 64 {
        return Some((json!(0), json!((1_u64 << bits) - 1)));
    }
    None
}

impl<'a> Builder<'a> {
    pub(super) fn field_schema(&self, field: &pons_ir::Field) -> Result<Value, EmitError> {
        let mut schema = self.type_schema(&field.ty)?;
        match field.access {
            Some(Access::ReadOnly) => schema["readOnly"] = json!(true),
            Some(Access::WriteOnly) => schema["writeOnly"] = json!(true),
            _ => {}
        }
        add_doc(&mut schema, field.doc.as_ref());
        if let FieldDefault::Value(default) = &field.default {
            schema["default"] = default.clone();
        }
        add_extensions(
            &mut schema,
            "field",
            json!({
                "ident":field.ident,
                "presence":field.presence,
                "access":field.access,
                "doc":field.doc,
                "ext":field.ext,
                "diagnostics":field.diagnostics,
            }),
        );
        Ok(schema)
    }

    pub(super) fn schemas(&self) -> Result<Map<String, Value>, EmitError> {
        let mut schemas = Map::new();
        for (id, definition) in &self.document.types {
            let mut schema = self.definition_schema(definition)?;
            add_doc(&mut schema, definition.doc.as_ref());
            let openness = match &definition.definition {
                TypeKind::Struct { openness, .. } => Some(openness),
                _ => None,
            };
            add_extensions(
                &mut schema,
                "type",
                json!({"id":id,"name":definition.name,"direction":definition.direction,"origin":definition.origin,"doc":definition.doc,"objectOpenness":openness,"ext":definition.ext,"diagnostics":definition.diagnostics}),
            );
            schemas.insert(self.schema_names[id].clone(), schema);
        }
        Ok(schemas)
    }

    fn definition_schema(&self, definition: &TypeDef) -> Result<Value, EmitError> {
        match &definition.definition {
            TypeKind::Struct {
                fields,
                flattened,
                openness,
            } => {
                let mut object = fields_schema_with(self, fields)?;
                let additional = match openness {
                    ObjectOpenness::Reject => json!(false),
                    ObjectOpenness::PreserveTyped { value } => self.type_schema(value)?,
                    ObjectOpenness::PreserveOpaque => json!(true),
                };
                if flattened.is_empty() {
                    object["additionalProperties"] = additional;
                    Ok(object)
                } else {
                    object["unevaluatedProperties"] = additional;
                    let mut all_of = vec![object];
                    for ty in flattened {
                        all_of.push(self.type_schema(ty)?);
                    }
                    Ok(json!({"allOf":all_of}))
                }
            }
            TypeKind::Enum { repr, variants } => self.enum_schema(repr, variants),
            TypeKind::Newtype { inner } => self.type_schema(inner),
        }
    }

    pub(super) fn type_schema(&self, ty: &TypeRef) -> Result<Value, EmitError> {
        match ty {
            TypeRef::Named(named) => {
                let definition = self
                    .document
                    .types
                    .get(&named.id)
                    .ok_or_else(|| EmitError(format!("unknown type {}", named.id)))?;
                if let Some(variant_name) = &named.variant {
                    let TypeKind::Enum { repr, variants } = &definition.definition else {
                        return Err(EmitError(format!("type {} is not an enum", named.id)));
                    };
                    let variant = variants
                        .iter()
                        .find(|candidate| candidate.wire == *variant_name)
                        .ok_or_else(|| EmitError(format!("unknown variant {variant_name}")))?;
                    return self.enum_variant_schema(repr, variant);
                }
                Ok(reference(&self.schema_names[&named.id]))
            }
            TypeRef::Inline(ty) => self.inline_schema(ty),
        }
    }

    fn inline_schema(&self, ty: &InlineType) -> Result<Value, EmitError> {
        Ok(match ty {
            InlineType::Bool => json!({"type":"boolean"}),
            InlineType::Int {
                bits,
                signed,
                wire,
                minimum,
                maximum,
                multiple_of,
            } => {
                if *wire == IntegerWire::String {
                    let pattern = if *signed {
                        "^-?(0|[1-9][0-9]*)$"
                    } else {
                        "^(0|[1-9][0-9]*)$"
                    };
                    json!({
                        "type":"string",
                        "pattern":pattern,
                        "x-pons-integer":{
                            "bits":bits,
                            "signed":signed,
                            "wire":"string",
                            "minimum":minimum,
                            "maximum":maximum,
                            "multiple_of":multiple_of
                        }
                    })
                } else {
                    let mut schema = json!({"type":"integer"});
                    match (*bits, *signed) {
                        (32, true) => schema["format"] = json!("int32"),
                        (64, true) => schema["format"] = json!("int64"),
                        _ => {}
                    }
                    schema["x-pons-integer"] = json!({"bits":bits,"signed":signed,"wire":"number"});
                    if let Some((minimum, maximum)) = integer_bounds(*bits, *signed) {
                        schema["minimum"] = minimum;
                        schema["maximum"] = maximum;
                    }
                    if let Some(minimum) = minimum {
                        schema["minimum"] = minimum.clone();
                    }
                    if let Some(maximum) = maximum {
                        schema["maximum"] = maximum.clone();
                    }
                    if let Some(multiple_of) = multiple_of {
                        schema["multipleOf"] = multiple_of.clone();
                    }
                    schema
                }
            }
            InlineType::Float { bits } => {
                json!({"type":"number","format":if *bits == 32 {"float"} else {"double"}})
            }
            InlineType::Decimal { wire } => json!({
                "type":if *wire == DecimalWire::String {"string"} else {"number"},
                "x-pons-decimal-wire":wire,
            }),
            InlineType::String {
                format,
                min_length,
                max_length,
                pattern,
            } => {
                let mut schema = json!({"type":"string"});
                if let Some(format) = format {
                    schema["format"] = json!(format);
                }
                if let Some(minimum) = min_length {
                    schema["minLength"] = json!(minimum);
                }
                if let Some(maximum) = max_length {
                    schema["maxLength"] = json!(maximum);
                }
                if let Some(pattern) = pattern {
                    schema["pattern"] = json!(pattern);
                }
                schema
            }
            InlineType::Bytes { encoding } => match encoding {
                ByteEncoding::Base64 => json!({"type":"string","contentEncoding":"base64"}),
                ByteEncoding::Base64url => json!({"type":"string","contentEncoding":"base64url"}),
                ByteEncoding::Hex => json!({"type":"string","pattern":"^(?:[0-9a-fA-F]{2})*$"}),
                ByteEncoding::Array => {
                    json!({"type":"array","items":{"type":"integer","minimum":0,"maximum":255}})
                }
            },
            InlineType::Null => json!({"type":"null"}),
            InlineType::Any => json!({}),
            InlineType::Nullable { inner } => {
                json!({"anyOf":[self.type_schema(inner)?,{"type":"null"}]})
            }
            InlineType::Array { item, unique } => {
                let mut schema = json!({"type":"array","items":self.type_schema(item)?});
                if let Some(unique) = unique {
                    schema["uniqueItems"] = json!(unique);
                }
                schema
            }
            InlineType::Map { key, value } => {
                json!({
                    "type":"object",
                    "propertyNames":self.type_schema(key)?,
                    "additionalProperties":self.type_schema(value)?
                })
            }
            InlineType::Tuple { items } => {
                json!({"type":"array","prefixItems":items.iter().map(|item| self.type_schema(item)).collect::<Result<Vec<_>,_>>()?,"minItems":items.len(),"maxItems":items.len(),"items":false})
            }
        })
    }

    fn enum_schema(&self, repr: &EnumRepr, variants: &[Variant]) -> Result<Value, EmitError> {
        if let EnumRepr::Internal { .. } = repr {
            for variant in variants {
                match &variant.payload {
                    VariantPayload::Unit | VariantPayload::Struct { .. } => {}
                    VariantPayload::Newtype { ty } if self.object_type(ty, &mut Vec::new())? => {}
                    VariantPayload::Newtype { .. } | VariantPayload::Tuple { .. } => {
                        return Err(EmitError(format!(
                            "internally tagged enum variant {} must have an object payload",
                            variant.wire
                        )));
                    }
                }
            }
        }
        let schemas = variants
            .iter()
            .map(|variant| self.enum_variant_schema(repr, variant))
            .collect::<Result<Vec<_>, _>>()?;
        let mut schema = if matches!(repr, EnumRepr::Untagged) {
            json!({"anyOf":schemas})
        } else {
            json!({"oneOf":schemas})
        };
        schema["x-pons-enum-repr"] = json!(repr);
        Ok(schema)
    }

    fn object_type(&self, ty: &TypeRef, visited: &mut Vec<String>) -> Result<bool, EmitError> {
        match ty {
            TypeRef::Inline(InlineType::Map { .. }) => Ok(true),
            TypeRef::Named(named) if visited.contains(&named.id) => Ok(false),
            TypeRef::Named(named) => {
                let definition = self
                    .document
                    .types
                    .get(&named.id)
                    .ok_or_else(|| EmitError(format!("unknown type {}", named.id)))?;
                match &definition.definition {
                    TypeKind::Struct { .. } => Ok(true),
                    TypeKind::Newtype { inner } => {
                        visited.push(named.id.clone());
                        let result = self.object_type(inner, visited);
                        visited.pop();
                        result
                    }
                    TypeKind::Enum { .. } => Ok(false),
                }
            }
            TypeRef::Inline(_) => Ok(false),
        }
    }

    fn enum_variant_schema(&self, repr: &EnumRepr, variant: &Variant) -> Result<Value, EmitError> {
        let payload = self.payload_schema(&variant.payload)?;
        let mut schema = match repr {
            EnumRepr::String => json!({"type":"string","const":variant.wire}),
            EnumRepr::External => match variant.payload {
                VariantPayload::Unit => json!({"type":"string","const":variant.wire}),
                _ => {
                    let mut properties = Map::new();
                    properties.insert(variant.wire.clone(), payload);
                    json!({"type":"object","properties":properties,"required":[variant.wire],"minProperties":1,"maxProperties":1})
                }
            },
            EnumRepr::Internal { tag } => match &variant.payload {
                VariantPayload::Unit => tagged_schema(tag, &variant.wire),
                VariantPayload::Struct { fields } => {
                    let mut schema = fields_schema_with(self, fields)?;
                    schema["properties"][tag] = json!({"const":variant.wire});
                    if schema.get("required").is_none() {
                        schema["required"] = json!([]);
                    }
                    schema["required"].as_array_mut().unwrap().push(json!(tag));
                    schema
                }
                _ => json!({"allOf":[tagged_schema(tag, &variant.wire),payload]}),
            },
            EnumRepr::Adjacent { tag, content } => {
                let mut properties = Map::new();
                properties.insert(tag.clone(), json!({"const":variant.wire}));
                let mut required = vec![json!(tag)];
                if !matches!(variant.payload, VariantPayload::Unit) {
                    properties.insert(content.clone(), payload);
                    required.push(json!(content));
                }
                json!({"type":"object","properties":properties,"required":required})
            }
            EnumRepr::Untagged => payload,
        };
        add_doc(&mut schema, variant.doc.as_ref());
        add_extensions(
            &mut schema,
            "variant",
            json!({
                "wire":variant.wire,
                "ident":variant.ident,
                "doc":variant.doc,
                "ext":variant.ext,
                "diagnostics":variant.diagnostics,
            }),
        );
        Ok(schema)
    }

    fn payload_schema(&self, payload: &VariantPayload) -> Result<Value, EmitError> {
        match payload {
            VariantPayload::Unit => Ok(json!({"type":"null"})),
            VariantPayload::Newtype { ty } => self.type_schema(ty),
            VariantPayload::Tuple { items } => self.inline_schema(&InlineType::Tuple {
                items: items.clone(),
            }),
            VariantPayload::Struct { fields } => fields_schema_with(self, fields),
        }
    }
}

fn tagged_schema(tag: &str, variant: &str) -> Value {
    let mut properties = Map::new();
    properties.insert(tag.into(), json!({"const":variant}));
    json!({"type":"object","properties":properties,"required":[tag]})
}

fn fields_schema_with(
    builder: &Builder<'_>,
    fields: &[pons_ir::Field],
) -> Result<Value, EmitError> {
    let mut properties = Map::new();
    let mut required = Vec::new();
    for field in fields {
        let mut schema = builder.field_schema(field)?;
        properties.insert(field.wire.clone(), schema.take());
        if field.presence == Presence::Required {
            required.push(field.wire.clone());
        }
    }
    let mut object = json!({"type":"object","properties":properties});
    if !required.is_empty() {
        object["required"] = json!(required);
    }
    Ok(object)
}
