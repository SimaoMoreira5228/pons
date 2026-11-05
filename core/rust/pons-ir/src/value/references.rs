use serde_json::Value;

use super::Validator;
use super::composites::Member;
use crate::violation::Violation;
use crate::{InlineType, NamedRef, TypeKind, TypeRef};

impl<'a> Validator<'a> {
    pub(super) fn named(&mut self, named: &'a NamedRef, path: &str, value: &Value) {
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

    pub(super) fn flattened(
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
            TypeKind::Struct { fields, .. } => members.extend(fields.iter().map(Member::new)),
            TypeKind::Enum { .. } => {}
        }
    }

    pub(super) fn openness_value(
        &self,
        ty: &'a TypeRef,
        newtypes: &mut Vec<String>,
    ) -> &'a TypeRef {
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
}
