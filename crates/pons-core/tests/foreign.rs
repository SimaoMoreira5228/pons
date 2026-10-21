#![cfg(any(
    feature = "bytes",
    feature = "chrono",
    feature = "rust_decimal",
    feature = "serde_json",
    feature = "time",
    feature = "uuid"
))]
#![allow(dead_code)]

use pons_core::{Schema, TypeCx};
use pons_ir::{Direction, TypeKind, TypeRef};

#[cfg(any(feature = "uuid", feature = "chrono", feature = "time"))]
use pons_ir::{FieldFormat, InlineType};

#[cfg(any(feature = "uuid", feature = "chrono", feature = "time"))]
fn formatted_string(format: FieldFormat) -> TypeRef {
    TypeRef::Inline(InlineType::String {
        format: Some(format),
        min_length: None,
        max_length: None,
        pattern: None,
    })
}

fn only_field_type(cx: &TypeCx, reference: &TypeRef) -> TypeRef {
    let TypeRef::Named(named) = reference else {
        panic!()
    };
    let TypeKind::Struct { fields, .. } = &cx.definitions()[&named.id].definition else {
        panic!()
    };
    assert_eq!(fields.len(), 1);
    fields[0].ty.clone()
}

#[cfg(feature = "uuid")]
#[derive(pons_core::Schema)]
struct UuidRecord {
    id: uuid::Uuid,
}

#[cfg(feature = "uuid")]
#[test]
fn uuid_maps_to_a_uuid_string() {
    let mut cx = TypeCx::new();
    let reference = UuidRecord::schema(&mut cx, Direction::Both);
    assert_eq!(
        only_field_type(&cx, &reference),
        formatted_string(FieldFormat::Uuid)
    );
}

#[cfg(feature = "chrono")]
#[derive(pons_core::Schema)]
struct ChronoDateTimeRecord {
    at: chrono::DateTime<chrono::Utc>,
}

#[cfg(feature = "chrono")]
#[test]
fn chrono_datetime_maps_to_a_date_time_string() {
    let mut cx = TypeCx::new();
    let reference = ChronoDateTimeRecord::schema(&mut cx, Direction::Both);
    assert_eq!(
        only_field_type(&cx, &reference),
        formatted_string(FieldFormat::DateTime)
    );
}

#[cfg(feature = "chrono")]
#[derive(pons_core::Schema)]
struct ChronoNaiveDateRecord {
    day: chrono::NaiveDate,
}

#[cfg(feature = "chrono")]
#[test]
fn chrono_naive_date_maps_to_a_date_string() {
    let mut cx = TypeCx::new();
    let reference = ChronoNaiveDateRecord::schema(&mut cx, Direction::Both);
    assert_eq!(
        only_field_type(&cx, &reference),
        formatted_string(FieldFormat::Date)
    );
}

#[cfg(feature = "chrono")]
#[derive(pons_core::Schema)]
struct ChronoNaiveTimeRecord {
    at: chrono::NaiveTime,
}

#[cfg(feature = "chrono")]
#[test]
fn chrono_naive_time_maps_to_a_time_string() {
    let mut cx = TypeCx::new();
    let reference = ChronoNaiveTimeRecord::schema(&mut cx, Direction::Both);
    assert_eq!(
        only_field_type(&cx, &reference),
        formatted_string(FieldFormat::Time)
    );
}

#[cfg(feature = "time")]
#[derive(pons_core::Schema)]
struct OffsetDateTimeRecord {
    at: time::OffsetDateTime,
}

#[cfg(feature = "time")]
#[test]
fn offset_date_time_maps_to_a_date_time_string() {
    let mut cx = TypeCx::new();
    let reference = OffsetDateTimeRecord::schema(&mut cx, Direction::Both);
    assert_eq!(
        only_field_type(&cx, &reference),
        formatted_string(FieldFormat::DateTime)
    );
}

#[cfg(feature = "time")]
#[derive(pons_core::Schema)]
struct PrimitiveDateTimeRecord {
    at: time::PrimitiveDateTime,
}

#[cfg(feature = "time")]
#[test]
fn primitive_date_time_maps_to_a_date_time_string() {
    let mut cx = TypeCx::new();
    let reference = PrimitiveDateTimeRecord::schema(&mut cx, Direction::Both);
    assert_eq!(
        only_field_type(&cx, &reference),
        formatted_string(FieldFormat::DateTime)
    );
}

#[cfg(feature = "time")]
#[derive(pons_core::Schema)]
struct TimeDateRecord {
    day: time::Date,
}

#[cfg(feature = "time")]
#[test]
fn time_date_maps_to_a_date_string() {
    let mut cx = TypeCx::new();
    let reference = TimeDateRecord::schema(&mut cx, Direction::Both);
    assert_eq!(
        only_field_type(&cx, &reference),
        formatted_string(FieldFormat::Date)
    );
}

#[cfg(feature = "time")]
#[derive(pons_core::Schema)]
struct TimeOfDayRecord {
    at: time::Time,
}

#[cfg(feature = "time")]
#[test]
fn time_of_day_maps_to_a_time_string() {
    let mut cx = TypeCx::new();
    let reference = TimeOfDayRecord::schema(&mut cx, Direction::Both);
    assert_eq!(
        only_field_type(&cx, &reference),
        formatted_string(FieldFormat::Time)
    );
}

#[cfg(feature = "rust_decimal")]
#[derive(pons_core::Schema)]
struct DecimalRecord {
    amount: rust_decimal::Decimal,
}

#[cfg(feature = "rust_decimal")]
#[test]
fn decimal_maps_to_a_string_wire_decimal() {
    let mut cx = TypeCx::new();
    let reference = DecimalRecord::schema(&mut cx, Direction::Both);
    assert_eq!(
        only_field_type(&cx, &reference),
        TypeRef::Inline(pons_ir::InlineType::Decimal {
            wire: pons_ir::DecimalWire::String,
        })
    );
}

#[cfg(feature = "bytes")]
#[derive(pons_core::Schema)]
struct BytesRecord {
    value: bytes::Bytes,
}

#[cfg(feature = "bytes")]
#[test]
fn bytes_maps_to_base64() {
    let mut cx = TypeCx::new();
    let reference = BytesRecord::schema(&mut cx, Direction::Both);
    assert_eq!(
        only_field_type(&cx, &reference),
        TypeRef::Inline(pons_ir::InlineType::Bytes {
            encoding: pons_ir::ByteEncoding::Base64,
        })
    );
}

#[cfg(feature = "bytes")]
#[derive(pons_core::Schema)]
struct BytesMutRecord {
    value: bytes::BytesMut,
}

#[cfg(feature = "bytes")]
#[test]
fn bytes_mut_maps_to_base64() {
    let mut cx = TypeCx::new();
    let reference = BytesMutRecord::schema(&mut cx, Direction::Both);
    assert_eq!(
        only_field_type(&cx, &reference),
        TypeRef::Inline(pons_ir::InlineType::Bytes {
            encoding: pons_ir::ByteEncoding::Base64,
        })
    );
}

#[cfg(feature = "serde_json")]
#[derive(pons_core::Schema)]
struct JsonRecord {
    value: serde_json::Value,
}

#[cfg(feature = "serde_json")]
#[test]
fn json_value_maps_to_any() {
    let mut cx = TypeCx::new();
    let reference = JsonRecord::schema(&mut cx, Direction::Both);
    assert_eq!(
        only_field_type(&cx, &reference),
        TypeRef::Inline(pons_ir::InlineType::Any)
    );
}
