use std::collections::BTreeSet;

use pons_ir::{Document, InlineType, TypeDef, TypeKind, TypeRef};

use super::Walk;

pub(super) enum Flattened<'a> {
    Struct(&'a TypeDef),
    Opaque,
    Other,
}

impl<'a> Walk<'a> {
    pub(super) fn flattened(&self, members: &[TypeRef]) -> Vec<Flattened<'a>> {
        let mut found = Vec::new();
        let mut seen = BTreeSet::new();
        for member in members {
            let key = match member {
                TypeRef::Named(named) => named.id.clone(),
                TypeRef::Inline(_) => String::new(),
            };
            if seen.insert(key) {
                found.push(self.flattened_member(member));
            }
        }
        found
    }

    pub(super) fn inlined(&self, reference: &TypeRef) -> Option<&'a TypeDef> {
        match self.flattened_member(reference) {
            Flattened::Struct(definition) => Some(definition),
            _ => None,
        }
    }

    fn flattened_member(&self, reference: &TypeRef) -> Flattened<'a> {
        let document = self.document;
        let mut current = reference;
        let mut seen = BTreeSet::new();
        loop {
            match current {
                TypeRef::Inline(InlineType::Opaque { .. }) => return Flattened::Opaque,
                TypeRef::Inline(_) => return Flattened::Other,
                TypeRef::Named(named) => {
                    if named.variant.is_some() || !seen.insert(named.id.clone()) {
                        return Flattened::Other;
                    }
                    let Some(definition) = document.types.get(&named.id) else {
                        return Flattened::Other;
                    };
                    match &definition.definition {
                        TypeKind::Newtype { inner } => current = inner,
                        TypeKind::Struct { .. } => return Flattened::Struct(definition),
                        TypeKind::Enum { .. } => return Flattened::Other,
                    }
                }
            }
        }
    }
}

pub(super) fn openness_value<'a>(ty: &'a TypeRef, document: &'a Document) -> &'a TypeRef {
    let mut current = ty;
    let mut visited = BTreeSet::new();
    loop {
        match current {
            TypeRef::Inline(InlineType::Map { value, .. }) => return value,
            TypeRef::Named(named) if visited.insert(named.id.clone()) => {
                let Some(TypeKind::Newtype { inner }) = document
                    .types
                    .get(&named.id)
                    .map(|definition| &definition.definition)
                else {
                    return current;
                };
                current = inner;
            }
            _ => return current,
        }
    }
}
