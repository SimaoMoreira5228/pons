use std::cmp::Ordering;

use serde_json::Value;

use super::Validator;
use super::violations::child;
use crate::document::{Exact, exact_integer};
use crate::violation::{FloatProblem, IntegerProblem, Violation};
use crate::{ByteEncoding, DecimalWire, InlineType, IntegerWire};

impl<'a> Validator<'a> {
    pub(super) fn integer(&mut self, ty: &'a InlineType, path: &str, value: &Value) {
        let InlineType::Int {
            bits,
            signed,
            wire,
            minimum,
            maximum,
            multiple_of,
        } = ty
        else {
            return;
        };
        let (bits, signed) = (*bits, *signed);
        let Some(integer) = self.wire_integer(*wire, bits, signed, path, value) else {
            return;
        };
        let problem = if below_native(integer, bits, signed) || above_native(integer, bits, signed)
        {
            IntegerProblem::NativeRange { bits, signed }
        } else if let Some(minimum) = minimum
            .as_ref()
            .filter(|bound| bounded(integer, bound).is_lt())
        {
            IntegerProblem::BelowMinimum {
                minimum: minimum.to_string(),
            }
        } else if let Some(maximum) = maximum
            .as_ref()
            .filter(|bound| bounded(integer, bound).is_gt())
        {
            IntegerProblem::AboveMaximum {
                maximum: maximum.to_string(),
            }
        } else {
            match divisible(integer, multiple_of.as_ref()) {
                Some(problem) => problem,
                None => return,
            }
        };
        self.reject(Violation::Integer {
            path: path.to_owned(),
            problem,
        });
    }

    fn wire_integer(
        &mut self,
        wire: IntegerWire,
        bits: u16,
        signed: bool,
        path: &str,
        value: &Value,
    ) -> Option<(bool, u128)> {
        match wire {
            IntegerWire::Number => {
                if !self.expect(path, "a number", value, Value::is_number) {
                    return None;
                }
                match exact_integer(value) {
                    Exact::Integer(integer) => Some(integer),
                    Exact::Fraction => {
                        self.reject(Violation::Integer {
                            path: path.to_owned(),
                            problem: IntegerProblem::NotAnInteger {
                                found: value.to_string(),
                            },
                        });
                        None
                    }
                    Exact::Beyond => {
                        self.reject(Violation::Integer {
                            path: path.to_owned(),
                            problem: IntegerProblem::NativeRange { bits, signed },
                        });
                        None
                    }
                }
            }
            IntegerWire::String => {
                let Some(text) = value.as_str() else {
                    self.type_violation(path, "an integer string", value);
                    return None;
                };
                if !canonical_integer(text, signed) {
                    self.reject(Violation::Integer {
                        path: path.to_owned(),
                        problem: IntegerProblem::NotAnInteger {
                            found: text.to_owned(),
                        },
                    });
                    return None;
                }
                let (negative, digits) = match text.strip_prefix('-') {
                    Some(digits) => (true, digits),
                    None => (false, text),
                };
                match digits.parse::<u128>() {
                    Ok(magnitude) => Some((negative, magnitude)),
                    Err(_) => {
                        self.reject(Violation::Integer {
                            path: path.to_owned(),
                            problem: IntegerProblem::NativeRange { bits, signed },
                        });
                        None
                    }
                }
            }
        }
    }

    pub(super) fn floating(&mut self, ty: &'a InlineType, path: &str, value: &Value) {
        let InlineType::Float {
            bits,
            minimum,
            maximum,
            exclusive_minimum,
            exclusive_maximum,
            multiple_of,
        } = ty
        else {
            return;
        };
        self.number(path, value);
        let Some(number) = value.as_f64().filter(|number| number.is_finite()) else {
            return;
        };
        let problem = (*bits == 32 && number.abs() > f64::from(f32::MAX))
            .then_some(FloatProblem::NativeRange { bits: *bits })
            .or_else(|| minimum_problem(number, minimum.as_ref()))
            .or_else(|| maximum_problem(number, maximum.as_ref()))
            .or_else(|| exclusive_minimum_problem(number, exclusive_minimum.as_ref()))
            .or_else(|| exclusive_maximum_problem(number, exclusive_maximum.as_ref()));
        let problem = match problem {
            Some(problem) => problem,
            None => match multiple_problem(number, multiple_of.as_ref()) {
                Some(problem) => problem,
                None => return,
            },
        };
        self.reject(Violation::Float {
            path: path.to_owned(),
            problem,
        });
    }

    pub(super) fn decimal(&mut self, wire: DecimalWire, path: &str, value: &Value) {
        match wire {
            DecimalWire::Number => self.number(path, value),
            DecimalWire::String => {
                let Some(text) = value.as_str() else {
                    return self.type_violation(path, "a decimal string", value);
                };
                if !is_decimal(text) {
                    self.content_violation(path, "a decimal string", value);
                }
            }
        }
    }

