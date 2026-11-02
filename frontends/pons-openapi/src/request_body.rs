use pons_ir::{Body, ContentKind, Direction};
use serde_json::Value;

use crate::document::{Importer, push_warning};
use crate::error::ImportError;
use crate::media;
use crate::naming::extensions;
use crate::refs::{Resolver, Site};

pub(crate) fn build<'v>(
    importer: &mut Importer,
    base: &str,
    label: &str,
    value: Option<&'v Value>,
    resolver: &'v Resolver<'_>,
) -> Result<Option<Body>, ImportError> {
    let Some(value) = value else {
        return Ok(None);
    };
    let site = Site::Operation(label);
    let Some(value) = resolver.resolve(value, site, &mut importer.diagnostics) else {
        return Ok(None);
    };
    let Some(object) = value.as_object() else {
        push_warning(
            &mut importer.diagnostics,
            format!("{site}: the request body is not an object, so the body is dropped"),
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
