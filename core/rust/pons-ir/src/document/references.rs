use std::collections::BTreeSet;

use super::constraints::{
    integer_above_native, integer_below_native, integer_minimum_exceeds_maximum, positive_number,
};
use super::{Document, Exact, ValidationError, exact_integer};
use crate::operation::{ErrorResponse, Operation};
use crate::types::{InlineType, OpaqueWire, TypeKind, TypeRef};

impl Document {
    pub(super) fn struct_field_type<'a>(
        &'a self,
        ty: &'a TypeRef,
        wire: &str,
        visited: &mut BTreeSet<String>,
    ) -> Option<&'a TypeRef> {
        let TypeRef::Named(named) = ty else {
            return None;
        };
        if named.variant.is_some() || !visited.insert(named.id.clone()) {
            return None;
        }
        let found = match self.types.get(&named.id).map(|found| &found.definition) {
            Some(TypeKind::Struct {
                fields, flattened, ..
            }) => match fields.iter().find(|field| field.wire == wire) {
                Some(field) => Some(&field.ty),
                None => flattened
                    .iter()
                    .find_map(|member| self.struct_field_type(member, wire, visited)),
            },
            Some(TypeKind::Newtype { inner }) => self.struct_field_type(inner, wire, visited),
            _ => None,
        };
        visited.remove(&named.id);
        found
    }

    pub(super) fn is_integer_type(&self, ty: &TypeRef, visited: &mut BTreeSet<String>) -> bool {
        match ty {
            TypeRef::Inline(InlineType::Int { .. }) => true,
            TypeRef::Named(named) => {
                if !visited.insert(named.id.clone()) {
                    return false;
                }
                let result = self.types.get(&named.id).is_some_and(|definition| {
                    matches!(
                        &definition.definition,
                        TypeKind::Newtype { inner } if self.is_integer_type(inner, visited)
                    )
                });
                visited.remove(&named.id);
                result
            }
            _ => false,
        }
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

    pub(super) fn validate_inline(
        &self,
        owner: &str,
        ty: &TypeRef,
        errors: &mut Vec<ValidationError>,
    ) {
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
            InlineType::Float {
                bits,
                minimum,
                maximum,
                exclusive_minimum,
                exclusive_maximum,
                multiple_of,
            } => {
                if ![32, 64].contains(bits) {
                    errors.push(ValidationError::InvalidValue {
                        path: owner.into(),
                        message: format!("float width must be 32 or 64, got {bits}"),
                    });
                }
                for (name, value) in [
                    ("minimum", minimum),
                    ("maximum", maximum),
                    ("exclusive_minimum", exclusive_minimum),
                    ("exclusive_maximum", exclusive_maximum),
                    ("multiple_of", multiple_of),
                ] {
                    let Some(value) = value else { continue };
                    match value.as_f64() {
                        None => errors.push(ValidationError::InvalidValue {
                            path: owner.into(),
                            message: format!("float {name} must be a number"),
                        }),
                        Some(number) if !number.is_finite() => {
                            errors.push(ValidationError::InvalidValue {
                                path: owner.into(),
                                message: format!("float {name} must be finite"),
                            })
                        }
                        Some(_) => {}
                    }
                }
                if minimum.is_some() && exclusive_minimum.is_some() {
                    errors.push(ValidationError::InvalidValue {
                        path: owner.into(),
                        message: "float minimum and exclusive_minimum cannot both apply".into(),
                    });
                }
                if maximum.is_some() && exclusive_maximum.is_some() {
                    errors.push(ValidationError::InvalidValue {
                        path: owner.into(),
                        message: "float maximum and exclusive_maximum cannot both apply".into(),
                    });
                }
                if let Some(value) = multiple_of
                    && value.is_number()
                    && !positive_number(value)
                {
                    errors.push(ValidationError::InvalidValue {
                        path: owner.into(),
                        message: "float multiple_of must be greater than zero".into(),
                    });
                }
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

    pub(super) fn validate_ref(
        &self,
        owner: &str,
        reference: &TypeRef,
        errors: &mut Vec<ValidationError>,
    ) {
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