    fn number(&mut self, path: &str, value: &Value) {
        if !self.expect(path, "a number", value, Value::is_number) {
            return;
        }
        if !value.as_f64().unwrap_or_default().is_finite() {
            self.type_violation(path, "a finite number", value);
        }
    }

    pub(super) fn text(
        &mut self,
        minimum: Option<u64>,
        maximum: Option<u64>,
        pattern: Option<&str>,
        path: &str,
        value: &Value,
    ) {
        let Some(text) = value.as_str() else {
            return self.type_violation(path, "a string", value);
        };
        let found = text.chars().count() as u64;
        if minimum.is_some_and(|minimum| found < minimum)
            || maximum.is_some_and(|maximum| found > maximum)
        {
            return self.reject(Violation::StringLength {
                path: path.to_owned(),
                minimum,
                maximum,
                found,
            });
        }
        let Some(pattern) = pattern else { return };
        match crate::pattern::matches(pattern, text) {
            Ok(true) => {}
            Ok(false) => self.reject(Violation::Pattern {
                path: path.to_owned(),
                pattern: pattern.to_owned(),
            }),
            Err(reason) => self.reject(Violation::UnreadablePattern {
                path: path.to_owned(),
                pattern: pattern.to_owned(),
                reason,
            }),
        }
    }

    pub(super) fn byte_string(&mut self, encoding: ByteEncoding, path: &str, value: &Value) {
        if encoding == ByteEncoding::Array {
            let Some(items) = value.as_array() else {
                return self.type_violation(path, "an array of bytes", value);
            };
            for (index, item) in items.iter().enumerate() {
                let path = child(path, &index.to_string());
                match exact_integer(item) {
                    Exact::Integer((false, byte)) if byte <= 255 => {}
                    Exact::Fraction => self.type_violation(&path, "a byte between 0 and 255", item),
                    _ => self.content_violation(&path, "a byte between 0 and 255", item),
                }
            }
            return;
        }
        let Some(text) = value.as_str() else {
            return self.type_violation(path, "encoded bytes", value);
        };
        let shaped = match encoding {
            ByteEncoding::Hex => {
                text.len() % 2 == 0 && text.bytes().all(|byte| byte.is_ascii_hexdigit())
            }
            ByteEncoding::Base64 => is_base64(text, false),
            ByteEncoding::Base64url => is_base64(text, true),
            ByteEncoding::Array => unreachable!("array bytes are handled above"),
        };
        if !shaped {
            self.reject(Violation::Bytes {
                path: path.to_owned(),
                encoding,
            });
        }
    }
}

fn native_bounds(bits: u16, signed: bool) -> Option<(u128, u128)> {
    if !matches!(bits, 8 | 16 | 32 | 64 | 128) {
        return None;
    }
    let ceiling = 1_u128 << (u32::from(bits) - 1);
    Some(match signed {
        true => (ceiling, ceiling - 1),
        false if bits == 128 => (0, u128::MAX),
        false => (0, ceiling * 2 - 1),
    })
}

fn below_native(integer: (bool, u128), bits: u16, signed: bool) -> bool {
    let (negative, magnitude) = integer;
    let Some((lowest, _)) = native_bounds(bits, signed) else {
        return false;
    };
    match (negative, signed) {
        (false, _) => false,
        (true, true) => magnitude > lowest,
        (true, false) => true,
    }
}

fn above_native(integer: (bool, u128), bits: u16, signed: bool) -> bool {
    let (negative, magnitude) = integer;
    let Some((_, highest)) = native_bounds(bits, signed) else {
        return false;
    };
    !negative && magnitude > highest
}

fn compare(left: (bool, u128), right: (bool, u128)) -> Ordering {
    match (left.0, right.0) {
        (false, true) => Ordering::Greater,
        (true, false) => Ordering::Less,
        (false, false) => left.1.cmp(&right.1),
        (true, true) => right.1.cmp(&left.1),
    }
}

fn bounded(integer: (bool, u128), bound: &Value) -> Ordering {
    compare(
        integer,
        match exact_integer(bound) {
            Exact::Integer(bound) => bound,
            _ => (false, u128::MAX),
        },
    )
}

fn divisible(integer: (bool, u128), multiple_of: Option<&Value>) -> Option<IntegerProblem> {
    let step = multiple_of?;
    let Exact::Integer((negative, divisor)) = exact_integer(step) else {
        return Some(IntegerProblem::UnreadableConstraint {
            constraint: step.to_string(),
        });
    };
    if negative || divisor == 0 {
        return Some(IntegerProblem::UnreadableConstraint {
            constraint: step.to_string(),
        });
    }
    (!integer.1.is_multiple_of(divisor)).then(|| IntegerProblem::NotAMultiple {
        multiple_of: step.to_string(),
    })
}

fn positive_float_bound(value: &Value) -> Option<f64> {
    value
        .as_f64()
        .filter(|number| number.is_finite() && *number > 0.0)
}

