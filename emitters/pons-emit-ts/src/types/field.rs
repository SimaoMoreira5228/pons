use std::collections::{BTreeMap, BTreeSet};

use pons_ir::{Access, Direction, Document, Field};

use crate::naming::camel;
use crate::render::string;
use crate::{EmitError, FieldCase, TsOptions};

use super::inline::type_ref_with_namespace;

pub(super) fn field_is_visible(field: &Field, direction: Direction) -> bool {
    !matches!(
        (direction, field.access),
        (Direction::Input, Some(Access::ReadOnly)) | (Direction::Output, Some(Access::WriteOnly))
    )
}

pub(super) fn field_type(
    field: &Field,
    direction: Direction,
    document: &Document,
    names: &BTreeMap<String, String>,
    options: &TsOptions,
    qualify: bool,
    narrowed: &mut BTreeSet<String>,
) -> Result<String, EmitError> {
    let key = string(&member_name(field, options));
    let access_specific = matches!(
        (direction, field.access),
        (Direction::Both, Some(Access::ReadOnly | Access::WriteOnly))
    );
    let presence = if field.presence == pons_ir::Presence::Optional || access_specific {
        "?"
    } else {
        ""
    };
    Ok(format!(
        "{key}{presence}: {}",
        type_ref_with_namespace(&field.ty, document, names, options, qualify, narrowed)?
    ))
}

fn member_name(field: &Field, options: &TsOptions) -> String {
    match options.field_case {
        FieldCase::Wire => field.wire.clone(),
        FieldCase::Camel => camel(&field.ident),
    }
}
