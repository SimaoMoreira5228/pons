use std::collections::BTreeSet;

use serde_json::Value;

use super::{Document, Exact, ValidationError, exact_integer};
use crate::operation::{ErrorResponse, Operation};
use crate::types::{
    Access, Direction, EnumRepr, Field, FieldDefault, InlineType, OpaqueWire, Presence, TypeDef,
    TypeKind, TypeRef, Variant, VariantPayload,
};

fn render(ty: &TypeRef) -> String {
    serde_json::to_string(ty).unwrap_or_default()
}

impl Document {
    pub(super) fn validate_type(
        &self,
        id: &str,
        definition: &TypeDef,
        errors: &mut Vec<ValidationError>,
    ) {
        let owner = format!("type {id}");
        if id.is_empty()
            || definition.name.is_empty()
            || definition.name.iter().any(String::is_empty)
        {
            errors.push(ValidationError::InvalidValue {
                path: owner.clone(),
                message: "named types need a non-empty id and name".into(),
            });
        }
        definition.definition.visit_refs(&mut |reference| {
            self.validate_ref(&owner, reference, errors);
            self.validate_inline(&owner, reference, errors);
        });
        if let TypeKind::Struct { flattened, .. } = &definition.definition {
            for (index, ty) in flattened.iter().enumerate() {
                if self.is_nullable_type(ty, &mut BTreeSet::new()) {
                    errors.push(ValidationError::InvalidValue {
                        path: format!("{owner}.flattened[{index}]"),
                        message: "nullable flattened types are not representable".into(),
                    });
                }
                if !self.inlines_a_struct(ty, &mut BTreeSet::new()) {
                    errors.push(ValidationError::InvalidValue {
                        path: format!("{owner}.flattened[{index}]"),
                        message: format!(
                            "flattened {} is not a struct, because a flattened member contributes its fields to one object",
                            render(ty)
                        ),
                    });
                }
                if let Some(repeated) = self.flatten_cycle(ty, &mut vec![id.to_owned()]) {
                    errors.push(ValidationError::InvalidValue {
                        path: format!("{owner}.flattened[{index}]"),
                        message: format!(
                            "the type {repeated} cannot be inlined because flattened members form a cycle"
                        ),
                    });
                }
            }
        }

        match &definition.definition {
            TypeKind::Struct { fields, .. } => {
                self.validate_fields(&owner, definition.direction, fields, errors)
            }
            TypeKind::Enum { repr, variants } => {
                match repr {
                    EnumRepr::Internal { tag } if tag.is_empty() => {
                        errors.push(ValidationError::InvalidValue {
                            path: owner.clone(),
                            message: "enum tag cannot be empty".into(),
                        });
                    }
                    EnumRepr::Adjacent { tag, content } => {
                        if tag.is_empty() {
                            errors.push(ValidationError::InvalidValue {
                                path: owner.clone(),
                                message: "adjacent enum tag cannot be empty".into(),
                            });
                        }
                        if content.is_empty() {
                            errors.push(ValidationError::InvalidValue {
                                path: owner.clone(),
                                message: "adjacent enum content cannot be empty".into(),
                            });
                        }
                        if tag == content {
                            errors.push(ValidationError::InvalidValue {
                                path: owner.clone(),
                                message: "adjacent enum tag and content must differ".into(),
                            });
                        }
                    }
                    _ => {}
                }
                if variants.is_empty() {
                    errors.push(ValidationError::InvalidValue {
                        path: owner.clone(),
                        message: "enum needs at least one variant".into(),
                    });
                }
                if variants.len() == 1
                    && !matches!(definition.direction, Direction::Output)
                    && let Some(variant) = variants.first()
                    && crate::ext::catch_all(variant)
                {
                    errors.push(ValidationError::InvalidValue {
                        path: owner.clone(),
                        message: format!(
                            "the only variant {} is the {} catch-all, and an input-side type the request reads accepts anything, so the contract describes nothing",
                            variant.wire,
                            crate::ext::OPEN_VARIANT
                        ),
                    });
                }
                let mut names = BTreeSet::new();
                for variant in variants {
                    self.validate_diagnostics(
                        &format!("{owner}.variants.{}", variant.wire),
                        &variant.diagnostics,
                        errors,
                    );
                    if !names.insert(&variant.wire) {
                        errors.push(ValidationError::InvalidValue {
                            path: owner.clone(),
                            message: format!("duplicate enum variant {}", variant.wire),
                        });
                    }
                    if crate::ext::catch_all(variant)
                        && !matches!(variant.payload, VariantPayload::Unit)
                    {
                        errors.push(ValidationError::InvalidValue {
                            path: owner.clone(),
                            message: format!(
                                "the {} catch-all on variant {} must have a unit payload, because it is the value no other variant claims",
                                crate::ext::OPEN_VARIANT,
                                variant.wire
                            ),
                        });
                    }
                    if matches!(repr, EnumRepr::String)
                        && !matches!(variant.payload, VariantPayload::Unit)
                    {
                        errors.push(ValidationError::InvalidValue {
                            path: owner.clone(),
                            message: "string enum variants must have unit payloads".into(),
                        });
                    }
                    if matches!(repr, EnumRepr::Internal { .. })
                        && matches!(variant.payload, VariantPayload::Tuple { .. })
                    {
                        errors.push(ValidationError::InvalidValue {
                            path: owner.clone(),
                            message: format!(
                                "internally tagged enum variant {} must have an object payload, because the tag shares one object with it",
                                variant.wire
                            ),
                        });
                    }
                    let reserved = match repr {
                        EnumRepr::Internal { tag } => vec![tag.as_str()],
                        EnumRepr::Adjacent { tag, content } => vec![tag.as_str(), content.as_str()],
                        _ => Vec::new(),
                    };
                    if let VariantPayload::Struct { fields } = &variant.payload {
                        self.validate_fields(&owner, definition.direction, fields, errors);
                    }
                    if !reserved.is_empty()
                        && self.payload_uses_reserved(
                            &variant.payload,
                            &reserved,
                            &mut BTreeSet::new(),
                        )
                    {
                        errors.push(ValidationError::InvalidValue {
                            path: owner.clone(),
                            message: format!("variant {} uses a reserved enum field", variant.wire),
                        });
                    }
                }
            }
            TypeKind::Newtype { inner } => {
                if self.reenters(id, inner, &mut BTreeSet::new()) {
                    errors.push(ValidationError::InvalidValue {
                        path: owner.clone(),
                        message: format!(
                            "the type {id} cannot be resolved because newtypes form a cycle"
                        ),
                    });
                }
            }
        }
    }

