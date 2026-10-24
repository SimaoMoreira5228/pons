use pons_ir::{
    Access, Document, Field, FieldDefault, InlineType, ObjectOpenness, Presence, TypeKind, TypeRef,
};
use serde_json::{Map, Value, json};

use super::{add_doc, add_extensions};
use crate::{Builder, EmitError};

impl<'a> Builder<'a> {
    pub(crate) fn field_schema(&self, field: &pons_ir::Field) -> Result<Value, EmitError> {
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

    pub(super) fn flattened_schema(
        &self,
        ty: &TypeRef,
        visiting: &mut Vec<String>,
        all_of: &mut Vec<Value>,
    ) -> Result<(), EmitError> {
        let TypeRef::Named(named) = ty else {
            return Err(not_a_struct_member(ty));
        };
        if visiting.contains(&named.id) {
            return Err(EmitError(format!(
                "the type {} cannot be inlined because flattened members form a cycle",
                named.id
            )));
        }
        let Some(definition) = self.document.types.get(&named.id) else {
            return Err(EmitError(format!("unknown type {}", named.id)));
        };
        visiting.push(named.id.clone());
        let result = match &definition.definition {
            TypeKind::Newtype { inner } => self.flattened_schema(inner, visiting, all_of),
            TypeKind::Enum { .. } => Err(not_a_struct_member(ty)),
            TypeKind::Struct {
                fields, flattened, ..
            } => {
                all_of.push(fields_schema_with(self, fields)?);
                for member in flattened {
                    self.flattened_schema(member, visiting, all_of)?;
                }
                Ok(())
            }
        };
        visiting.pop();
        result
    }

    pub(super) fn object_type(
        &self,
        ty: &TypeRef,
        visited: &mut Vec<String>,
    ) -> Result<bool, EmitError> {
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

    pub(super) fn inlined_object(&self, ty: &TypeRef) -> Result<Option<Value>, EmitError> {
        let Some((fields, openness)) = self.object_fields(ty, &mut Vec::new())? else {
            return Ok(None);
        };
        let mut object = fields_schema_with(self, fields)?;
        object["additionalProperties"] = self.openness_schema(&openness)?;
        Ok(Some(object))
    }

    pub(crate) fn object_fields<'b>(
        &'b self,
        ty: &'b TypeRef,
        visiting: &mut Vec<String>,
    ) -> Result<Option<(Vec<&'b Field>, ObjectOpenness)>, EmitError> {
        let TypeRef::Named(named) = ty else {
            return Ok(None);
        };
        if named.variant.is_some() || visiting.contains(&named.id) {
            return Ok(None);
        }
        let Some(definition) = self.document.types.get(&named.id) else {
            return Err(EmitError(format!("unknown type {}", named.id)));
        };
        visiting.push(named.id.clone());
        let result = match &definition.definition {
            TypeKind::Newtype { inner } => self.object_fields(inner, visiting),
            TypeKind::Enum { .. } => Ok(None),
            TypeKind::Struct {
                fields,
                flattened,
                openness,
            } => {
                let mut collected = fields.iter().collect::<Vec<_>>();
                for member in flattened {
                    let Some((inner, _)) = self.object_fields(member, visiting)? else {
                        return Err(not_a_struct_member(member));
                    };
                    collected.extend(inner);
                }
                Ok(Some((collected, openness.clone())))
            }
        };
        visiting.pop();
        result
    }

    pub(super) fn openness_schema(&self, openness: &ObjectOpenness) -> Result<Value, EmitError> {
        Ok(match openness {
            ObjectOpenness::Reject => json!(false),
            ObjectOpenness::PreserveTyped { value } => {
                self.type_schema(openness_value(value, self.document))?
            }
            ObjectOpenness::PreserveOpaque => json!(true),
        })
    }
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

fn openness_value<'a>(ty: &'a TypeRef, document: &'a Document) -> &'a TypeRef {
    let mut current = ty;
    let mut visited = Vec::new();
    loop {
        match current {
            TypeRef::Inline(InlineType::Map { value, .. }) => return value,
            TypeRef::Named(named) if !visited.contains(&named.id) => {
                let Some(TypeKind::Newtype { inner }) = document
                    .types
                    .get(&named.id)
                    .map(|definition| &definition.definition)
                else {
                    return current;
                };
                visited.push(named.id.clone());
                current = inner;
            }
            _ => return current,
        }
    }
}

pub(super) fn fields_schema_with<'a>(
    builder: &Builder<'_>,
    fields: impl IntoIterator<Item = &'a Field>,
) -> Result<Value, EmitError> {
    let mut properties = Map::new();
    let mut required = Vec::new();
    for field in fields {
        let mut schema = builder.field_schema(field)?;
        properties.insert(field.wire.clone(), schema.take());

        if field.presence == Presence::Required
            && !matches!(field.access, Some(Access::ReadOnly | Access::WriteOnly))
        {
            required.push(field.wire.clone());
        }
    }
    let mut object = json!({"type":"object","properties":properties});
    if !required.is_empty() {
        object["required"] = json!(required);
    }
    Ok(object)
}

#[cfg(test)]
mod tests {
    use pons_ir::{
        Access, Direction, EnumRepr, Field, InlineType, IntegerWire, NamedRef, ObjectOpenness,
        Presence, TypeDef, TypeKind, TypeRef, Variant, VariantPayload,
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

    fn integer(bits: u16, signed: bool) -> TypeRef {
        TypeRef::Inline(InlineType::Int {
            bits,
            signed,
            wire: IntegerWire::Number,
            minimum: None,
            maximum: None,
            multiple_of: None,
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

    fn enum_type(name: &str) -> TypeDef {
        TypeDef {
            name: vec![name.into()],
            direction: Direction::Both,
            definition: TypeKind::Enum {
                repr: EnumRepr::External,
                variants: vec![Variant {
                    wire: "x".into(),
                    ident: vec!["x".into()],
                    payload: VariantPayload::Unit,
                    doc: None,
                    diagnostics: Vec::new(),
                    ext: Default::default(),
                }],
            },
            origin: None,
            doc: None,
            diagnostics: Vec::new(),
            ext: Default::default(),
        }
    }

    fn map() -> TypeRef {
        TypeRef::Inline(InlineType::Map {
            key: Box::new(string()),
            value: Box::new(string()),
        })
    }

    fn directional(wire: &str, access: Access) -> Field {
        Field {
            access: Some(access),
            default: pons_ir::FieldDefault::Value(json!("x")),
            ..field(wire, string())
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
    fn flattened_members_inline_their_properties() {
        let mut document = fixture();
        document.types.insert(
            "Inner".into(),
            closed("inner", vec![field("b", integer(16, false))], Vec::new()),
        );
        document.types.insert(
            "Outer".into(),
            closed("outer", vec![field("a", string())], vec![named("Inner")]),
        );
        let output: Value = serde_json::from_str(&crate::emit(&document).unwrap()).unwrap();

        assert_eq!(
            output["components"]["schemas"]["Inner"]["additionalProperties"],
            json!(false)
        );
        let outer = &output["components"]["schemas"]["Outer"];
        assert_eq!(outer["unevaluatedProperties"], json!(false));
        assert!(outer.get("additionalProperties").is_none(), "{outer:#}");
        let all_of = outer["allOf"].as_array().unwrap();
        assert!(all_of[0].get("unevaluatedProperties").is_none());
        assert_eq!(all_of[0]["properties"]["a"]["type"], "string");
        assert_eq!(all_of[1]["properties"]["b"]["type"], "integer");
        assert_eq!(all_of[1]["required"], json!(["b"]));
        assert!(all_of[1].get("$ref").is_none(), "{:#}", all_of[1]);
        assert!(
            all_of[1].get("additionalProperties").is_none(),
            "{:#}",
            all_of[1]
        );
        assert!(
            all_of[1].get("unevaluatedProperties").is_none(),
            "{:#}",
            all_of[1]
        );
    }

    #[test]
    fn flattened_transitive_members_stay_inline() {
        let mut document = fixture();
        document.types.insert(
            "Leaf".into(),
            closed("leaf", vec![field("c", integer(16, false))], Vec::new()),
        );
        document.types.insert(
            "Inner".into(),
            closed(
                "inner",
                vec![field("b", integer(16, false))],
                vec![named("Leaf")],
            ),
        );
        document.types.insert(
            "Outer".into(),
            closed("outer", Vec::new(), vec![named("Inner")]),
        );
        let output: Value = serde_json::from_str(&crate::emit(&document).unwrap()).unwrap();
        let outer = &output["components"]["schemas"]["Outer"];
        assert_eq!(outer["unevaluatedProperties"], json!(false));
        let all_of = outer["allOf"].as_array().unwrap();
        assert_eq!(all_of[1]["properties"]["b"]["type"], "integer");
        assert_eq!(all_of[2]["properties"]["c"]["type"], "integer");
        for entry in all_of.iter().skip(1) {
            assert!(entry.get("additionalProperties").is_none(), "{entry:#}");
        }
    }

    #[test]
    fn a_flattened_struct_validates_the_payloads_the_typescript_type_accepts() {
        let mut document = fixture();
        document.types.insert(
            "Inner".into(),
            closed("inner", vec![field("b", integer(16, false))], Vec::new()),
        );
        document.types.insert(
            "Outer".into(),
            closed("outer", vec![field("a", string())], vec![named("Inner")]),
        );

        assert!(validates(&document, "Outer", json!({"a":"x","b":2})));
        assert!(!validates(&document, "Outer", json!({"a":"x","b":2,"c":3})));
        assert!(!validates(&document, "Outer", json!({"a":"x"})));
    }

    #[test]
    fn a_flattened_struct_keeps_typing_the_extra_keys_of_a_typed_openness() {
        let mut document = fixture();
        document.types.insert(
            "Inner".into(),
            closed("inner", vec![field("b", integer(16, false))], Vec::new()),
        );
        document.types.insert(
            "Outer".into(),
            struct_type(
                "outer",
                vec![field("a", string())],
                vec![named("Inner")],
                ObjectOpenness::PreserveTyped { value: string() },
            ),
        );

        assert!(validates(&document, "Outer", json!({"a":"x","b":2})));
        assert!(validates(
            &document,
            "Outer",
            json!({"a":"x","b":2,"c":"extra"})
        ));
        assert!(!validates(&document, "Outer", json!({"a":"x","b":2,"c":3})));
    }

    #[test]
    fn rejects_a_flattened_map() {
        let mut document = fixture();
        document.types.insert(
            "Outer".into(),
            closed("outer", vec![field("a", string())], vec![map()]),
        );
        assert!(crate::emit(&document).unwrap_err().0.contains(
            "is not a struct, because a flattened member contributes its fields to one object"
        ));
    }

    #[test]
    fn rejects_a_flattened_enum() {
        let mut document = fixture();
        document.types.insert("Choice".into(), enum_type("choice"));
        document.types.insert(
            "Outer".into(),
            closed("outer", vec![field("a", string())], vec![named("Choice")]),
        );
        assert!(crate::emit(&document).unwrap_err().0.contains(
            "is not a struct, because a flattened member contributes its fields to one object"
        ));
    }

    #[test]
    fn rejects_a_cyclic_flatten() {
        let mut document = fixture();
        document.types.insert(
            "Outer".into(),
            closed("outer", vec![field("a", string())], vec![named("Inner")]),
        );
        document.types.insert(
            "Inner".into(),
            closed("inner", vec![field("b", string())], vec![named("Outer")]),
        );
        assert!(
            crate::emit(&document)
                .unwrap_err()
                .0
                .contains("flattened members form a cycle")
        );
    }

    #[test]
    fn rejects_a_cyclic_flatten_through_a_newtype() {
        let mut document = fixture();
        document
            .types
            .insert("Alias".into(), newtype("alias", named("Inner")));
        document.types.insert(
            "Outer".into(),
            closed("outer", vec![field("a", string())], vec![named("Alias")]),
        );
        document.types.insert(
            "Inner".into(),
            closed("inner", vec![field("b", string())], vec![named("Alias")]),
        );
        assert!(
            crate::emit(&document)
                .unwrap_err()
                .0
                .contains("cannot be inlined because flattened members form a cycle")
        );
    }

    #[test]
    fn a_directional_field_is_never_required_of_the_payload_that_drops_it() {
        let mut document = fixture();
        document.types.insert(
            "Account".into(),
            closed(
                "account",
                vec![
                    directional("id", Access::ReadOnly),
                    directional("secret", Access::WriteOnly),
                    field("name", string()),
                ],
                Vec::new(),
            ),
        );
        document.types.insert(
            "Envelope".into(),
            closed(
                "envelope",
                vec![field("kind", string())],
                vec![named("Account")],
            ),
        );
        let output: Value = serde_json::from_str(&crate::emit(&document).unwrap()).unwrap();
        let account = &output["components"]["schemas"]["Account"];
        assert_eq!(account["required"], json!(["name"]));
        assert_eq!(account["properties"]["id"]["readOnly"], true);
        assert_eq!(account["properties"]["secret"]["writeOnly"], true);
        let all_of = output["components"]["schemas"]["Envelope"]["allOf"]
            .as_array()
            .unwrap();
        assert_eq!(all_of[1]["required"], json!(["name"]));

        for instance in [
            json!({"name":"ada"}),
            json!({"id":"7","name":"ada"}),
            json!({"secret":"s","name":"ada"}),
            json!({"id":"7","secret":"s","name":"ada"}),
        ] {
            assert!(
                validates(&document, "Account", instance.clone()),
                "the document rejects {instance}"
            );
        }
        assert!(!validates(&document, "Account", json!({"id":"7"})));
        assert!(!validates(
            &document,
            "Account",
            json!({"name":"ada","id":7})
        ));
        assert!(validates(
            &document,
            "Envelope",
            json!({"kind":"k","name":"ada"})
        ));
        assert!(!validates(
            &document,
            "Envelope",
            json!({"kind":"k","id":"7"})
        ));
    }
}
