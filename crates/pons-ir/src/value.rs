use std::cmp::Ordering;

use serde_json::{Map, Value};

use crate::document::integer_magnitude;
use crate::violation::{IntegerProblem, Violation, describe};
use crate::{
    Access, ByteEncoding, DecimalWire, Direction, Document, EnumRepr, Field, InlineType,
    IntegerWire, NamedRef, ObjectOpenness, Presence, TypeKind, TypeRef, Variant, VariantPayload,
};

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

struct Member<'a> {
    wire: &'a str,
    ty: &'a TypeRef,
    presence: Presence,
    access: Option<Access>,
}

struct Validator<'a> {
    document: &'a Document,
    direction: Direction,
    violations: Vec<Violation>,

    position: Vec<String>,
}

impl<'a> Validator<'a> {
    fn reject(&mut self, violation: Violation) {
        self.violations.push(violation);
    }

    fn type_violation(&mut self, path: &str, expected: &str, found: &Value) {
        self.reject(Violation::Type {
            path: path.to_owned(),
            expected: expected.to_owned(),
            found: describe(found),
        });
    }

    fn content_violation(&mut self, path: &str, expected: &str, found: &Value) {
        self.reject(Violation::Type {
            path: path.to_owned(),
            expected: expected.to_owned(),
            found: found.to_string(),
        });
    }

    fn expect(
        &mut self,
        path: &str,
        expected: &str,
        value: &Value,
        shape: fn(&Value) -> bool,
    ) -> bool {
        if shape(value) {
            true
        } else {
            self.type_violation(path, expected, value);
            false
        }
    }

    fn check(&mut self, ty: &'a TypeRef, path: &str, value: &Value) {
        match ty {
            TypeRef::Named(named) => self.named(named, path, value),
            TypeRef::Inline(inline) => self.inline(inline, path, value),
        }
    }

    fn named(&mut self, named: &'a NamedRef, path: &str, value: &Value) {
        let document = self.document;
        let Some(definition) = document.types.get(&named.id) else {
            return self.reject(Violation::UnknownType {
                path: path.to_owned(),
                reference: named.id.clone(),
            });
        };
        if self.position.contains(&named.id) {
            return self.reject(Violation::RecursiveType {
                path: path.to_owned(),
                reference: named.id.clone(),
            });
        }
        match &definition.definition {
            TypeKind::Newtype { inner } => {
                self.position.push(named.id.clone());
                self.check(inner, path, value);
                self.position.pop();
            }
            TypeKind::Struct {
                fields,
                flattened,
                openness,
            } => self.object(fields, flattened, openness, path, value),
            TypeKind::Enum { repr, variants } => {
                self.enumeration(repr, variants, named.variant.as_deref(), path, value)
            }
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
        }
    }

