use std::collections::{BTreeMap, BTreeSet};

use pons_ir::{Document, TypeRef};

use crate::render::jsdoc;
use crate::{EmitError, TsOptions};

use self::definition::{definition_type, newtype_brand};
use self::inline::type_ref_with_namespace;
use self::metadata::type_definitions;

mod definition;
mod field;
mod flatten;
mod inline;
mod metadata;

#[cfg(test)]
mod tests;

pub(super) fn emit(
    document: &Document,
    names: &BTreeMap<String, String>,
    options: &TsOptions,
    date_only: bool,
) -> Result<String, EmitError> {
    let mut output = String::new();
    if date_only {
        output.push_str(
            "declare const brandDateOnly: unique symbol;\n/** A calendar day with no timezone, spelled the way the wire spells it. A `date-time` is an instant and is carried as a `Date`; a `date` is a day, which no `Date` can hold. */\nexport type DateOnly = string & { readonly [brandDateOnly]: never };\n\n",
        );
    }
    for (id, definition) in &document.types {
        let name = &names[id];
        let comment = jsdoc(definition.doc.as_ref());
        if let Some(brand) = newtype_brand(definition, name, options) {
            output.push_str(&brand);
        }
        if !comment.is_empty() {
            output.push_str(&comment);
            output.push('\n');
        }
        output.push_str(&format!(
            "export type {name} = {};\n\n",
            definition_type(id, definition, document, names, options)?
        ));
    }
    output.push_str(&type_definitions(document, options)?);
    Ok(output)
}

pub(super) fn type_ref(
    reference: &TypeRef,
    document: &Document,
    names: &BTreeMap<String, String>,
    options: &TsOptions,
) -> Result<String, EmitError> {
    type_ref_with_namespace(
        reference,
        document,
        names,
        options,
        true,
        &mut BTreeSet::new(),
    )
}
