use std::any::type_name;

use pons_core::{Schema, TypeCx};
use pons_ir::{Direction, TypeRef};
use pons_json_schema::RootType;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use utoipa::ToSchema;

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
