use std::collections::BTreeSet;

use pons_ir::{
    Access, ByteEncoding, DecimalWire, Direction, Document, EnumRepr, Field, FieldDefault,
    FieldFormat, InlineType, IntegerWire, NamedRef, Presence, TypeKind, TypeRef, Variant,
    VariantPayload, validate_value,
};
use serde_json::{Map, Value};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Gap {
    Recursive(String),
    Unsupported(String),
}

impl Gap {
    pub(crate) fn reason(&self) -> String {
        match self {
            Self::Recursive(id) => format!("the type {id} is recursive"),
            Self::Unsupported(message) => message.clone(),
        }
    }
}

pub(crate) struct Examples<'a> {
    document: &'a Document,
    stack: Vec<String>,
    counter: u64,
}

impl<'a> Examples<'a> {
    pub(crate) fn new(document: &'a Document) -> Self {
        Self {
            document,
            stack: Vec::new(),
            counter: 0,
        }
    }

    pub(crate) fn value_in(&mut self, ty: &TypeRef, direction: Direction) -> Result<Value, Gap> {
        match ty {
            TypeRef::Named(named) => self.named(named, direction),
            TypeRef::Inline(inline) => self.inline(inline, direction),
        }
    }

    fn next(&mut self) -> u64 {
        let counter = self.counter;
        self.counter += 1;
        counter
    }

    fn named(&mut self, named: &NamedRef, direction: Direction) -> Result<Value, Gap> {
        if self.stack.contains(&named.id) {
            return Err(Gap::Recursive(named.id.clone()));
        }
        let Some(definition) = self.document.types.get(&named.id) else {
            return Err(Gap::Unsupported(format!("unknown type {}", named.id)));
        };
        self.stack.push(named.id.clone());
        let value = match &definition.definition {
            TypeKind::Newtype { inner } => self.value_in(inner, direction),
            TypeKind::Struct {
                fields, flattened, ..
            } => self.object(fields, flattened, direction).map(Value::Object),
            TypeKind::Enum { repr, variants } => {
                self.enumeration(repr, variants, named.variant.as_deref(), direction)
            }
        };
        self.stack.pop();
        value
    }

    fn inline(&mut self, inline: &InlineType, direction: Direction) -> Result<Value, Gap> {
        match inline {
            InlineType::Bool => Ok(Value::Bool(true)),
            InlineType::Int {
                bits,
                signed,
                wire,
                minimum,
                maximum,
                multiple_of,
            } => self.integer(*bits, *signed, *wire, minimum, maximum, multiple_of),
            InlineType::Float { .. } => {
                Ok(serde_json::Number::from_f64(1.5).map_or(Value::Null, Value::Number))
            }
            InlineType::Decimal { wire } => Ok(match wire {
                DecimalWire::String => Value::String("1.5".into()),
                DecimalWire::Number => Value::from(1.5),
            }),
            InlineType::String {
                format,
                min_length,
                max_length,
                pattern,
            } => self.string(*format, *min_length, *max_length, pattern.as_deref()),
            InlineType::Bytes { encoding } => Ok(match encoding {
                ByteEncoding::Array => Value::Array(vec![Value::from(0)]),
                ByteEncoding::Hex => Value::String("00".into()),
                ByteEncoding::Base64 => Value::String("AA==".into()),
                ByteEncoding::Base64url => Value::String("AA".into()),
            }),
            InlineType::Null => Ok(Value::Null),
            InlineType::Any => Ok(Value::Null),
            InlineType::Opaque { wire, .. } => Ok(match wire {
                pons_ir::OpaqueWire::Text => Value::String("pons".into()),
                pons_ir::OpaqueWire::Bytes => Value::String(String::new()),
                pons_ir::OpaqueWire::Json | pons_ir::OpaqueWire::Unknown => Value::Null,
            }),
            InlineType::Nullable { inner } => self.value_in(inner, direction),
            InlineType::Array { item, .. } => {
                Ok(Value::Array(vec![self.value_in(item, direction)?]))
            }
            InlineType::Map { key, value } => self.map(key, value, direction),
            InlineType::Tuple { items } => items
                .iter()
                .map(|item| self.value_in(item, direction))
                .collect::<Result<Vec<_>, _>>()
                .map(Value::Array),
            InlineType::Struct {
                fields, flattened, ..
            } => self.object(fields, flattened, direction).map(Value::Object),
        }
    }

