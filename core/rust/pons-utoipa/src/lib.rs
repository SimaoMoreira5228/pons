use std::any::type_name;
use std::collections::BTreeMap;

use pons_core::{Schema, TypeCx};
use pons_ir::{Direction, InlineType, OpaqueWire, TypeRef};
use pons_json_schema::RootType;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{Map, Value};
use utoipa::ToSchema;
use utoipa::openapi::Components;

#[repr(transparent)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Utoipa<T>(pub T);

impl<T> Utoipa<T> {
    pub fn new(inner: T) -> Self {
        Self(inner)
    }

    pub fn into_inner(self) -> T {
        self.0
    }
}

impl<T> Serialize for Utoipa<T>
where
    T: Serialize,
{
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        self.0.serialize(serializer)
    }
}

impl<'de, T> Deserialize<'de> for Utoipa<T>
where
    T: Deserialize<'de>,
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        T::deserialize(deserializer).map(Self)
    }
}

impl<T> Schema for Utoipa<T>
where
    T: ToSchema,
{
    fn schema(cx: &mut TypeCx, direction: Direction) -> TypeRef {
        let schema =
            serde_json::to_value(T::schema()).expect("a utoipa schema serializes to a JSON value");
        let base = type_name::<T>().to_owned();
        let name = T::name();
        pons_json_schema::lower(
            cx,
            schema,
            RootType {
                id: &base,
                name: &name,
                origin: Some(&base),
                direction,
            },
        )
    }
}

pub fn schema_for<T: ToSchema>(
    components: &Components,
    cx: &mut TypeCx,
    direction: Direction,
) -> TypeRef {
    let name = T::name();
    let name = name.as_ref();
    if !components.schemas.contains_key(name) {
        cx.push_type_diagnostic(
            "PONS-W202",
            format!("the component \"{name}\" is missing from the OpenAPI document"),
        );
        return TypeRef::Inline(InlineType::Opaque {
            codec: "json-schema:ref".to_owned(),
            wire: OpaqueWire::Json,
            ext: BTreeMap::new(),
        });
    }

    let mut defs = Map::new();
    for (component, schema) in &components.schemas {
        let mut value = serde_json::to_value(schema)
            .expect("a utoipa component schema serializes to a JSON value");
        rewrite_component_refs(&mut value);
        defs.insert(component.clone(), value);
    }

    let root = Value::Object(Map::from_iter([
        ("$defs".to_owned(), Value::Object(defs)),
        ("$ref".to_owned(), Value::String(format!("#/$defs/{name}"))),
    ]));
    let base = type_name::<T>().to_owned();
    pons_json_schema::lower(
        cx,
        root,
        RootType {
            id: &base,
            name,
            origin: Some(&base),
            direction,
        },
    )
}

fn rewrite_component_refs(value: &mut Value) {
    match value {
        Value::Object(object) => {
            if let Some(Value::String(reference)) = object.get_mut("$ref")
                && let Some(component) = reference.strip_prefix("#/components/schemas/")
            {
                *reference = format!("#/$defs/{component}");
            }
            object.values_mut().for_each(rewrite_component_refs);
        }
        Value::Array(items) => items.iter_mut().for_each(rewrite_component_refs),
        _ => {}
    }
}
