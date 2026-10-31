use pons_ir::{Body, ContentKind, Direction};
use serde_json::Value;

use crate::components::{Components, Section};
use crate::document::Importer;
use crate::error::ImportError;
use crate::media;
use crate::naming::extensions;

pub(crate) fn build(
    importer: &mut Importer<'_>,
    base: &str,
    label: &str,
    value: Option<&Value>,
    components: &Components,
) -> Result<Option<Body>, ImportError> {
    let Some(value) = value else {
        return Ok(None);
    };
    let Some(object) = components
        .resolve(value, Section::RequestBodies)
        .and_then(Value::as_object)
    else {
        importer.note_dropped(
            Some(label),
            "the request body reference cannot be resolved, so the body is dropped",
        );
        return Ok(None);
    };
    let Some(selected) = media::select(
        object.get("content"),
        label,
        "request body",
        &mut importer.diagnostics,
    )?
    else {
        return Ok(None);
    };
    let kind = media::request_kind(selected.media);
    let ty = importer.types.payload(
        &format!("{base}::request"),
        &format!("{label} request"),
        selected.schema,
        kind.bytes,
        kind.content == ContentKind::Text,
        Direction::Input,
    );
    Ok(Some(Body {
        ty,
        content: kind.content,
        required: object
            .get("required")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        media: kind.media,
        ext: extensions(object),
    }))
}
