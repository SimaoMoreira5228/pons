use pons_ir::{Field, ObjectOpenness, TypeRef};
use serde_json::{Value, json};

use super::objects::fields_schema_with;
use crate::{Builder, EmitError};

impl<'a> Builder<'a> {
    pub(super) fn nullable_schema(&self, inner: &TypeRef) -> Result<Value, EmitError> {
        Ok(json!({"anyOf":[self.type_schema(inner)?,{"type":"null"}]}))
    }

    pub(super) fn array_schema(
        &self,
        item: &TypeRef,
        unique: &Option<bool>,
    ) -> Result<Value, EmitError> {
        let mut schema = json!({"type":"array","items":self.type_schema(item)?});
        if let Some(unique) = unique {
            schema["uniqueItems"] = json!(unique);
        }
        Ok(schema)
    }

    pub(super) fn map_schema(&self, key: &TypeRef, value: &TypeRef) -> Result<Value, EmitError> {
        Ok(json!({
            "type":"object",
            "propertyNames":self.type_schema(key)?,
            "additionalProperties":self.type_schema(value)?
        }))
    }

    pub(super) fn tuple_schema(&self, items: &[TypeRef]) -> Result<Value, EmitError> {
        Ok(
            json!({"type":"array","prefixItems":items.iter().map(|item| self.type_schema(item)).collect::<Result<Vec<_>,_>>()?,"minItems":items.len(),"maxItems":items.len(),"items":false}),
        )
    }

    pub(super) fn inline_struct_schema(
        &self,
        fields: &[Field],
        flattened: &[TypeRef],
        openness: &ObjectOpenness,
    ) -> Result<Value, EmitError> {
        let mut all: Vec<&Field> = fields.iter().collect();
        for member in flattened {
            let Some((inner, _)) = self.object_fields(member, &mut Vec::new())? else {
                return Err(EmitError(
                    "an inline struct member cannot be flattened because it is not a struct".into(),
                ));
            };
            all.extend(inner);
        }
        let mut object = fields_schema_with(self, all)?;
        object["additionalProperties"] = self.openness_schema(openness)?;
        Ok(object)
    }
}
