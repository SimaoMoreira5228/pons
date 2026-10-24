use pons_ir::{Direction, TypeRef};

#[cfg(feature = "rust_decimal")]
use pons_ir::DecimalWire;
#[cfg(any(feature = "uuid", feature = "chrono", feature = "time"))]
use pons_ir::FieldFormat;
#[cfg(any(
    feature = "uuid",
    feature = "chrono",
    feature = "time",
    feature = "rust_decimal",
    feature = "bytes",
    feature = "serde_json"
))]
use pons_ir::InlineType;

use crate::{Schema, TypeCx};

#[cfg(any(feature = "uuid", feature = "chrono", feature = "time"))]
fn formatted_string(format: FieldFormat) -> TypeRef {
    TypeRef::Inline(InlineType::String {
        format: Some(format),
        min_length: None,
        max_length: None,
        pattern: None,
    })
}

#[cfg(feature = "uuid")]
impl Schema for uuid::Uuid {
    fn schema(_: &mut TypeCx, _: Direction) -> TypeRef {
        formatted_string(FieldFormat::Uuid)
    }
}

#[cfg(feature = "chrono")]
impl<Tz: chrono::TimeZone> Schema for chrono::DateTime<Tz> {
    fn schema(_: &mut TypeCx, _: Direction) -> TypeRef {
        formatted_string(FieldFormat::DateTime)
    }
}

#[cfg(feature = "chrono")]
impl Schema for chrono::NaiveDate {
    fn schema(_: &mut TypeCx, _: Direction) -> TypeRef {
        formatted_string(FieldFormat::Date)
    }
}

#[cfg(feature = "chrono")]
impl Schema for chrono::NaiveTime {
    fn schema(_: &mut TypeCx, _: Direction) -> TypeRef {
        formatted_string(FieldFormat::Time)
    }
}

#[cfg(feature = "time")]
impl Schema for time::OffsetDateTime {
    fn schema(_: &mut TypeCx, _: Direction) -> TypeRef {
        formatted_string(FieldFormat::DateTime)
    }
}

#[cfg(feature = "time")]
impl Schema for time::PrimitiveDateTime {
    fn schema(_: &mut TypeCx, _: Direction) -> TypeRef {
        formatted_string(FieldFormat::DateTime)
    }
}

#[cfg(feature = "time")]
impl Schema for time::Date {
    fn schema(_: &mut TypeCx, _: Direction) -> TypeRef {
        formatted_string(FieldFormat::Date)
    }
}

#[cfg(feature = "time")]
impl Schema for time::Time {
    fn schema(_: &mut TypeCx, _: Direction) -> TypeRef {
        formatted_string(FieldFormat::Time)
    }
}

#[cfg(feature = "rust_decimal")]
impl Schema for rust_decimal::Decimal {
    fn schema(_: &mut TypeCx, _: Direction) -> TypeRef {
        TypeRef::Inline(InlineType::Decimal {
            wire: DecimalWire::String,
        })
    }
}

#[cfg(feature = "bytes")]
impl Schema for bytes::Bytes {
    fn schema(_: &mut TypeCx, _: Direction) -> TypeRef {
        TypeRef::Inline(InlineType::Bytes {
            encoding: pons_ir::ByteEncoding::Base64,
        })
    }
}

#[cfg(feature = "bytes")]
impl Schema for bytes::BytesMut {
    fn schema(_: &mut TypeCx, _: Direction) -> TypeRef {
        TypeRef::Inline(InlineType::Bytes {
            encoding: pons_ir::ByteEncoding::Base64,
        })
    }
}

#[cfg(feature = "serde_json")]
impl Schema for serde_json::Value {
    fn schema(_: &mut TypeCx, _: Direction) -> TypeRef {
        TypeRef::Inline(InlineType::Any)
    }
}
