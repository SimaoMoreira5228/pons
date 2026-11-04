mod attributes;
mod convert;
mod naming;

use pons_core::{Schema, TypeCx};
use pons_ir::{Direction, TypeRef};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

#[repr(transparent)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Specta<T>(pub T);

impl<T> Specta<T> {
    pub fn new(inner: T) -> Self {
        Self(inner)
    }

    pub fn into_inner(self) -> T {
        self.0
    }
}

impl<T> Serialize for Specta<T>
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

impl<'de, T> Deserialize<'de> for Specta<T>
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

impl<T> Schema for Specta<T>
where
    T: specta::Type,
{
    fn schema(cx: &mut TypeCx, direction: Direction) -> TypeRef {
        let mut types = specta::Types::default();
        let root = T::definition(&mut types);
        convert::lower_root(cx, &types, &root, direction, std::any::type_name::<T>())
    }
}
