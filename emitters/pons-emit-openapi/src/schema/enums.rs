use pons_ir::{EnumRepr, InlineType, Variant, VariantPayload};
use serde_json::{Map, Value, json};

use super::objects::fields_schema_with;
use super::{add_doc, add_extensions};
use crate::{Builder, EmitError};

impl<'a> Builder<'a> {
    pub(super) fn enum_schema(
        &self,
        repr: &EnumRepr,
        variants: &[Variant],
    ) -> Result<Value, EmitError> {
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

    pub(super) fn enum_variant_schema(
        &self,
        repr: &EnumRepr,
        variant: &Variant,
    ) -> Result<Value, EmitError> {
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
                    tagged(fields_schema_with(self, fields)?, tag, &variant.wire)
                }

                VariantPayload::Newtype { ty } => match self.inlined_object(ty)? {
                    Some(object) => tagged(object, tag, &variant.wire),
                    None => json!({"allOf":[tagged_schema(tag, &variant.wire),payload]}),
                },
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

fn tagged(mut object: Value, tag: &str, variant: &str) -> Value {
    object["properties"][tag] = json!({"const":variant});
    match object["required"].as_array_mut() {
        Some(required) => {
            if !required.iter().any(|name| name.as_str() == Some(tag)) {
                required.push(json!(tag));
            }
        }
        None => object["required"] = json!([tag]),
    }
    object
}

#[cfg(test)]
mod tests {
    use pons_ir::{
        Direction, EnumRepr, Field, InlineType, NamedRef, ObjectOpenness, Presence, TypeDef,
        TypeKind, TypeRef, Variant, VariantPayload,
    };
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

    fn string() -> TypeRef {
        TypeRef::Inline(InlineType::String {
            format: None,
            min_length: None,
            max_length: None,
            pattern: None,
        })
    }

    fn field(wire: &str, ty: TypeRef) -> Field {
        Field {
            wire: wire.into(),
            ident: vec![wire.into()],
            ty,
            presence: Presence::Required,
            default: Default::default(),
            access: None,
            doc: None,
            diagnostics: Vec::new(),
            ext: Default::default(),
        }
    }

    fn struct_type(
        name: &str,
        fields: Vec<Field>,
        flattened: Vec<TypeRef>,
        openness: ObjectOpenness,
    ) -> TypeDef {
        TypeDef {
            name: vec![name.into()],
            direction: Direction::Both,
            definition: TypeKind::Struct {
                fields,
                flattened,
                openness,
            },
            origin: None,
            doc: None,
            diagnostics: Vec::new(),
            ext: Default::default(),
        }
    }

    fn closed(name: &str, fields: Vec<Field>, flattened: Vec<TypeRef>) -> TypeDef {
        struct_type(name, fields, flattened, ObjectOpenness::Reject)
    }

    fn internal(name: &str, tag: &str, variants: Vec<(&str, VariantPayload)>) -> TypeDef {
        TypeDef {
            name: vec![name.into()],
            direction: Direction::Both,
            definition: TypeKind::Enum {
                repr: EnumRepr::Internal { tag: tag.into() },
                variants: variants
                    .into_iter()
                    .map(|(wire, payload)| Variant {
                        wire: wire.into(),
                        ident: vec![wire.into()],
                        payload,
                        doc: None,
                        diagnostics: Vec::new(),
                        ext: Default::default(),
                    })
                    .collect(),
            },
            origin: None,
            doc: None,
            diagnostics: Vec::new(),
            ext: Default::default(),
        }
    }

    fn component_schema(document: &pons_ir::Document, name: &str) -> Value {
        let output: Value = serde_json::from_str(&crate::emit(document).unwrap()).unwrap();
        let mut schema = json!({"$ref":format!("#/components/schemas/{name}")});
        schema["components"] = output["components"].clone();
        schema
    }

    fn validates(document: &pons_ir::Document, name: &str, instance: Value) -> bool {
        jsonschema::Validator::new(&component_schema(document, name))
            .expect("the emitted schema must be a valid JSON Schema")
            .is_valid(&instance)
    }

    #[test]
    fn an_internally_tagged_newtype_payload_composes_with_its_tag() {
        for (id, definition) in [
            (
                "Detail",
                closed("detail", vec![field("reason", string())], Vec::new()),
            ),
            (
                "Typed",
                struct_type(
                    "typed",
                    vec![field("reason", string())],
                    Vec::new(),
                    ObjectOpenness::PreserveTyped { value: string() },
                ),
            ),
            (
                "Open",
                struct_type(
                    "open",
                    vec![field("reason", string())],
                    Vec::new(),
                    ObjectOpenness::PreserveOpaque,
                ),
            ),
        ] {
            let mut document = fixture();
            document.types.insert(id.into(), definition);
            document.types.insert(
                "Notice".into(),
                internal(
                    "notice",
                    "code",
                    vec![
                        ("stop", VariantPayload::Newtype { ty: named(id) }),
                        ("quiet", VariantPayload::Unit),
                    ],
                ),
            );

            let output: Value = serde_json::from_str(&crate::emit(&document).unwrap()).unwrap();
            let branches = output["components"]["schemas"]["Notice"]["oneOf"]
                .as_array()
                .unwrap();
            let stop = branches
                .iter()
                .find(|schema| schema["properties"]["code"]["const"] == "stop")
                .unwrap();
            assert!(stop.get("allOf").is_none(), "{stop:#}");
            assert!(stop.get("$ref").is_none(), "{stop:#}");
            assert_eq!(stop["properties"]["reason"]["type"], "string");
            assert_eq!(stop["required"], json!(["reason", "code"]));

            assert!(validates(
                &document,
                "Notice",
                json!({"reason":"x","code":"stop"})
            ));
            assert!(validates(&document, "Notice", json!({"code":"quiet"})));
            assert!(
                !validates(&document, "Notice", json!({"code":"stop"})),
                "the payload fields are still required"
            );
            assert_eq!(
                validates(
                    &document,
                    "Notice",
                    json!({"reason":"x","code":"stop","extra":1})
                ),
                id == "Open",
                "the payload's own openness still judges the keys beside the tag"
            );
        }
    }

    #[test]
    fn a_tag_named_like_a_payload_field_is_refused_before_emission() {
        for presence in [Presence::Required, Presence::Optional] {
            let mut detail = field("code", string());
            detail.presence = presence;
            let mut document = fixture();
            document
                .types
                .insert("Detail".into(), closed("detail", vec![detail], Vec::new()));
            document.types.insert(
                "Notice".into(),
                internal(
                    "notice",
                    "code",
                    vec![(
                        "stop",
                        VariantPayload::Newtype {
                            ty: named("Detail"),
                        },
                    )],
                ),
            );
            let refusal = crate::emit(&document).expect_err(
                "a tag that collides with a payload field through a newtype must not emit",
            );
            assert!(refusal.0.contains("reserved enum field"), "{}", refusal.0);
        }
    }
}