    fn inlines_a_struct(&self, ty: &TypeRef, seen: &mut BTreeSet<String>) -> bool {
        match ty {
            TypeRef::Inline(InlineType::Opaque { .. }) => true,
            TypeRef::Named(named) => {
                if named.variant.is_some() || !seen.insert(named.id.clone()) {
                    return false;
                }
                self.types
                    .get(&named.id)
                    .is_some_and(|found| match &found.definition {
                        TypeKind::Struct { .. } => true,
                        TypeKind::Newtype { inner } => self.inlines_a_struct(inner, seen),
                        TypeKind::Enum { .. } => false,
                    })
            }
            TypeRef::Inline(_) => false,
        }
    }

    fn flatten_cycle(&self, member: &TypeRef, visiting: &mut Vec<String>) -> Option<String> {
        let TypeRef::Named(named) = member else {
            return None;
        };
        if visiting.contains(&named.id) {
            return Some(named.id.clone());
        }
        let definition = self.types.get(&named.id)?;
        visiting.push(named.id.clone());
        let cyclic = match &definition.definition {
            TypeKind::Newtype { inner } => self.flatten_cycle(inner, visiting),
            TypeKind::Struct { flattened, .. } => flattened
                .iter()
                .find_map(|member| self.flatten_cycle(member, visiting)),
            TypeKind::Enum { .. } => None,
        };
        visiting.pop();
        cyclic
    }