fn readable_float_bound(value: &Value) -> Option<f64> {
    value.as_f64().filter(|number| number.is_finite())
}

fn minimum_problem(number: f64, minimum: Option<&Value>) -> Option<FloatProblem> {
    let minimum = minimum?;
    match readable_float_bound(minimum) {
        None => Some(FloatProblem::UnreadableConstraint {
            constraint: minimum.to_string(),
        }),
        Some(bound) if number < bound => Some(FloatProblem::BelowMinimum {
            minimum: minimum.to_string(),
        }),
        Some(_) => None,
    }
}

fn maximum_problem(number: f64, maximum: Option<&Value>) -> Option<FloatProblem> {
    let maximum = maximum?;
    match readable_float_bound(maximum) {
        None => Some(FloatProblem::UnreadableConstraint {
            constraint: maximum.to_string(),
        }),
        Some(bound) if number > bound => Some(FloatProblem::AboveMaximum {
            maximum: maximum.to_string(),
        }),
        Some(_) => None,
    }
}

fn exclusive_minimum_problem(number: f64, minimum: Option<&Value>) -> Option<FloatProblem> {
    let minimum = minimum?;
    match readable_float_bound(minimum) {
        None => Some(FloatProblem::UnreadableConstraint {
            constraint: minimum.to_string(),
        }),
        Some(bound) if number <= bound => Some(FloatProblem::NotAboveExclusiveMinimum {
            minimum: minimum.to_string(),
        }),
        Some(_) => None,
    }
}

fn exclusive_maximum_problem(number: f64, maximum: Option<&Value>) -> Option<FloatProblem> {
    let maximum = maximum?;
    match readable_float_bound(maximum) {
        None => Some(FloatProblem::UnreadableConstraint {
            constraint: maximum.to_string(),
        }),
        Some(bound) if number >= bound => Some(FloatProblem::NotBelowExclusiveMaximum {
            maximum: maximum.to_string(),
        }),
        Some(_) => None,
    }
}

fn multiple_problem(number: f64, multiple_of: Option<&Value>) -> Option<FloatProblem> {
    let step = multiple_of?;
    let Some(divisor) = positive_float_bound(step) else {
        return Some(FloatProblem::UnreadableConstraint {
            constraint: step.to_string(),
        });
    };
    let quotient = number / divisor;
    let tolerance = f64::EPSILON * 8.0 * quotient.abs().max(1.0);
    ((quotient - quotient.round()).abs() > tolerance).then(|| FloatProblem::NotAMultiple {
        multiple_of: step.to_string(),
    })
}

fn canonical_integer(text: &str, signed: bool) -> bool {
    let digits = signed
        .then_some(())
        .map_or(text, |()| text.strip_prefix('-').unwrap_or(text));
    !digits.is_empty()
        && (digits == "0" || digits.starts_with(|digit: char| ('1'..='9').contains(&digit)))
        && digits.bytes().all(|digit| digit.is_ascii_digit())
}

fn is_decimal(text: &str) -> bool {
    let (mantissa, exponent) = match text.split_once(['e', 'E']) {
        Some((mantissa, exponent)) => (mantissa, Some(exponent)),
        None => (text, None),
    };
    if let Some(exponent) = exponent {
        let sign = exponent.strip_prefix(['+', '-']).unwrap_or(exponent);
        if sign.is_empty() || !sign.bytes().all(|digit| digit.is_ascii_digit()) {
            return false;
        }
    }
    let mantissa = mantissa.strip_prefix('-').unwrap_or(mantissa);
    let (whole, fraction) = mantissa.split_once('.').unwrap_or((mantissa, ""));
    if fraction.contains('.') {
        return false;
    }
    let digits = |part: &str| part.bytes().all(|digit| digit.is_ascii_digit());
    (digits(whole) || whole.is_empty())
        && digits(fraction)
        && !(whole.is_empty() && fraction.is_empty())
}

fn is_base64(text: &str, url: bool) -> bool {
    let alphabet = |byte: u8| match byte {
        b'-' | b'_' => url,
        b'+' | b'/' => !url,
        _ => byte.is_ascii_alphanumeric(),
    };
    let bytes = text.as_bytes();
    let (body, padding) = match bytes {
        [rest @ .., b'=', b'='] => (rest, 2),
        [rest @ .., b'='] => (rest, 1),
        rest => (rest, 0),
    };
    let mut index = 0;
    while index + 4 <= body.len() && body[index..index + 4].iter().all(|byte| alphabet(*byte)) {
        index += 4;
    }
    let tail = &body[index..];
    let shaped = match (tail.len(), padding) {
        (0, 0) => true,
        (2, 2) | (3, 1) => true,
        (2 | 3, 0) => url,
        _ => false,
    };
    shaped && tail.iter().all(|byte| alphabet(*byte))
}