    fn integer(&mut self, ty: &'a InlineType, path: &str, value: &Value) {
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
        let Some(integer) = self.exact_integer(*wire, bits, signed, path, value) else {
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

    fn exact_integer(
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
                match integer_magnitude(value) {
                    Some(integer) => Some(integer),
                    None => {
                        self.reject(Violation::Integer {
                            path: path.to_owned(),
                            problem: IntegerProblem::NotAnInteger {
                                found: value.to_string(),
                            },
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

    fn floating(&mut self, bits: u8, path: &str, value: &Value) {
        self.number(path, value);
        if bits != 32 {
            return;
        }
        match value.as_f64() {
            Some(number) if number.is_finite() && number.abs() > f64::from(f32::MAX) => {
                self.reject(Violation::FloatRange {
                    path: path.to_owned(),
                    bits,
                });
            }
            _ => {}
        }
    }

    fn decimal(&mut self, wire: DecimalWire, path: &str, value: &Value) {
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

    fn text(
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

    fn byte_string(&mut self, encoding: ByteEncoding, path: &str, value: &Value) {
        if encoding == ByteEncoding::Array {
            let Some(items) = value.as_array() else {
                return self.type_violation(path, "an array of bytes", value);
            };
            for (index, item) in items.iter().enumerate() {
                let path = child(path, &index.to_string());
                match integer_magnitude(item) {
                    Some((false, byte)) if byte <= 255 => {}
                    Some(_) => self.content_violation(&path, "a byte between 0 and 255", item),
                    None => self.type_violation(&path, "a byte between 0 and 255", item),
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

    fn sequence(&mut self, item: &'a TypeRef, unique: Option<bool>, path: &str, value: &Value) {
        let Some(items) = value.as_array() else {
            return self.type_violation(path, "an array", value);
        };
        let mut seen: Vec<(String, usize)> = Vec::new();
        for (index, element) in items.iter().enumerate() {
            self.position.clear();
            self.check(item, &child(path, &index.to_string()), element);
            if unique != Some(true) {
                continue;
            }
            let rendered = element.to_string();
            match seen.iter().find(|(other, _)| *other == rendered) {
                Some((_, first)) => self.reject(Violation::DuplicateItem {
                    path: path.to_owned(),
                    item: index,
                    first: *first,
                }),
                None => seen.push((rendered, index)),
            }
        }
    }

    fn mapping(&mut self, key: &'a TypeRef, item: &'a TypeRef, path: &str, value: &Value) {
        let Some(entries) = value.as_object() else {
            return self.type_violation(path, "an object", value);
        };
        for (name, entry) in entries {
            let path = child(path, name);
            self.position.clear();
            self.check(key, &path, &Value::String(name.clone()));
            self.check(item, &path, entry);
        }
    }

    fn tuple(&mut self, items: &'a [TypeRef], path: &str, value: &Value) {
        let Some(elements) = value.as_array() else {
            return self.type_violation(path, "an array", value);
        };
        if elements.len() != items.len() {
            return self.reject(Violation::TupleArity {
                path: path.to_owned(),
                expected: items.len(),
                found: elements.len(),
            });
        }
        for (index, (item, element)) in items.iter().zip(elements).enumerate() {
            self.position.clear();
            self.check(item, &child(path, &index.to_string()), element);
        }
    }

    fn object(
        &mut self,
        fields: &'a [Field],
        flattened: &'a [TypeRef],
        openness: &'a ObjectOpenness,
        path: &str,
        value: &Value,
    ) {
        if !self.expect(path, "an object", value, Value::is_object) {
            return;
        }
        let Some(entries) = value.as_object() else {
            return;
        };
        let mut members = Vec::new();
        for field in fields {
            members.push(Member {
                wire: &field.wire,
                ty: &field.ty,
                presence: field.presence,
                access: field.access,
            });
        }
        for member in flattened {
            self.flattened(member, &mut Vec::new(), &mut members);
        }
        for member in &members {
            self.member(member, entries, path);
        }
        match openness {
            ObjectOpenness::Reject => {
                for name in entries.keys() {
                    if !members.iter().any(|member| member.wire == name) {
                        self.reject(Violation::UnexpectedField {
                            path: child(path, name),
                            field: name.clone(),
                        });
                    }
                }
            }
            ObjectOpenness::PreserveTyped { value } => {
                let extra = self.openness_value(value, &mut Vec::new());
                for (name, entry) in entries {
                    if !members.iter().any(|member| member.wire == name) {
                        self.check(extra, &child(path, name), entry);
                    }
                }
            }
            ObjectOpenness::PreserveOpaque => {}
        }
    }

    fn member(&mut self, member: &Member<'a>, entries: &Map<String, Value>, path: &str) {
        let path = child(path, member.wire);
        let crosses = !matches!(
            (self.direction, member.access),
            (Direction::Input, Some(Access::ReadOnly))
                | (Direction::Output, Some(Access::WriteOnly))
        );
        let present = entries.contains_key(member.wire);
        if !crosses {
            if present || member.presence == Presence::Required {
                return self.reject(Violation::Access {
                    path,
                    field: member.wire.to_owned(),
                    access: member.access.unwrap_or(Access::ReadWrite),
                    direction: self.direction,
                });
            }
            return;
        }
        match entries.get(member.wire) {
            Some(value) => {
                self.position.clear();
                self.check(member.ty, &path, value)
            }
            None if member.presence == Presence::Required => {
                self.reject(Violation::MissingField {
                    path,
                    field: member.wire.to_owned(),
                });
            }
            None => {}
        }
    }

    fn flattened(
        &self,
        ty: &'a TypeRef,
        newtypes: &mut Vec<String>,
        members: &mut Vec<Member<'a>>,
    ) {
        let TypeRef::Named(named) = ty else {
            return;
        };
        if newtypes.contains(&named.id) {
            return;
        }
        let Some(definition) = self.document.types.get(&named.id) else {
            return;
        };
        match &definition.definition {
            TypeKind::Newtype { inner } => {
                newtypes.push(named.id.clone());
                self.flattened(inner, newtypes, members);
                newtypes.pop();
            }
            TypeKind::Struct { fields, .. } => members.extend(fields.iter().map(|field| Member {
                wire: &field.wire,
                ty: &field.ty,
                presence: field.presence,
                access: field.access,
            })),
            TypeKind::Enum { .. } => {}
        }
    }

    fn openness_value(&self, ty: &'a TypeRef, newtypes: &mut Vec<String>) -> &'a TypeRef {
        let mut current = ty;
        loop {
            match current {
                TypeRef::Inline(InlineType::Map { value, .. }) => return value,
                TypeRef::Named(named) if !newtypes.contains(&named.id) => {
                    match self
                        .document
                        .types
                        .get(&named.id)
                        .map(|found| &found.definition)
                    {
                        Some(TypeKind::Newtype { inner }) => {
                            newtypes.push(named.id.clone());
                            current = inner;
                        }
                        _ => return current,
                    }
                }
                _ => return current,
            }
        }
    }

    fn enumeration(
        &mut self,
        repr: &'a EnumRepr,
        variants: &'a [Variant],
        selected: Option<&str>,
        path: &str,
        value: &Value,
    ) {
        match repr {
            EnumRepr::Untagged => {
                let candidates = variants
                    .iter()
                    .filter(|variant| selected.is_none_or(|wire| variant.wire == wire))
                    .collect::<Vec<_>>();
                if candidates.is_empty() {
                    return self.reject(Violation::Variant {
                        path: path.to_owned(),
                        expected: selected.map(str::to_owned),
                        found: value.to_string(),
                    });
                }
                for variant in candidates {
                    let mark = self.violations.len();
                    self.payload(variant, path, value, true);
                    if self.violations.len() == mark {
                        return;
                    }
                    self.violations.truncate(mark);
                }
                self.reject(Violation::NoVariant {
                    path: path.to_owned(),
                });
            }
            EnumRepr::External if value.is_string() => {
                let Some(variant) = self.variant(variants, selected, path, value) else {
                    return;
                };
                self.payload(variant, path, value, false);
            }
            EnumRepr::External => {
                if !self.expect(path, "an object", value, Value::is_object) {
                    return;
                }
                let Some(entries) = value.as_object() else {
                    return;
                };
                if entries.len() != 1 {
                    return self.type_violation(path, "an object with one variant", value);
                }
                let Some((wire, payload)) = entries.iter().next() else {
                    return;
                };
                let Some(variant) = self.named_variant(variants, selected, path, wire) else {
                    return;
                };
                self.payload(variant, &child(path, wire), payload, false);
            }
            EnumRepr::Internal { tag } => {
                let Some(variant) = self.tagged(variants, selected, path, value, tag) else {
                    return;
                };
                if matches!(variant.payload, VariantPayload::Tuple { .. }) {
                    self.expect(path, "an object", value, Value::is_object);
                    return;
                }
                self.payload(variant, path, value, false);
            }
            EnumRepr::Adjacent { tag, content } => {
                let Some(variant) = self.tagged(variants, selected, path, value, tag) else {
                    return;
                };
                if matches!(variant.payload, VariantPayload::Unit) {
                    return;
                }
                let Some(entries) = value.as_object() else {
                    return;
                };
                let Some(payload) = entries.get(content) else {
                    return self.reject(Violation::MissingField {
                        path: child(path, content),
                        field: content.clone(),
                    });
                };
                self.payload(variant, &child(path, content), payload, false);
            }
            EnumRepr::String => {
                let Some(variant) = self.variant(variants, selected, path, value) else {
                    return;
                };
                self.payload(variant, path, value, false);
            }
        }
    }

    fn variant(
        &mut self,
        variants: &'a [Variant],
        selected: Option<&str>,
        path: &str,
        value: &Value,
    ) -> Option<&'a Variant> {
        let Some(wire) = value.as_str() else {
            self.type_violation(path, "an enum variant name", value);
            return None;
        };
        self.named_variant(variants, selected, path, wire)
    }

    fn named_variant(
        &mut self,
        variants: &'a [Variant],
        selected: Option<&str>,
        path: &str,
        wire: &str,
    ) -> Option<&'a Variant> {
        let variant = variants.iter().find(|variant| variant.wire == wire);
        if variant.is_some() && selected.is_none_or(|selected| selected == wire) {
            return variant;
        }
        self.reject(Violation::Variant {
            path: path.to_owned(),
            expected: selected.map(str::to_owned),
            found: wire.to_owned(),
        });
        None
    }

    fn tagged(
        &mut self,
        variants: &'a [Variant],
        selected: Option<&str>,
        path: &str,
        value: &Value,
        tag: &str,
    ) -> Option<&'a Variant> {
        if !self.expect(path, "an object", value, Value::is_object) {
            return None;
        }
        let entries = value.as_object()?;
        let Some(wire) = entries.get(tag) else {
            self.reject(Violation::MissingField {
                path: child(path, tag),
                field: tag.to_owned(),
            });
            return None;
        };
        let path = child(path, tag);
        let Some(wire) = wire.as_str() else {
            self.type_violation(&path, "an enum variant name", wire);
            return None;
        };
        self.named_variant(variants, selected, &path, wire)
    }

    fn payload(&mut self, variant: &'a Variant, path: &str, value: &Value, untagged: bool) {
        match &variant.payload {
            VariantPayload::Unit if untagged && !value.is_null() => {
                self.type_violation(path, "null", value)
            }
            VariantPayload::Unit => {}
            VariantPayload::Newtype { ty } => self.check(ty, path, value),
            VariantPayload::Tuple { items } => self.tuple(items, path, value),
            VariantPayload::Struct { fields } => {
                self.object(fields, &[], &ObjectOpenness::PreserveOpaque, path, value)
            }
        }
    }
}

fn child(path: &str, token: &str) -> String {
    format!("{path}/{}", token.replace('~', "~0").replace('/', "~1"))
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
        integer_magnitude(bound).unwrap_or((false, u128::MAX)),
    )
}

fn divisible(integer: (bool, u128), multiple_of: Option<&Value>) -> Option<IntegerProblem> {
    let step = multiple_of?;
    let Some((negative, divisor)) = integer_magnitude(step) else {
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
