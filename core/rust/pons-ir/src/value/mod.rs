use serde_json::Value;

use crate::violation::Violation;
use crate::{Direction, Document, InlineType, TypeRef};

mod composites;
mod references;
mod scalars;
mod violations;

pub fn validate_value(
    document: &Document,
    ty: &TypeRef,
    direction: Direction,
    value: &Value,
) -> Result<(), Vec<Violation>> {
    let mut validator = Validator {
        document,
        direction,
        violations: Vec::new(),
        position: Vec::new(),
    };
    validator.check(ty, "", value);
    match validator.violations.is_empty() {
        true => Ok(()),
        false => Err(validator.violations),
    }
}

struct Validator<'a> {
    document: &'a Document,
    direction: Direction,
    violations: Vec<Violation>,

    position: Vec<String>,
}

impl<'a> Validator<'a> {
    fn check(&mut self, ty: &'a TypeRef, path: &str, value: &Value) {
        match ty {
            TypeRef::Named(named) => self.named(named, path, value),
            TypeRef::Inline(inline) => self.inline(inline, path, value),
        }
    }

    fn inline(&mut self, inline: &'a InlineType, path: &str, value: &Value) {
        match inline {
            InlineType::Bool => {
                self.expect(path, "a boolean", value, Value::is_boolean);
            }
            InlineType::Null => {
                self.expect(path, "null", value, Value::is_null);
            }
            InlineType::Any | InlineType::Opaque { .. } => {}
            InlineType::Int { .. } => self.integer(inline, path, value),
            InlineType::Float { bits } => self.floating(*bits, path, value),
            InlineType::Decimal { wire } => self.decimal(*wire, path, value),
            InlineType::String {
                min_length,
                max_length,
                pattern,
                ..
            } => self.text(*min_length, *max_length, pattern.as_deref(), path, value),
            InlineType::Bytes { encoding } => self.byte_string(*encoding, path, value),
            InlineType::Nullable { inner } => {
                if !value.is_null() {
                    self.check(inner, path, value);
                }
            }
            InlineType::Array { item, unique } => self.sequence(item, *unique, path, value),
            InlineType::Map { key, value: item } => self.mapping(key, item, path, value),
            InlineType::Tuple { items } => self.tuple(items, path, value),
            InlineType::Struct {
                fields,
                flattened,
                openness,
            } => self.object(fields, flattened, openness, path, value),
        }
    }
}