    fn integer(
        &mut self,
        bits: u16,
        signed: bool,
        wire: IntegerWire,
        minimum: &Option<Value>,
        maximum: &Option<Value>,
        multiple_of: &Option<Value>,
    ) -> Result<Value, Gap> {
        let text = match signed {
            true => {
                let lowest = lower_bound(bits);
                let highest = upper_bound(bits);
                let minimum = minimum
                    .as_ref()
                    .and_then(integer_bound)
                    .unwrap_or(lowest)
                    .max(lowest);
                let maximum = maximum
                    .as_ref()
                    .and_then(integer_bound)
                    .unwrap_or(highest)
                    .min(highest);
                if minimum > maximum {
                    return Err(Gap::Unsupported(format!(
                        "no {}{bits} integer satisfies its bounds",
                        sign(signed)
                    )));
                }
                let mut value = if maximum < 0 {
                    maximum
                } else if minimum > 0 {
                    minimum
                } else {
                    0
                };
                if let Some(step) = multiple_of.as_ref().and_then(integer_bound) {
                    if step <= 0 {
                        return Err(Gap::Unsupported(
                            "integer multiple_of is not positive".into(),
                        ));
                    }
                    let remainder = value.rem_euclid(step);
                    if remainder != 0 {
                        value = value.saturating_add(step - remainder);
                    }
                }
                if value > maximum {
                    return Err(Gap::Unsupported(format!(
                        "no {}{bits} integer satisfies its bounds",
                        sign(signed)
                    )));
                }
                value.to_string()
            }
            false => {
                let highest = unsigned_upper_bound(bits);
                let minimum = minimum
                    .as_ref()
                    .and_then(unsigned_bound)
                    .unwrap_or(0)
                    .min(highest);
                let maximum = maximum
                    .as_ref()
                    .and_then(unsigned_bound)
                    .unwrap_or(highest)
                    .min(highest);
                let mut value = minimum;
                if let Some(step) = multiple_of.as_ref().and_then(unsigned_bound) {
                    if step == 0 {
                        return Err(Gap::Unsupported("integer multiple_of is zero".into()));
                    }
                    let remainder = value % step;
                    if remainder != 0 {
                        value = value.checked_add(step - remainder).unwrap_or(highest + 1);
                    }
                }
                if value > maximum {
                    return Err(Gap::Unsupported(format!(
                        "no {}{bits} integer satisfies its bounds",
                        sign(signed)
                    )));
                }
                value.to_string()
            }
        };
        match wire {
            IntegerWire::String => Ok(Value::String(text)),
            IntegerWire::Number => serde_json::from_str(&text)
                .map_err(|_| Gap::Unsupported(format!("cannot write the integer {text}"))),
        }
    }

    fn string(
        &mut self,
        format: Option<FieldFormat>,
        min_length: Option<u64>,
        max_length: Option<u64>,
        pattern: Option<&str>,
    ) -> Result<Value, Gap> {
        if min_length.is_some_and(|minimum| minimum > 4096)
            || max_length.is_some_and(|maximum| maximum > 4096)
        {
            return Err(Gap::Unsupported(
                "the declared string length is too large to generate".into(),
            ));
        }
        let counter = self.next();
        let base = match format {
            None => format!("pons{counter}"),
            Some(FieldFormat::Uuid) => format!("00000000-0000-4000-8000-{counter:012x}"),
            Some(FieldFormat::DateTime) => "2026-01-01T00:00:00Z".into(),
            Some(FieldFormat::Date) => "2026-01-01".into(),
            Some(FieldFormat::Time) => "00:00:00".into(),
            Some(FieldFormat::Duration) => format!("PT{}S", counter + 1),
            Some(FieldFormat::Uri) => format!("https://example.test/{counter}"),
            Some(FieldFormat::Email) => format!("user{counter}@example.test"),
            Some(FieldFormat::Ipv4) => "192.0.2.1".into(),
            Some(FieldFormat::Ipv6) => "2001:db8::1".into(),
        };
        let mut candidates = Vec::new();
        if let Some(pattern) = pattern
            && let Some(sample) = crate::pattern::sample(pattern)
        {
            candidates.push(sample);
        }
        candidates.push(base.clone());
        candidates.push(format!("pons{counter}"));
        candidates.push(counter.to_string());
        if let Some(maximum) = max_length {
            candidates.push("a".repeat(usize::try_from(maximum).unwrap_or(0)));
        }

        let mut seen = BTreeSet::new();
        for candidate in candidates {
            if !seen.insert(candidate.clone()) {
                continue;
            }
            let Some(candidate) = fit(candidate, format, min_length, max_length) else {
                continue;
            };
            let check = TypeRef::Inline(InlineType::String {
                format: None,
                min_length,
                max_length,
                pattern: pattern.map(str::to_owned),
            });
            if validate_value(
                self.document,
                &check,
                Direction::Input,
                &Value::String(candidate.clone()),
            )
            .is_ok()
            {
                return Ok(Value::String(candidate));
            }
        }
        Err(Gap::Unsupported(match pattern {
            Some(pattern) => format!("no string satisfies {pattern:?}"),
            None => "no string satisfies the declared length".into(),
        }))
    }

