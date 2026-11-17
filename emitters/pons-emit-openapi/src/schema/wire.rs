use pons_ir::{
    ByteEncoding, DecimalWire, EnumRepr, InlineType, IntegerWire, TypeKind, TypeRef, VariantPayload,
};

use crate::{Builder, EmitError};

impl<'a> Builder<'a> {
    pub(crate) fn wire_form(
        &self,
        ty: &TypeRef,
        place: Place,
    ) -> Result<Option<WireForm>, EmitError> {
        self.wire_form_of(ty, place, &mut Vec::new())
    }

    fn wire_form_of(
        &self,
        ty: &TypeRef,
        place: Place,
        visiting: &mut Vec<String>,
    ) -> Result<Option<WireForm>, EmitError> {
        let inline = match ty {
            TypeRef::Inline(inline) => inline,
            TypeRef::Named(named) => {
                let definition = self
                    .document
                    .types
                    .get(&named.id)
                    .ok_or_else(|| EmitError(format!("unknown type {}", named.id)))?;
                if let Some(wire) = &named.variant {
                    let TypeKind::Enum { variants, .. } = &definition.definition else {
                        return Err(EmitError(format!("type {} is not an enum", named.id)));
                    };
                    let Some(variant) = variants.iter().find(|one| &one.wire == wire) else {
                        return Err(EmitError(format!("unknown variant {wire}")));
                    };
                    return match &variant.payload {
                        VariantPayload::Unit => Ok(Some(WireForm::Text)),
                        VariantPayload::Newtype { ty } => self.wire_form_of(ty, place, visiting),
                        VariantPayload::Struct { .. } | VariantPayload::Tuple { .. } => {
                            Ok(Some(WireForm::Json))
                        }
                    };
                }
                if visiting.contains(&named.id) {
                    return Err(EmitError(format!(
                        "the type {} cannot be resolved because newtypes form a cycle",
                        named.id
                    )));
                }
                visiting.push(named.id.clone());
                let result = match &definition.definition {
                    TypeKind::Newtype { inner } => self.wire_form_of(inner, place, visiting),
                    TypeKind::Struct { .. } => Ok(Some(WireForm::Json)),
                    TypeKind::Enum { repr, .. } => Ok(match repr {
                        EnumRepr::String => Some(WireForm::Text),

                        EnumRepr::External => None,
                        EnumRepr::Internal { .. }
                        | EnumRepr::Adjacent { .. }
                        | EnumRepr::Untagged => Some(WireForm::Json),
                    }),
                };
                visiting.pop();
                return result;
            }
        };
        Ok(Some(match inline {
            InlineType::String { .. }
            | InlineType::Int {
                wire: IntegerWire::String,
                ..
            }
            | InlineType::Decimal {
                wire: DecimalWire::String,
            }
            | InlineType::Bytes {
                encoding: ByteEncoding::Base64 | ByteEncoding::Base64url | ByteEncoding::Hex,
            } => WireForm::Text,

            InlineType::Bool
            | InlineType::Int {
                wire: IntegerWire::Number,
                ..
            }
            | InlineType::Float { .. }
            | InlineType::Decimal {
                wire: DecimalWire::Number,
            }
            | InlineType::Any => match place {
                Place::Header => WireForm::Text,
                Place::Part { .. } => WireForm::Json,
            },

            InlineType::Opaque { .. } | InlineType::Null => return Ok(None),
            InlineType::Nullable { inner } => {
                return match place {
                    Place::Part { required: true } => Ok(None),
                    place => self.wire_form_of(inner, place, visiting),
                };
            }
            InlineType::Array { .. }
            | InlineType::Map { .. }
            | InlineType::Tuple { .. }
            | InlineType::Struct { .. } => WireForm::Json,

            InlineType::Bytes {
                encoding: ByteEncoding::Array,
            } => WireForm::Json,
        }))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WireForm {
    Text,
    Json,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Place {
    Header,
    Part { required: bool },
}

#[cfg(test)]
mod tests {
    use pons_ir::{Direction, NamedRef, TypeDef, TypeKind, TypeRef};

    fn fixture() -> pons_ir::Document {
        serde_json::from_str(include_str!("../../../../spec/fixtures/login.json")).unwrap()
    }

    fn named(id: &str) -> TypeRef {
        TypeRef::Named(NamedRef {
            id: id.into(),
            variant: None,
        })
    }

    fn newtype(name: &str, inner: TypeRef) -> TypeDef {
        TypeDef {
            name: vec![name.into()],
            direction: Direction::Both,
            definition: TypeKind::Newtype { inner },
            origin: None,
            doc: None,
            diagnostics: Vec::new(),
            ext: Default::default(),
        }
    }

    #[test]
    fn rejects_a_self_referential_newtype() {
        let mut document = fixture();
        document
            .types
            .insert("Loop".into(), newtype("loop", named("Loop")));
        assert!(
            crate::emit(&document)
                .unwrap_err()
                .0
                .contains("the type Loop cannot be resolved because newtypes form a cycle")
        );
    }

    #[test]
    fn rejects_a_cycle_of_newtypes() {
        let mut document = fixture();
        document.types.insert("A".into(), newtype("a", named("B")));
        document.types.insert("B".into(), newtype("b", named("A")));
        let error = crate::emit(&document).unwrap_err();
        for member in ["A", "B"] {
            let expected =
                format!("the type {member} cannot be resolved because newtypes form a cycle");
            if error.0.contains(&expected) {
                return;
            }
        }
        panic!("{}", error.0);
    }
}
