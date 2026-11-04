use serde_json::{Map, Value};

use super::Validator;
use super::violations::child;
use crate::document::{Exact, exact_integer};
use crate::violation::Violation;
use crate::{
    Access, Direction, EnumRepr, Field, FieldDefault, ObjectOpenness, Presence, TypeRef, Variant,
    VariantPayload,
};

pub(super) struct Member<'a> {
    wire: &'a str,
    aliases: Option<&'a Value>,
    ty: &'a TypeRef,
    presence: Presence,
    access: Option<Access>,
    filled: bool,
}

impl<'a> Member<'a> {
    pub(super) fn new(field: &'a Field) -> Self {
        Self {
            wire: &field.wire,
            aliases: field.ext.get(crate::ext::ALIAS),
            ty: &field.ty,
            presence: field.presence,
            access: field.access,
            filled: matches!(field.default, FieldDefault::Value(_)),
        }
    }

    fn names(&self, aliased: bool) -> impl Iterator<Item = &'a str> + use<'a> {
        let aliases = aliased.then_some(self.aliases).flatten();
        std::iter::once(self.wire).chain(
            aliases
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str),
        )
    }

    fn accepts(&self, name: &str, aliased: bool) -> bool {
        self.names(aliased).any(|accepted| accepted == name)
    }
}

impl<'a> Validator<'a> {
    pub(super) fn sequence(
        &mut self,
        item: &'a TypeRef,
        unique: Option<bool>,
        path: &str,
        value: &Value,
    ) {
        let Some(items) = value.as_array() else {
            return self.type_violation(path, "an array", value);
        };
        let mut seen: Vec<(&Value, usize)> = Vec::new();
        for (index, element) in items.iter().enumerate() {
            self.position.clear();
            self.check(item, &child(path, &index.to_string()), element);
            if unique != Some(true) {
                continue;
            }
            match seen.iter().find(|(other, _)| same_item(other, element)) {
                Some((_, first)) => self.reject(Violation::DuplicateItem {
                    path: path.to_owned(),
                    item: index,
                    first: *first,
                }),
                None => seen.push((element, index)),
            }
        }
    }

    pub(super) fn mapping(
        &mut self,
        key: &'a TypeRef,
        item: &'a TypeRef,
        path: &str,
        value: &Value,
    ) {
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

    pub(super) fn tuple(&mut self, items: &'a [TypeRef], path: &str, value: &Value) {
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

    pub(super) fn object(
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
            members.push(Member::new(field));
        }
        for member in flattened {
            self.flattened(member, &mut Vec::new(), &mut members);
        }
        for member in &members {
            self.member(member, entries, path);
        }
        let aliased = self.direction == Direction::Input;
        match openness {
            ObjectOpenness::Reject => {
                for name in entries.keys() {
                    if !members.iter().any(|member| member.accepts(name, aliased)) {
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
                    if !members.iter().any(|member| member.accepts(name, aliased)) {
                        self.check(extra, &child(path, name), entry);
                    }
                }
            }
            ObjectOpenness::PreserveOpaque => {}
        }
    }

    fn member(&mut self, member: &Member<'a>, entries: &Map<String, Value>, path: &str) {
        let aliased = self.direction == Direction::Input;
        let crosses = !matches!(
            (self.direction, member.access),
            (Direction::Input, Some(Access::ReadOnly))
                | (Direction::Output, Some(Access::WriteOnly))
        );
        if !crosses {
            let present = member.names(aliased).any(|name| entries.contains_key(name));
            if present || (member.presence == Presence::Required && !member.filled) {
                return self.reject(Violation::Access {
                    path: child(path, member.wire),
                    field: member.wire.to_owned(),
                    access: member.access.unwrap_or(Access::ReadWrite),
                    direction: self.direction,
                });
            }
            return;
        }
        let mut first: Option<(&str, &Value)> = None;
        for name in member.names(aliased) {
            let Some(value) = entries.get(name) else {
                continue;
            };
            match first {
                Some((already, _)) => self.reject(Violation::DuplicateField {
                    path: child(path, name),
                    field: member.wire.to_owned(),
                    first: already.to_owned(),
                }),
                None => first = Some((name, value)),
            }
        }
        let path = child(path, member.wire);
        match first {
            Some((_, value)) => {
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

    pub(super) fn enumeration(
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

fn same_item(left: &Value, right: &Value) -> bool {
    match (left, right) {
        (Value::Number(_), Value::Number(_)) => same_number(left, right),
        (Value::Array(left), Value::Array(right)) => {
            left.len() == right.len()
                && left
                    .iter()
                    .zip(right)
                    .all(|(left, right)| same_item(left, right))
        }
        (Value::Object(left), Value::Object(right)) => {
            left.len() == right.len()
                && left
                    .iter()
                    .all(|(key, value)| right.get(key).is_some_and(|found| same_item(value, found)))
        }
        _ => left == right,
    }
}

fn same_number(left: &Value, right: &Value) -> bool {
    let float = |value: &Value| {
        value
            .as_number()
            .filter(|number| number.is_f64())
            .and_then(|number| number.as_f64())
    };
    match (float(left), float(right)) {
        (Some(left), Some(right)) => left == right,
        (Some(number), None) => {
            matches!(exact_integer(right), Exact::Integer(integer) if same_magnitude(number, integer))
        }
        (None, Some(number)) => {
            matches!(exact_integer(left), Exact::Integer(integer) if same_magnitude(number, integer))
        }
        (None, None) => match (exact_integer(left), exact_integer(right)) {
            (Exact::Integer(one), Exact::Integer(other)) => one == other,
            _ => left == right,
        },
    }
}

fn same_magnitude(number: f64, (negative, magnitude): (bool, u128)) -> bool {
    let found = if negative { -number } else { number };
    found.is_finite() && number.fract() == 0.0 && found >= 0.0 && found as u128 == magnitude
}