    fn map(&mut self, key: &TypeRef, value: &TypeRef, direction: Direction) -> Result<Value, Gap> {
        let key = self.value_in(key, Direction::Input)?;
        let Value::String(key) = key else {
            return Ok(Value::Object(Map::new()));
        };
        let mut object = Map::new();
        object.insert(key, self.value_in(value, direction)?);
        Ok(Value::Object(object))
    }

    fn object(
        &mut self,
        fields: &[Field],
        flattened: &[TypeRef],
        direction: Direction,
    ) -> Result<Map<String, Value>, Gap> {
        let mut object = self.fields(fields, direction)?;
        for flat in flattened {
            self.merge_flat(flat, direction, &mut object)?;
        }
        Ok(object)
    }

    fn fields(
        &mut self,
        fields: &[Field],
        direction: Direction,
    ) -> Result<Map<String, Value>, Gap> {
        let mut object = Map::new();
        for field in fields {
            if !crosses(field, direction) {
                continue;
            }
            if let FieldDefault::Value(value) = &field.default {
                object.insert(field.wire.clone(), value.clone());
                continue;
            }
            match self.value_in(&field.ty, direction) {
                Ok(value) => {
                    object.insert(field.wire.clone(), value);
                }
                Err(_) if field.presence == Presence::Optional => {}
                Err(error) => return Err(error),
            }
        }
        Ok(object)
    }

    fn merge_flat(
        &mut self,
        ty: &TypeRef,
        direction: Direction,
        object: &mut Map<String, Value>,
    ) -> Result<(), Gap> {
        let TypeRef::Named(named) = ty else {
            return Err(Gap::Unsupported(
                "a flattened inline type has no object shape".into(),
            ));
        };
        if self.stack.contains(&named.id) {
            return Err(Gap::Recursive(named.id.clone()));
        }
        let Some(definition) = self.document.types.get(&named.id) else {
            return Err(Gap::Unsupported(format!("unknown type {}", named.id)));
        };
        self.stack.push(named.id.clone());
        let result = match &definition.definition {
            TypeKind::Newtype { inner } => self.merge_flat(inner, direction, object),
            TypeKind::Struct {
                fields, flattened, ..
            } => {
                let mut own = self.fields(fields, direction)?;
                for flat in flattened {
                    self.merge_flat(flat, direction, &mut own)?;
                }
                object.extend(own);
                Ok(())
            }
            TypeKind::Enum { .. } => Err(Gap::Unsupported(format!(
                "a flattened enum {} has no object shape",
                named.id
            ))),
        };
        self.stack.pop();
        result
    }

    fn enumeration(
        &mut self,
        repr: &EnumRepr,
        variants: &[Variant],
        selected: Option<&str>,
        direction: Direction,
    ) -> Result<Value, Gap> {
        let variant = match selected {
            Some(selected) => variants
                .iter()
                .find(|variant| variant.wire == selected)
                .ok_or_else(|| Gap::Unsupported(format!("the enum has no variant {selected:?}")))?,
            None => variants
                .first()
                .ok_or_else(|| Gap::Unsupported("the enum has no variants".into()))?,
        };
        match repr {
            EnumRepr::String => {
                if matches!(variant.payload, VariantPayload::Unit) {
                    Ok(Value::String(variant.wire.clone()))
                } else {
                    Err(Gap::Unsupported(format!(
                        "the string variant {:?} carries a payload",
                        variant.wire
                    )))
                }
            }
            EnumRepr::External => match &variant.payload {
                VariantPayload::Unit => Ok(Value::String(variant.wire.clone())),
                payload => {
                    let mut object = Map::new();
                    object.insert(variant.wire.clone(), self.payload(payload, direction)?);
                    Ok(Value::Object(object))
                }
            },
            EnumRepr::Internal { tag } => {
                let mut object = match &variant.payload {
                    VariantPayload::Unit | VariantPayload::Tuple { .. } => Map::new(),
                    VariantPayload::Struct { fields } => self.fields(fields, direction)?,
                    VariantPayload::Newtype { ty } => match self.value_in(ty, direction)? {
                        Value::Object(object) => object,
                        _ => {
                            return Err(Gap::Unsupported(format!(
                                "the internal variant {:?} needs an object payload",
                                variant.wire
                            )));
                        }
                    },
                };
                object.insert(tag.clone(), Value::String(variant.wire.clone()));
                Ok(Value::Object(object))
            }
            EnumRepr::Adjacent { tag, content } => {
                let mut object = Map::new();
                object.insert(tag.clone(), Value::String(variant.wire.clone()));
                if !matches!(variant.payload, VariantPayload::Unit) {
                    object.insert(content.clone(), self.payload(&variant.payload, direction)?);
                }
                Ok(Value::Object(object))
            }
            EnumRepr::Untagged => self.payload(&variant.payload, direction),
        }
    }

