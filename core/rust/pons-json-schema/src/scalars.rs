use pons_core::TypeCx;
use pons_ir::{ByteEncoding, FieldFormat, InlineType, IntegerWire, TypeRef};
use serde_json::{Map, Value};

pub(super) const DROPPED_KEYWORD: &str = "PONS-W203";
const UNSTATED_INTEGER: &str = "PONS-W201";

pub(super) fn lower_string(cx: &mut TypeCx, object: &Map<String, Value>) -> TypeRef {
    if let Some(encoding) = object.get("contentEncoding").and_then(Value::as_str) {
        match encoding {
            "base64" => return bytes(ByteEncoding::Base64),
            "base64url" => return bytes(ByteEncoding::Base64url),
            "hex" => return bytes(ByteEncoding::Hex),
            other => cx.push_type_diagnostic(
                DROPPED_KEYWORD,
                format!(
                    "contentEncoding \"{other}\" has no Pons byte encoding, so the value is a plain string"
                ),
            ),
        }
    }
    let format = object.get("format").and_then(Value::as_str);
    if format == Some("byte") {
        return bytes(ByteEncoding::Base64);
    }
    TypeRef::Inline(InlineType::String {
        format: format.and_then(|format| field_format(cx, format)),
        min_length: object.get("minLength").and_then(Value::as_u64),
        max_length: object.get("maxLength").and_then(Value::as_u64),
        pattern: object
            .get("pattern")
            .and_then(Value::as_str)
            .map(str::to_owned),
    })
}

fn field_format(cx: &mut TypeCx, format: &str) -> Option<FieldFormat> {
    let mapped = match format {
        "uuid" => FieldFormat::Uuid,
        "date-time" => FieldFormat::DateTime,
        "date" => FieldFormat::Date,
        "time" => FieldFormat::Time,
        "duration" => FieldFormat::Duration,
        "uri" => FieldFormat::Uri,
        "email" => FieldFormat::Email,
        "ipv4" => FieldFormat::Ipv4,
        "ipv6" => FieldFormat::Ipv6,
        other => {
            cx.push_type_diagnostic(
                DROPPED_KEYWORD,
                format!("string format \"{other}\" has no Pons format and is dropped"),
            );
            return None;
        }
    };
    Some(mapped)
}

pub(super) fn lower_integer(cx: &mut TypeCx, object: &Map<String, Value>) -> TypeRef {
    let (bits, signed) = match object.get("format").and_then(Value::as_str) {
        Some("int8") => (8, true),
        Some("uint8") => (8, false),
        Some("int16") => (16, true),
        Some("uint16") => (16, false),
        Some("int32") => (32, true),
        Some("uint32") => (32, false),
        Some("int64") => (64, true),
        Some("uint64") => (64, false),
        Some("int128") => (128, true),
        Some("uint128") => (128, false),
        Some("int") => (64, true),
        Some("uint") => (64, false),
        Some(other) => {
            cx.push_type_diagnostic(
                UNSTATED_INTEGER,
                format!(
                    "integer format \"{other}\" does not state a width and signedness; the value lowers to a 64-bit unsigned number"
                ),
            );
            (64, false)
        }
        None => {
            cx.push_type_diagnostic(
                UNSTATED_INTEGER,
                "the schema states \"integer\" without a format, so the source does not state the width or signedness; the value lowers to a 64-bit unsigned number",
            );
            (64, false)
        }
    };
    TypeRef::Inline(InlineType::Int {
        bits,
        signed,
        wire: IntegerWire::Number,
        minimum: object.get("minimum").cloned(),
        maximum: object.get("maximum").cloned(),
        multiple_of: object.get("multipleOf").cloned(),
    })
}

pub(super) fn lower_number(object: &Map<String, Value>) -> TypeRef {
    let bits = match object.get("format").and_then(Value::as_str) {
        Some("float") => 32,
        _ => 64,
    };
    TypeRef::Inline(InlineType::Float {
        bits,
        minimum: object.get("minimum").cloned(),
        maximum: object.get("maximum").cloned(),
        exclusive_minimum: object.get("exclusiveMinimum").cloned(),
        exclusive_maximum: object.get("exclusiveMaximum").cloned(),
        multiple_of: object.get("multipleOf").cloned(),
    })
}

fn bytes(encoding: ByteEncoding) -> TypeRef {
    TypeRef::Inline(InlineType::Bytes { encoding })
}

pub(super) fn string_type() -> TypeRef {
    TypeRef::Inline(InlineType::String {
        format: None,
        min_length: None,
        max_length: None,
        pattern: None,
    })
}
