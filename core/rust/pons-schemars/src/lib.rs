mod convert;
mod scalars;

use pons_core::{Schema, TypeCx};
use pons_ir::{Direction, TypeRef};
use schemars::JsonSchema;
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
        convert::lower::<T>(cx, direction)
    }
}
