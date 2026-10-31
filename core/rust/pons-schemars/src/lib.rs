use std::any::type_name;

use pons_core::{Schema, TypeCx};
use pons_ir::{Direction, TypeRef};
use pons_json_schema::RootType;
use schemars::JsonSchema;
use schemars::generate::{SchemaGenerator, SchemaSettings};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

#[repr(transparent)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Schemars<T>(pub T);

impl<T> Schemars<T> {
    pub fn new(inner: T) -> Self {
        Self(inner)
    }

    pub fn into_inner(self) -> T {
        self.0
    }
}

impl<T> Serialize for Schemars<T>
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

impl<'de, T> Deserialize<'de> for Schemars<T>
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

impl<T> Schema for Schemars<T>
where
    T: JsonSchema,
{
    fn schema(cx: &mut TypeCx, direction: Direction) -> TypeRef {
        let settings = match direction {
            Direction::Input => SchemaSettings::draft2020_12().for_deserialize(),
            Direction::Output => SchemaSettings::draft2020_12().for_serialize(),
            Direction::Both => SchemaSettings::draft2020_12(),
        };
        let mut generator = SchemaGenerator::new(settings);
        let schema: schemars::Schema = generator.root_schema_for::<T>();
        let base = type_name::<T>().to_owned();
        let name = T::schema_name();
        pons_json_schema::lower(
            cx,
            schema.to_value(),
            RootType {
                id: &base,
                name: &name,
                origin: Some(&base),
                direction,
            },
        )
    }
}