    fn reenters(
        &self,
        id: &str,
        reference: &TypeRef,
        seen: &mut BTreeSet<(String, Option<String>)>,
    ) -> bool {
        match reference {
            TypeRef::Named(named) => {
                if named.id == id {
                    return true;
                }
                if !seen.insert((named.id.clone(), named.variant.clone())) {
                    return false;
                }
                let Some(definition) = self.types.get(&named.id) else {
                    return false;
                };
                match &definition.definition {
                    TypeKind::Newtype { inner } => self.reenters(id, inner, seen),
                    TypeKind::Enum { variants, .. } => match named
                        .variant
                        .as_deref()
                        .and_then(|variant| variants.iter().find(|entry| entry.wire == variant))
                    {
                        Some(Variant {
                            payload: VariantPayload::Newtype { ty },
                            ..
                        }) => self.reenters(id, ty, seen),
                        _ => false,
                    },
                    TypeKind::Struct { .. } => false,
                }
            }
            TypeRef::Inline(InlineType::Nullable { inner }) => self.reenters(id, inner, seen),
            TypeRef::Inline(InlineType::Array { item, .. }) => self.reenters(id, item, seen),
            TypeRef::Inline(InlineType::Map { key, value }) => {
                self.reenters(id, key, seen) || self.reenters(id, value, seen)
            }
            TypeRef::Inline(InlineType::Tuple { items }) => {
                items.iter().any(|item| self.reenters(id, item, seen))
            }
            TypeRef::Inline(_) => false,
        }
    }

    fn payload_uses_reserved(
        &self,
        payload: &VariantPayload,
        reserved: &[&str],
        seen: &mut BTreeSet<String>,
    ) -> bool {
        match payload {
            VariantPayload::Struct { fields } => fields
                .iter()
                .any(|field| reserved.contains(&field.wire.as_str())),
            VariantPayload::Newtype { ty } => self.named_uses_reserved(ty, reserved, seen),
            VariantPayload::Unit | VariantPayload::Tuple { .. } => false,
        }
    }

    fn named_uses_reserved(
        &self,
        ty: &TypeRef,
        reserved: &[&str],
        seen: &mut BTreeSet<String>,
    ) -> bool {
        let TypeRef::Named(named) = ty else {
            return false;
        };
        if named.variant.is_some() || !seen.insert(named.id.clone()) {
            return false;
        }
        let found = match self.types.get(&named.id).map(|found| &found.definition) {
            Some(TypeKind::Struct {
                fields, flattened, ..
            }) => {
                fields
                    .iter()
                    .any(|field| reserved.contains(&field.wire.as_str()))
                    || flattened
                        .iter()
                        .any(|member| self.named_uses_reserved(member, reserved, seen))
            }
            Some(TypeKind::Newtype { inner }) => self.named_uses_reserved(inner, reserved, seen),
            _ => false,
        };
        seen.remove(&named.id);
        found
    }

    pub(super) fn struct_field_named(
        &self,
        ty: &TypeRef,
        wire: &str,
        visited: &mut BTreeSet<String>,
    ) -> bool {
        let TypeRef::Named(named) = ty else {
            return false;
        };
        if named.variant.is_some() || !visited.insert(named.id.clone()) {
            return false;
        }
        let found = match self.types.get(&named.id).map(|found| &found.definition) {
            Some(TypeKind::Struct {
                fields, flattened, ..
            }) => {
                fields.iter().any(|field| field.wire == wire)
                    || flattened
                        .iter()
                        .any(|member| self.struct_field_named(member, wire, visited))
            }
            Some(TypeKind::Newtype { inner }) => self.struct_field_named(inner, wire, visited),
            _ => false,
        };
        visited.remove(&named.id);
        found
    }

    pub(super) fn is_byte_type(&self, ty: &TypeRef, visited: &mut BTreeSet<String>) -> bool {
        match ty {
            TypeRef::Inline(InlineType::Bytes { .. }) => true,
            TypeRef::Inline(InlineType::Nullable { inner }) => self.is_byte_type(inner, visited),
            TypeRef::Inline(InlineType::Opaque {
                wire: OpaqueWire::Bytes,
                ..
            }) => true,
            TypeRef::Named(named) => {
                if !visited.insert(named.id.clone()) {
                    return false;
                }
                let result = self.types.get(&named.id).is_some_and(|definition| {
                    matches!(
                        &definition.definition,
                        TypeKind::Newtype { inner } if self.is_byte_type(inner, visited)
                    )
                });
                visited.remove(&named.id);
                result
            }
            TypeRef::Inline(_) => false,
        }
    }

