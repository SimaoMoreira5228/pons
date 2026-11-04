use pons_ir::Direction;
use specta::datatype::{Attributes, Field, Variant};
use specta::markers::{
    SERDE_CONTAINER_CONTENT, SERDE_CONTAINER_DENY_UNKNOWN_FIELDS,
    SERDE_CONTAINER_RENAME_ALL_DESERIALIZE, SERDE_CONTAINER_RENAME_ALL_FIELDS_DESERIALIZE,
    SERDE_CONTAINER_RENAME_ALL_FIELDS_SERIALIZE, SERDE_CONTAINER_RENAME_ALL_SERIALIZE,
    SERDE_CONTAINER_TAG, SERDE_CONTAINER_UNTAGGED, SERDE_FIELD_DEFAULT,
    SERDE_FIELD_RENAME_DESERIALIZE, SERDE_FIELD_RENAME_SERIALIZE, SERDE_FIELD_SKIP_DESERIALIZING,
    SERDE_FIELD_SKIP_SERIALIZING, SERDE_FIELD_SKIP_SERIALIZING_IF,
    SERDE_VARIANT_RENAME_ALL_DESERIALIZE, SERDE_VARIANT_RENAME_ALL_SERIALIZE,
    SERDE_VARIANT_RENAME_DESERIALIZE, SERDE_VARIANT_RENAME_SERIALIZE,
    SERDE_VARIANT_SKIP_DESERIALIZING, SERDE_VARIANT_SKIP_SERIALIZING,
};

fn string<'a>(attributes: &'a Attributes, key: &str) -> Option<&'a str> {
    attributes.get_named_as::<String>(key).map(String::as_str)
}

fn flag(attributes: &Attributes, key: &str) -> bool {
    attributes
        .get_named_as::<bool>(key)
        .copied()
        .unwrap_or(false)
}

fn directional<'a>(
    attributes: &'a Attributes,
    input: &str,
    output: &str,
    direction: Direction,
) -> Option<&'a str> {
    match direction {
        Direction::Input => string(attributes, input),
        Direction::Output => string(attributes, output),
        Direction::Both => string(attributes, output).or_else(|| string(attributes, input)),
    }
}

pub(crate) fn tag(attributes: &Attributes) -> Option<&str> {
    string(attributes, SERDE_CONTAINER_TAG)
}

pub(crate) fn content(attributes: &Attributes) -> Option<&str> {
    string(attributes, SERDE_CONTAINER_CONTENT)
}

pub(crate) fn untagged(attributes: &Attributes) -> bool {
    flag(attributes, SERDE_CONTAINER_UNTAGGED)
}

pub(crate) fn deny_unknown_fields(attributes: &Attributes) -> bool {
    flag(attributes, SERDE_CONTAINER_DENY_UNKNOWN_FIELDS)
}

pub(crate) fn container_rename_all(attributes: &Attributes, direction: Direction) -> Option<&str> {
    directional(
        attributes,
        SERDE_CONTAINER_RENAME_ALL_DESERIALIZE,
        SERDE_CONTAINER_RENAME_ALL_SERIALIZE,
        direction,
    )
}

pub(crate) fn fields_rename_all(attributes: &Attributes, direction: Direction) -> Option<&str> {
    directional(
        attributes,
        SERDE_CONTAINER_RENAME_ALL_FIELDS_DESERIALIZE,
        SERDE_CONTAINER_RENAME_ALL_FIELDS_SERIALIZE,
        direction,
    )
}

pub(crate) fn variant_rename_all(attributes: &Attributes, direction: Direction) -> Option<&str> {
    directional(
        attributes,
        SERDE_VARIANT_RENAME_ALL_DESERIALIZE,
        SERDE_VARIANT_RENAME_ALL_SERIALIZE,
        direction,
    )
}

pub(crate) fn field_rename(attributes: &Attributes, direction: Direction) -> Option<&str> {
    directional(
        attributes,
        SERDE_FIELD_RENAME_DESERIALIZE,
        SERDE_FIELD_RENAME_SERIALIZE,
        direction,
    )
}

pub(crate) fn variant_rename(attributes: &Attributes, direction: Direction) -> Option<&str> {
    directional(
        attributes,
        SERDE_VARIANT_RENAME_DESERIALIZE,
        SERDE_VARIANT_RENAME_SERIALIZE,
        direction,
    )
}

pub(crate) fn field_optional(field: &Field, direction: Direction) -> bool {
    if field.optional {
        return true;
    }
    let default = flag(&field.attributes, SERDE_FIELD_DEFAULT);
    let skipped = flag(&field.attributes, SERDE_FIELD_SKIP_SERIALIZING_IF);
    match direction {
        Direction::Input => default,
        Direction::Output => skipped,
        Direction::Both => default || skipped,
    }
}

pub(crate) fn field_skipped(field: &Field, direction: Direction) -> bool {
    if field.ty.is_none() {
        return true;
    }
    let serializing = flag(&field.attributes, SERDE_FIELD_SKIP_SERIALIZING);
    let deserializing = flag(&field.attributes, SERDE_FIELD_SKIP_DESERIALIZING);
    match direction {
        Direction::Input => deserializing,
        Direction::Output => serializing,
        Direction::Both => serializing && deserializing,
    }
}

pub(crate) fn variant_skipped(variant: &Variant, direction: Direction) -> bool {
    if variant.skip {
        return true;
    }
    let serializing = flag(&variant.attributes, SERDE_VARIANT_SKIP_SERIALIZING);
    let deserializing = flag(&variant.attributes, SERDE_VARIANT_SKIP_DESERIALIZING);
    match direction {
        Direction::Input => deserializing,
        Direction::Output => serializing,
        Direction::Both => serializing && deserializing,
    }
}
