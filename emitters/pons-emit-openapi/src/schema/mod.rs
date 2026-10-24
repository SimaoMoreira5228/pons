mod enums;
mod objects;
mod patterns;
mod scalars;

pub(crate) use self::scalars::{Place, WireForm};

use pons_ir::{EnumRepr, TypeDef, TypeKind, TypeRef};
use serde_json::{Map, Value, json};

use crate::{Builder, EmitError};

use self::objects::fields_schema_with;

impl<'a> Builder<'a> {
    pub(crate) fn schemas(&self) -> Result<Map<String, Value>, EmitError> {
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
                let additional = self.openness_schema(openness)?;
                if flattened.is_empty() {
                    object["additionalProperties"] = additional;
                    Ok(object)
                } else {
                    let mut all_of = vec![object];
                    for ty in flattened {
                        self.flattened_schema(ty, &mut Vec::new(), &mut all_of)?;
                    }
                    let mut wrapper = json!({"allOf":all_of});
                    wrapper["unevaluatedProperties"] = additional;
                    Ok(wrapper)
                }
            }
            TypeKind::Enum { repr, variants } => self.enum_schema(repr, variants),
            TypeKind::Newtype { inner } => self.type_schema(inner),
        }
    }

    pub(crate) fn type_schema(&self, ty: &TypeRef) -> Result<Value, EmitError> {
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
                    let index = variants
                        .iter()
                        .position(|candidate| candidate.wire == *variant_name)
                        .ok_or_else(|| EmitError(format!("unknown variant {variant_name}")))?;
                    let active = (named.id.clone(), variant_name.clone());
                    if !self.selected_variants.borrow_mut().insert(active.clone()) {
                        let keyword = if matches!(repr, EnumRepr::Untagged) {
                            "anyOf"
                        } else {
                            "oneOf"
                        };
                        return Ok(variant_reference(
                            &self.schema_names[&named.id],
                            keyword,
                            index,
                        ));
                    }
                    let result = self.enum_variant_schema(repr, &variants[index]);
                    self.selected_variants.borrow_mut().remove(&active);
                    return result;
                }
                Ok(reference(&self.schema_names[&named.id]))
            }
            TypeRef::Inline(ty) => self.inline_schema(ty),
        }
    }
}

fn reference(name: &str) -> Value {
    json!({"$ref": format!("#/components/schemas/{}", name.replace('~', "~0").replace('/', "~1"))})
}

fn variant_reference(name: &str, keyword: &str, index: usize) -> Value {
    let name = name.replace('~', "~0").replace('/', "~1");
    json!({"$ref": format!("#/components/schemas/{name}/{keyword}/{index}")})
}

pub(super) fn add_doc(schema: &mut Value, doc: Option<&pons_ir::Doc>) {
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

pub(super) fn add_extensions(schema: &mut Value, key: &str, metadata: Value) {
    let extensions = schema
        .as_object_mut()
        .unwrap()
        .entry("x-pons")
        .or_insert_with(|| json!({}));
    extensions[key] = metadata;
}

#[cfg(test)]
mod tests {
    use pons_ir::{Direction, NamedRef, TypeDef, TypeKind, TypeRef};
    use serde_json::{Value, json};

    fn fixture() -> pons_ir::Document {
        serde_json::from_str(include_str!("../../../../spec/fixtures/login.json")).unwrap()
    }

    fn named(id: &str) -> TypeRef {
        TypeRef::Named(NamedRef {
            id: id.into(),
            variant: None,
        })
    }

    fn newtype(name: &str, inner: TypeRef) -> TypeDef {
        TypeDef {
            name: vec![name.into()],
            direction: Direction::Both,
            definition: TypeKind::Newtype { inner },
            origin: None,
            doc: None,
            diagnostics: Vec::new(),
            ext: Default::default(),
        }
    }

    #[test]
    fn emits_a_newtype_that_resolves_to_a_struct() {
        let mut document = fixture();
        document
            .types
            .insert("Alias".into(), newtype("alias", named("User")));
        let output: Value = serde_json::from_str(&crate::emit(&document).unwrap()).unwrap();
        assert_eq!(
            output["components"]["schemas"]["Alias"]["$ref"],
            json!("#/components/schemas/User")
        );
    }
}
