use pons_ir::{Direction, FieldFormat, InlineType, IntegerWire, TypeRef};

use super::super::backend::{
    Backend, FloatShape, IntCarrier, IntegerShape, StringCarrier, StringShape,
};
use super::Walk;
use crate::{DateMode, EmitError, EmitMode, Int64};

pub(super) struct Inline {
    pub(super) name: String,
    pub(super) expression: String,
}

impl<'a> Walk<'a> {
    pub(super) fn inline_validator<B: Backend>(
        &mut self,
        backend: &mut B,
        reference: &TypeRef,
        subject: &str,
    ) -> Result<String, EmitError> {
        let json = crate::client::type_ref_json(reference, self.options)?;
        if let Some(inline) = self.inlines.get(&json) {
            return Ok(inline.name.clone());
        }
        let name = loop {
            self.inline_count += 1;
            let candidate = format!("validateInline{}", self.inline_count);
            if self.claimed.insert(candidate.clone()) {
                break candidate;
            }
        };
        let expression = self.in_subject(subject, |walk| walk.reference(backend, reference))?;
        self.inlines.insert(
            json,
            Inline {
                name: name.clone(),
                expression,
            },
        );
        Ok(name)
    }

    pub(super) fn reference<B: Backend>(
        &mut self,
        backend: &mut B,
        reference: &TypeRef,
    ) -> Result<String, EmitError> {
        if let TypeRef::Named(named) = reference {
            let name = self
                .declaration_name(&named.id, named.variant.as_deref())?
                .to_owned();
            let rendered = backend.reference(&name, named.variant.as_deref());
            self.dependencies.push(name);
            return self.record(backend.name(), rendered);
        }
        self.inline(backend, reference)
    }

    fn inline<B: Backend>(
        &mut self,
        backend: &mut B,
        reference: &TypeRef,
    ) -> Result<String, EmitError> {
        let TypeRef::Inline(inline) = reference else {
            unreachable!("a reference that is not inline is named")
        };
        let rendered = match inline {
            InlineType::Bool => backend.boolean(),
            InlineType::Null => backend.null(),
            InlineType::Any => backend.any(),
            InlineType::Opaque { .. } => backend.unknown(),
            InlineType::Nullable { inner } => {
                let inner = self.reference(backend, inner)?;
                backend.nullable(&inner)
            }
            InlineType::Int {
                bits,
                signed,
                wire,
                minimum,
                maximum,
                multiple_of,
            } => backend.integer(&IntegerShape {
                carrier: self.int_carrier(*wire, *bits),
                bits: *bits,
                signed: *signed,
                minimum: minimum.as_ref(),
                maximum: maximum.as_ref(),
                multiple_of: multiple_of.as_ref(),
            })?,
            InlineType::Float {
                bits,
                minimum,
                maximum,
                exclusive_minimum,
                exclusive_maximum,
                multiple_of,
            } => backend.float(&FloatShape {
                bits: *bits,
                minimum: minimum.as_ref(),
                maximum: maximum.as_ref(),
                exclusive_minimum: exclusive_minimum.as_ref(),
                exclusive_maximum: exclusive_maximum.as_ref(),
                multiple_of: multiple_of.as_ref(),
            })?,
            InlineType::Decimal { wire } => backend.decimal(*wire),
            InlineType::String {
                format,
                min_length,
                max_length,
                pattern,
            } => backend.string(&StringShape {
                format: *format,
                carrier: self.string_carrier(*format),
                min_length: *min_length,
                max_length: *max_length,
                pattern: pattern.as_deref(),
            }),
            InlineType::Bytes { .. } => backend.bytes(),
            InlineType::Array { item, unique } => {
                let item = self.reference(backend, item)?;
                backend.array(&item, *unique == Some(true))
            }
            InlineType::Map { key, value } => {
                let key = self.reference(backend, key)?;
                let value = self.reference(backend, value)?;
                backend.map(&key, &value)
            }
            InlineType::Tuple { items } => {
                let items = self.items(backend, items)?;
                backend.tuple(&items)
            }
            InlineType::Struct {
                fields,
                flattened,
                openness,
            } => {
                return self.structure(backend, fields, flattened, openness, Direction::Both);
            }
        };
        self.record(backend.name(), rendered)
    }

    pub(super) fn items<B: Backend>(
        &mut self,
        backend: &mut B,
        items: &[TypeRef],
    ) -> Result<Vec<String>, EmitError> {
        items
            .iter()
            .map(|item| self.reference(backend, item))
            .collect()
    }

    fn int_carrier(&self, wire: IntegerWire, bits: u16) -> IntCarrier {
        if wire == IntegerWire::String {
            return IntCarrier::Text;
        }
        if bits > 53 {
            return match self.options.int64 {
                Int64::String => IntCarrier::Text,
                Int64::Bigint if self.options.mode == EmitMode::Strict => IntCarrier::Bigint,
                Int64::Bigint | Int64::Number => IntCarrier::Number,
            };
        }
        IntCarrier::Number
    }

    fn string_carrier(&self, format: Option<FieldFormat>) -> StringCarrier {
        if format == Some(FieldFormat::DateTime) && self.options.date_mode == DateMode::Date {
            StringCarrier::Date
        } else {
            StringCarrier::Text
        }
    }
}
