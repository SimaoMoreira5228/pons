use pons_ir::{ContentKind, Diagnostic, ErrorContent, MediaDescriptor};
use serde_json::Value;

use crate::document::push_dropped;
use crate::error::ImportError;

pub(crate) struct Selected<'a> {
    pub(crate) media: &'a str,
    pub(crate) schema: Option<&'a Value>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Kind {
    pub(crate) content: ContentKind,
    pub(crate) media: Option<MediaDescriptor>,
    pub(crate) bytes: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ErrorKind {
    pub(crate) content: ErrorContent,
    pub(crate) media: Option<MediaDescriptor>,
    pub(crate) dropped: Option<String>,
}

pub(crate) fn select<'a>(
    content: Option<&'a Value>,
    operation: &str,
    subject: &str,
    diagnostics: &mut Vec<Diagnostic>,
) -> Result<Option<Selected<'a>>, ImportError> {
    let Some(content) = content else {
        return Ok(None);
    };
    let Some(object) = content.as_object() else {
        return Err(ImportError::Document(format!(
            "operation {operation}: the {subject} content is not an object"
        )));
    };
    if object.is_empty() {
        return Ok(None);
    }
    let mut keys = object.keys().map(String::as_str).collect::<Vec<_>>();
    keys.sort_by(|left, right| rank(left).cmp(&rank(right)).then_with(|| left.cmp(right)));
    let media = keys[0];
    if keys.len() > 1 {
        push_dropped(
            diagnostics,
            Some(operation),
            format!(
                "the {subject} declares several media types ({}); only {media:?} is imported",
                keys.join(", ")
            ),
        );
    }
    let schema = object[media].get("schema");
    Ok(Some(Selected { media, schema }))
}

pub(crate) fn request_kind(media: &str) -> Kind {
    match base(media).as_str() {
        "application/json" => plain(ContentKind::Json),
        "application/problem+json" => described(ContentKind::Json, media),
        "text/plain" => plain(ContentKind::Text),
        "application/octet-stream" => bytes(),
        "application/x-www-form-urlencoded" => plain(ContentKind::Form),
        "multipart/form-data" => plain(ContentKind::Multipart),
        base if base.ends_with("+json") => described(ContentKind::Json, media),
        base if base.starts_with("text/") => described(ContentKind::Text, media),
        _ => bytes_with(media),
    }
}

pub(crate) fn response_kind(media: &str) -> Kind {
    match base(media).as_str() {
        "application/json" => plain(ContentKind::Json),
        "application/problem+json" => plain(ContentKind::ProblemJson),
        "text/plain" => plain(ContentKind::Text),
        "application/octet-stream" => bytes(),
        "application/x-www-form-urlencoded" => plain(ContentKind::Form),
        "multipart/form-data" => plain(ContentKind::Multipart),
        base if base.ends_with("+json") => described(ContentKind::Json, media),
        base if base.starts_with("text/") => described(ContentKind::Text, media),
        _ => bytes_with(media),
    }
}

pub(crate) fn error_kind(media: &str) -> ErrorKind {
    match base(media).as_str() {
        "application/json" => ErrorKind {
            content: ErrorContent::Json,
            media: None,
            dropped: None,
        },
        "application/problem+json" => ErrorKind {
            content: ErrorContent::ProblemJson,
            media: None,
            dropped: None,
        },
        "text/plain" => ErrorKind {
            content: ErrorContent::Text,
            media: None,
            dropped: None,
        },
        base if base.ends_with("+json") => ErrorKind {
            content: ErrorContent::Json,
            media: Some(MediaDescriptor::from_declared(media)),
            dropped: None,
        },
        base if base.starts_with("text/") => ErrorKind {
            content: ErrorContent::Text,
            media: Some(MediaDescriptor::from_declared(media)),
            dropped: None,
        },
        _ => ErrorKind {
            content: ErrorContent::Json,
            media: None,
            dropped: Some(format!(
                "the error body media type {media:?} has no Pons error encoding and is imported as JSON"
            )),
        },
    }
}

fn plain(content: ContentKind) -> Kind {
    Kind {
        content,
        media: None,
        bytes: false,
    }
}

fn described(content: ContentKind, media: &str) -> Kind {
    Kind {
        content,
        media: Some(MediaDescriptor::from_declared(media)),
        bytes: false,
    }
}

fn bytes() -> Kind {
    Kind {
        content: ContentKind::Bytes,
        media: None,
        bytes: true,
    }
}

fn bytes_with(media: &str) -> Kind {
    Kind {
        content: ContentKind::Bytes,
        media: Some(MediaDescriptor::from_declared(media)),
        bytes: true,
    }
}

fn base(media: &str) -> String {
    media
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase()
}

fn rank(media: &str) -> (u8, String) {
    let base = base(media);
    let rank = match base.as_str() {
        "application/json" => 0,
        "application/problem+json" => 1,
        "text/plain" => 2,
        "application/octet-stream" => 3,
        _ => 4,
    };
    (rank, base)
}