    fn validate_fields(
        &self,
        owner: &str,
        direction: Direction,
        fields: &[Field],
        errors: &mut Vec<ValidationError>,
    ) {
        const DROPPED_ON_INPUT: (Access, &str) = (
            Access::ReadOnly,
            "a read_only key never crosses the input boundary",
        );
        const DROPPED_ON_OUTPUT: (Access, &str) = (
            Access::WriteOnly,
            "a write_only key never crosses the output boundary",
        );
        let dropped: &[(Access, &str)] = match direction {
            Direction::Input => &[DROPPED_ON_INPUT],
            Direction::Output => &[DROPPED_ON_OUTPUT],
            Direction::Both => &[DROPPED_ON_INPUT, DROPPED_ON_OUTPUT],
        };
        let mut names = BTreeSet::new();
        for field in fields {
            self.validate_diagnostics(
                &format!("{owner}.fields.{}", field.wire),
                &field.diagnostics,
                errors,
            );
            if !names.insert(field.wire.as_str()) {
                errors.push(ValidationError::InvalidValue {
                    path: owner.into(),
                    message: format!("duplicate field {}", field.wire),
                });
            }
            for (access, subject) in dropped {
                if crate::query::waits_for_a_dropped_key(field, *access) {
                    errors.push(ValidationError::InvalidValue {
                        path: format!("{owner}.fields.{}", field.wire),
                        message: crate::query::dropped_key(
                            &format!("{owner} field"),
                            &field.wire,
                            *access,
                            subject,
                        ),
                    });
                }
            }
            if field.presence == Presence::Required
                && !matches!(field.default, FieldDefault::Unspecified)
                && !matches!(field.access, Some(Access::ReadOnly | Access::WriteOnly))
            {
                errors.push(ValidationError::InvalidValue {
                    path: format!("{owner}.fields.{}", field.wire),
                    message: format!(
                        "the required field {} cannot carry a default, because its key is always on the wire and only a side that drops the key is filled by one",
                        field.wire
                    ),
                });
            }
            if let FieldDefault::Value(default) = &field.default
                && let Err(contradictions) =
                    crate::value::validate_value(self, &field.ty, Direction::Both, default)
            {
                errors.push(ValidationError::InvalidValue {
                    path: format!("{owner}.fields.{}", field.wire),
                    message: format!(
                        "the default for field {} contradicts its type: {}",
                        field.wire,
                        contradictions
                            .iter()
                            .map(|violation| violation.reason())
                            .collect::<Vec<_>>()
                            .join("; ")
                    ),
                });
            }
            self.check_aliases(owner, direction, field, &mut names, errors);
        }
    }

