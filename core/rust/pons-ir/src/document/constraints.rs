use serde_json::Value;

use super::{Exact, exact_integer};

pub(super) fn positive_number(value: &Value) -> bool {
    match exact_integer(value) {
        Exact::Integer((negative, magnitude)) => !negative && magnitude != 0,
        Exact::Beyond => true,
        Exact::Fraction => value.as_f64().is_some_and(|number| number > 0.0),
    }
}

pub(super) fn integer_minimum_exceeds_maximum(minimum: &Value, maximum: &Value) -> bool {
    if let (
        Exact::Integer((minimum_negative, minimum)),
        Exact::Integer((maximum_negative, maximum)),
    ) = (exact_integer(minimum), exact_integer(maximum))
    {
        return match (minimum_negative, maximum_negative) {
            (false, true) => true,
            (true, false) => false,
            (false, false) => minimum > maximum,
            (true, true) => minimum < maximum,
        };
    }
    minimum
        .as_f64()
        .zip(maximum.as_f64())
        .is_some_and(|(minimum, maximum)| minimum > maximum)
}

fn integer_native_max(bits: u16) -> u128 {
    if bits == 128 {
        u128::MAX
    } else {
        (1_u128 << u32::from(bits)) - 1
    }
}

pub(super) fn integer_below_native(value: &Value, bits: u16, signed: bool) -> bool {
    if !matches!(bits, 8 | 16 | 32 | 64 | 128) {
        return false;
    }
    match exact_integer(value) {
        Exact::Integer((negative, magnitude)) => {
            if signed {
                negative && magnitude > (1_u128 << (u32::from(bits) - 1))
            } else {
                negative || magnitude > integer_native_max(bits)
            }
        }
        Exact::Beyond => negative_literal(value),
        Exact::Fraction => false,
    }
}

pub(super) fn integer_above_native(value: &Value, bits: u16, signed: bool) -> bool {
    if !matches!(bits, 8 | 16 | 32 | 64 | 128) {
        return false;
    }
    match exact_integer(value) {
        Exact::Integer((negative, magnitude)) => {
            if signed {
                !negative && magnitude > ((1_u128 << (u32::from(bits) - 1)) - 1)
            } else {
                !negative && magnitude > integer_native_max(bits)
            }
        }
        Exact::Beyond => !negative_literal(value),
        Exact::Fraction => false,
    }
}

fn negative_literal(value: &Value) -> bool {
    value
        .as_number()
        .is_some_and(|number| number.to_string().starts_with('-'))
}
