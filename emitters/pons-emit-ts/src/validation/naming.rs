use std::collections::BTreeMap;

use pons_ir::{Direction, Document, TypeKind};

use crate::naming::{pascal, unique_names};

pub(crate) fn declaration_names(
    document: &Document,
    names: &BTreeMap<String, String>,
) -> BTreeMap<(String, Option<String>), String> {
    let mut keys = Vec::new();
    let mut bases = Vec::new();
    for (id, definition) in &document.types {
        let name = &names[id];
        if let TypeKind::Enum { variants, .. } = &definition.definition {
            for variant in variants {
                keys.push((id.clone(), Some(variant.wire.clone())));
                bases.push(format!("{name}{}", pascal(&variant.ident)));
            }
        }
        keys.push((id.clone(), None));
        bases.push(name.clone());
    }
    keys.into_iter().zip(unique_names(bases, &[])).collect()
}

pub(crate) fn validator_names(
    document: &Document,
    declarations: &BTreeMap<(String, Option<String>), String>,
    reserved: &[&str],
) -> BTreeMap<(String, Option<String>), String> {
    let mut keys = Vec::new();
    let mut bases = Vec::new();
    for (key, name) in declarations {
        let input_only = document
            .types
            .get(&key.0)
            .is_some_and(|definition| definition.direction == Direction::Input);
        if input_only {
            continue;
        }
        keys.push(key.clone());
        bases.push(format!("validate{name}"));
    }
    keys.into_iter()
        .zip(unique_names(bases, reserved))
        .collect()
}