    fn check_aliases<'a>(
        &self,
        owner: &str,
        direction: Direction,
        field: &'a Field,
        names: &mut BTreeSet<&'a str>,
        errors: &mut Vec<ValidationError>,
    ) {
        let Some(aliases) = field.ext.get(crate::ext::ALIAS) else {
            return;
        };
        let path = format!("{owner}.fields.{}", field.wire);
        let mut problems = Vec::new();
        if super::Boundary::Request.refuses(direction)
            || !super::field_crosses(field, super::Boundary::Request)
        {
            problems.push(format!(
                "the aliases of field {} name input spellings of a key that never crosses a request boundary",
                field.wire
            ));
        }
        match aliases {
            Value::Array(list) => {
                let mut listed = BTreeSet::new();
                for alias in list {
                    let Some(name) = alias.as_str().filter(|name| !name.is_empty()) else {
                        problems.push(format!(
                            "every name in extension {} must be a non-empty string, found {alias}",
                            crate::ext::ALIAS
                        ));
                        continue;
                    };
                    if name == field.wire {
                        problems.push(format!(
                            "the aliases of field {} cannot repeat its own wire name {name}",
                            field.wire
                        ));
                        continue;
                    }
                    if !listed.insert(name) {
                        problems.push(format!("the aliases of field {} repeat {name}", field.wire));
                        continue;
                    }
                    if !names.insert(name) {
                        problems.push(format!(
                            "the alias {name} of field {} is already a key {owner} owns",
                            field.wire
                        ));
                    }
                }
            }
            other => problems.push(format!(
                "extension {} must be an array of accepted input wire names, found {other}",
                crate::ext::ALIAS
            )),
        }
        errors.extend(
            problems
                .into_iter()
                .map(|message| ValidationError::InvalidValue {
                    path: path.clone(),
                    message,
                }),
        );
    }

    pub(super) fn validate_header_name(
        &self,
        owner: &str,
        context: &str,
        wire: &str,
        names: &mut BTreeSet<String>,
        errors: &mut Vec<ValidationError>,
    ) {
        if wire.is_empty() {
            errors.push(ValidationError::InvalidValue {
                path: owner.into(),
                message: format!("{context} name cannot be empty"),
            });
        } else if !wire
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&byte))
        {
            errors.push(ValidationError::InvalidValue {
                path: owner.into(),
                message: format!("{context} name {wire:?} is not a valid HTTP field name"),
            });
        } else if !names.insert(wire.to_ascii_lowercase()) {
            errors.push(ValidationError::InvalidValue {
                path: owner.into(),
                message: format!("duplicate {context} {wire}"),
            });
        }
    }

    fn validate_inline(&self, owner: &str, ty: &TypeRef, errors: &mut Vec<ValidationError>) {
        let TypeRef::Inline(ty) = ty else { return };
        match ty {
            InlineType::Int {
                bits,
                signed,
                minimum,
                maximum,
                multiple_of,
                ..
            } => {
                if ![8, 16, 32, 64, 128].contains(bits) {
                    errors.push(ValidationError::InvalidValue {
                        path: owner.into(),
                        message: format!("integer width must be 8, 16, 32, 64, or 128, got {bits}"),
                    });
                }
                for (name, value) in [
                    ("minimum", minimum),
                    ("maximum", maximum),
                    ("multiple_of", multiple_of),
                ] {
                    if let Some(value) = value {
                        if !value.is_number() {
                            errors.push(ValidationError::InvalidValue {
                                path: owner.into(),
                                message: format!("integer {name} must be a number"),
                            });
                        } else if matches!(exact_integer(value), Exact::Fraction) {
                            errors.push(ValidationError::InvalidValue {
                                path: owner.into(),
                                message: format!(
                                    "integer {name} must be an integral value, got {value}"
                                ),
                            });
                        }
                    }
                }
                for (name, value) in [("minimum", minimum), ("maximum", maximum)] {
                    if value
                        .as_ref()
                        .is_some_and(|value| integer_below_native(value, *bits, *signed))
                    {
                        errors.push(ValidationError::InvalidValue {
                            path: owner.into(),
                            message: format!("integer {name} is below its native range"),
                        });
                    }
                    if value
                        .as_ref()
                        .is_some_and(|value| integer_above_native(value, *bits, *signed))
                    {
                        errors.push(ValidationError::InvalidValue {
                            path: owner.into(),
                            message: format!("integer {name} is above its native range"),
                        });
                    }
                }
                if let (Some(minimum), Some(maximum)) = (minimum, maximum)
                    && integer_minimum_exceeds_maximum(minimum, maximum)
                {
                    errors.push(ValidationError::InvalidValue {
                        path: owner.into(),
                        message: "integer minimum exceeds maximum".into(),
                    });
                }
                match multiple_of {
                    Some(value) if !positive_number(value) => {
                        errors.push(ValidationError::InvalidValue {
                            path: owner.into(),
                            message: "integer multiple_of must be greater than zero".into(),
                        });
                    }
                    Some(value) if matches!(exact_integer(value), Exact::Beyond) => {
                        errors.push(ValidationError::InvalidValue {
                            path: owner.into(),
                            message: format!(
                                "integer multiple_of is too large to be an exact integer, got {value}"
                            ),
                        });
                    }
                    _ => {}
                }
            }
            InlineType::Float { bits } if ![32, 64].contains(bits) => {
                errors.push(ValidationError::InvalidValue {
                    path: owner.into(),
                    message: format!("float width must be 32 or 64, got {bits}"),
                });
            }
            InlineType::String {
                min_length,
                max_length,
                pattern,
                ..
            } => {
                if min_length
                    .zip(*max_length)
                    .is_some_and(|(minimum, maximum)| minimum > maximum)
                {
                    errors.push(ValidationError::InvalidValue {
                        path: owner.into(),
                        message: "string min_length exceeds max_length".into(),
                    });
                }
                if let Some(pattern) = pattern
                    && let Err(reason) = crate::pattern::compiles(pattern)
                {
                    errors.push(ValidationError::InvalidValue {
                        path: owner.into(),
                        message: format!("string pattern {pattern:?} is not a usable regular expression because {reason}"),
                    });
                }
            }
            InlineType::Opaque { codec, ext, .. } => {
                if codec.trim().is_empty() {
                    errors.push(ValidationError::InvalidValue {
                        path: owner.into(),
                        message: "opaque codec cannot be empty".into(),
                    });
                }
                crate::ext::check(owner, ext, errors);
            }
            InlineType::Map { key, .. }
                if !matches!(key.as_ref(), TypeRef::Inline(InlineType::String { .. })) =>
            {
                errors.push(ValidationError::InvalidValue {
                    path: owner.into(),
                    message: "map keys must use an inline string type".into(),
                });
            }
            InlineType::Nullable { .. }
            | InlineType::Array { .. }
            | InlineType::Map { .. }
            | InlineType::Tuple { .. } => {}
            _ => {}
        }
    }

    pub(super) fn visit_operation_refs(
        &self,
        owner: &str,
        operation: &Operation,
        errors: &mut Vec<ValidationError>,
    ) {
        operation.visit_refs(&mut |reference| {
            self.validate_ref(owner, reference, errors);
            self.validate_inline(owner, reference, errors);
        });
    }

    pub(super) fn visit_error_refs(
        &self,
        owner: &str,
        error: &ErrorResponse,
        errors: &mut Vec<ValidationError>,
    ) {
        if let Some(body) = &error.body {
            body.visit_refs(&mut |reference| {
                self.validate_ref(owner, reference, errors);
                self.validate_inline(owner, reference, errors);
            });
        }
        for header in &error.headers {
            header.ty.visit_refs(&mut |reference| {
                self.validate_ref(owner, reference, errors);
                self.validate_inline(owner, reference, errors);
            });
        }
    }

    fn validate_ref(&self, owner: &str, reference: &TypeRef, errors: &mut Vec<ValidationError>) {
        let TypeRef::Named(named) = reference else {
            return;
        };
        let Some(definition) = self.types.get(&named.id) else {
            errors.push(ValidationError::UnknownType {
                owner: owner.into(),
                reference: named.id.clone(),
            });
            return;
        };
        if let Some(variant) = &named.variant {
            let exists = matches!(
                &definition.definition,
                TypeKind::Enum { variants, .. } if variants.iter().any(|candidate| candidate.wire == *variant)
            );
            if !exists {
                errors.push(ValidationError::InvalidValue {
                    path: owner.into(),
                    message: format!("type {} has no enum variant {variant}", named.id),
                });
            }
        }
    }
}

fn positive_number(value: &Value) -> bool {
    match exact_integer(value) {
        Exact::Integer((negative, magnitude)) => !negative && magnitude != 0,
        Exact::Beyond => true,
        Exact::Fraction => value.as_f64().is_some_and(|number| number > 0.0),
    }
}

fn integer_minimum_exceeds_maximum(minimum: &Value, maximum: &Value) -> bool {
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

fn integer_below_native(value: &Value, bits: u16, signed: bool) -> bool {
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

fn integer_above_native(value: &Value, bits: u16, signed: bool) -> bool {
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
