use std::fmt;

use serde_json::Value;

use crate::{Access, ByteEncoding, Direction};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IntegerProblem {
    NotAnInteger { found: String },
    NativeRange { bits: u16, signed: bool },
    BelowMinimum { minimum: String },
    AboveMaximum { maximum: String },
    NotAMultiple { multiple_of: String },
    UnreadableConstraint { constraint: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Violation {
    UnknownType {
        path: String,
        reference: String,
    },
    RecursiveType {
        path: String,
        reference: String,
    },
    Type {
        path: String,
        expected: String,
        found: String,
    },
    Integer {
        path: String,
        problem: IntegerProblem,
    },
    FloatRange {
        path: String,
        bits: u8,
    },
    StringLength {
        path: String,
        minimum: Option<u64>,
        maximum: Option<u64>,
        found: u64,
    },
    Pattern {
        path: String,
        pattern: String,
    },
    UnreadablePattern {
        path: String,
        pattern: String,
        reason: String,
    },
    Bytes {
        path: String,
        encoding: ByteEncoding,
    },
    MissingField {
        path: String,
        field: String,
    },
    UnexpectedField {
        path: String,
        field: String,
    },
    Access {
        path: String,
        field: String,
        access: Access,
        direction: Direction,
    },
    DuplicateItem {
        path: String,
        item: usize,
        first: usize,
    },
    TupleArity {
        path: String,
        expected: usize,
        found: usize,
    },
    Variant {
        path: String,
        expected: Option<String>,
        found: String,
    },
    NoVariant {
        path: String,
    },
}

impl Violation {
    pub fn path(&self) -> &str {
        match self {
            Self::UnknownType { path, .. }
            | Self::RecursiveType { path, .. }
            | Self::Type { path, .. }
            | Self::Integer { path, .. }
            | Self::FloatRange { path, .. }
            | Self::StringLength { path, .. }
            | Self::Pattern { path, .. }
            | Self::UnreadablePattern { path, .. }
            | Self::Bytes { path, .. }
            | Self::MissingField { path, .. }
            | Self::UnexpectedField { path, .. }
            | Self::Access { path, .. }
            | Self::DuplicateItem { path, .. }
            | Self::TupleArity { path, .. }
            | Self::Variant { path, .. }
            | Self::NoVariant { path } => path,
        }
    }

    pub fn reason(&self) -> String {
        match self {
            Self::UnknownType { reference, .. } => format!("unknown type {reference}"),
            Self::RecursiveType { reference, .. } => {
                format!("type {reference} cannot be resolved because newtypes form a cycle")
            }
            Self::Type {
                expected, found, ..
            } => format!("expected {expected}, found {found}"),
            Self::Integer { problem, .. } => match problem {
                IntegerProblem::NotAnInteger { found } => {
                    format!("expected an exact integer, found {found}")
                }
                IntegerProblem::NativeRange { bits, signed } => {
                    format!("integer is outside the {}{bits} range", sign_name(*signed))
                }
                IntegerProblem::BelowMinimum { minimum } => {
                    format!("integer is below its minimum of {minimum}")
                }
                IntegerProblem::AboveMaximum { maximum } => {
                    format!("integer is above its maximum of {maximum}")
                }
                IntegerProblem::NotAMultiple { multiple_of } => {
                    format!("integer is not a multiple of {multiple_of}")
                }
                IntegerProblem::UnreadableConstraint { constraint } => {
                    format!("integer constraint {constraint} is not an exact integer")
                }
            },
            Self::FloatRange { bits, .. } => format!("number is outside the f{bits} range"),
            Self::StringLength {
                minimum,
                maximum,
                found,
                ..
            } => {
                if minimum.is_some_and(|minimum| *found < minimum) {
                    format!(
                        "string is shorter than the {} code points it allows",
                        minimum.unwrap_or_default()
                    )
                } else {
                    format!(
                        "string is longer than the {} code points it allows",
                        maximum.unwrap_or_default()
                    )
                }
            }
            Self::Pattern { pattern, .. } => {
                format!("string does not match the pattern {pattern}")
            }
            Self::UnreadablePattern {
                pattern, reason, ..
            } => {
                format!("pattern {pattern} cannot be evaluated because {reason}")
            }
            Self::Bytes { encoding, .. } => match encoding {
                ByteEncoding::Hex => "invalid hexadecimal bytes".into(),
                ByteEncoding::Base64 => "invalid base64 bytes".into(),
                ByteEncoding::Base64url => "invalid base64url bytes".into(),
                ByteEncoding::Array => "invalid byte array".into(),
            },
            Self::MissingField { field, .. } => format!("missing required field {field}"),
            Self::UnexpectedField { field, .. } => format!("unknown property {field}"),
            Self::Access {
                field,
                access,
                direction,
                ..
            } => format!(
                "field {field} is {} and cannot cross the {} boundary",
                access_name(*access),
                direction_name(*direction)
            ),
            Self::DuplicateItem { item, first, .. } => {
                format!("array item {item} repeats item {first}")
            }
            Self::TupleArity {
                expected, found, ..
            } => {
                format!("expected a tuple of {expected} items, found {found}")
            }
            Self::Variant {
                expected: Some(expected),
                found,
                ..
            } => format!("expected enum variant {expected}, found {found}"),
            Self::Variant {
                expected: None,
                found,
                ..
            } => format!("unknown enum variant {found}"),
            Self::NoVariant { .. } => "value matches none of the enum variants".into(),
        }
    }
}

impl fmt::Display for Violation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.path() {
            "" => f.write_str(&self.reason()),
            path => write!(f, "{path}: {}", self.reason()),
        }
    }
}

pub(crate) fn describe(value: &Value) -> String {
    match value {
        Value::Null => "null".into(),
        Value::Bool(_) => "a boolean".into(),
        Value::Number(_) => "a number".into(),
        Value::String(_) => "a string".into(),
        Value::Array(_) => "an array".into(),
        Value::Object(_) => "an object".into(),
    }
}

fn sign_name(signed: bool) -> &'static str {
    match signed {
        true => "i",
        false => "u",
    }
}

fn access_name(access: Access) -> &'static str {
    match access {
        Access::ReadWrite => "read_write",
        Access::ReadOnly => "read_only",
        Access::WriteOnly => "write_only",
    }
}

pub(crate) fn direction_name(direction: Direction) -> &'static str {
    match direction {
        Direction::Input => "input",
        Direction::Output => "output",
        Direction::Both => "both",
    }
}