    fn payload(&mut self, payload: &VariantPayload, direction: Direction) -> Result<Value, Gap> {
        match payload {
            VariantPayload::Unit => Ok(Value::Null),
            VariantPayload::Newtype { ty } => self.value_in(ty, direction),
            VariantPayload::Tuple { items } => items
                .iter()
                .map(|item| self.value_in(item, direction))
                .collect::<Result<Vec<_>, _>>()
                .map(Value::Array),
            VariantPayload::Struct { fields } => self.fields(fields, direction).map(Value::Object),
        }
    }
}

pub(crate) fn crosses(field: &Field, direction: Direction) -> bool {
    !matches!(
        (direction, field.access),
        (Direction::Input, Some(Access::ReadOnly)) | (Direction::Output, Some(Access::WriteOnly))
    )
}

fn fit(
    candidate: String,
    format: Option<FieldFormat>,
    min_length: Option<u64>,
    max_length: Option<u64>,
) -> Option<String> {
    let length = candidate.chars().count() as u64;
    if min_length.is_none_or(|minimum| length >= minimum)
        && max_length.is_none_or(|maximum| length <= maximum)
    {
        return Some(candidate);
    }
    match format {
        None | Some(FieldFormat::Uri) => {
            let mut candidate = candidate;
            if let Some(minimum) = min_length {
                let length = candidate.chars().count() as u64;
                if length < minimum {
                    candidate.push_str(&"x".repeat(usize::try_from(minimum - length).ok()?));
                }
            }
            if let Some(maximum) = max_length
                && candidate.chars().count() as u64 > maximum
            {
                candidate = candidate
                    .chars()
                    .take(usize::try_from(maximum).ok()?)
                    .collect();
            }
            Some(candidate)
        }
        Some(FieldFormat::Email) => {
            let (local, domain) = candidate.split_once('@')?;
            let mut local = local.to_owned();
            let total = |local: &str| (local.chars().count() + 1 + domain.chars().count()) as u64;
            if max_length.is_some_and(|maximum| total(&local) > maximum) {
                return None;
            }
            if let Some(minimum) = min_length {
                let missing = minimum.saturating_sub(total(&local));
                local.push_str(&"x".repeat(usize::try_from(missing).ok()?));
            }
            Some(format!("{local}@{domain}"))
        }
        Some(FieldFormat::Duration) => {
            let digits = candidate
                .strip_prefix("PT")
                .and_then(|rest| rest.strip_suffix('S'))?;
            let length = candidate.chars().count() as u64;
            let target = if min_length.is_some_and(|minimum| length < minimum) {
                min_length?
            } else {
                max_length?
            };
            let width = target.checked_sub(3)?;
            let width = usize::try_from(width).unwrap_or(usize::MAX).max(1);
            if digits.chars().count() > width {
                let shortened = digits
                    .chars()
                    .skip(digits.chars().count() - width)
                    .collect::<String>();
                return Some(format!("PT{shortened}S"));
            }
            Some(format!("PT{digits:0>width$}S"))
        }
        Some(_) => None,
    }
}

fn sign(signed: bool) -> &'static str {
    if signed { "i" } else { "u" }
}

fn lower_bound(bits: u16) -> i128 {
    match bits {
        128 => i128::MIN,
        bits if bits < 128 => -(1_i128 << (bits - 1)),
        _ => i128::MIN,
    }
}

fn upper_bound(bits: u16) -> i128 {
    match bits {
        128 => i128::MAX,
        bits if bits < 128 => (1_i128 << (bits - 1)) - 1,
        _ => i128::MAX,
    }
}

fn unsigned_upper_bound(bits: u16) -> u128 {
    match bits {
        128 => u128::MAX,
        bits if bits < 128 => (1_u128 << bits) - 1,
        _ => u128::MAX,
    }
}

fn integer_bound(value: &Value) -> Option<i128> {
    value
        .as_i64()
        .map(i128::from)
        .or_else(|| value.as_u64().map(i128::from))
        .or_else(|| value.as_str().and_then(|text| text.parse().ok()))
}

fn unsigned_bound(value: &Value) -> Option<u128> {
    value
        .as_u64()
        .map(u128::from)
        .or_else(|| value.as_str().and_then(|text| text.parse().ok()))
}
