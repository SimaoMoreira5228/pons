use crate::media::MediaDescriptor;
use crate::operation::{ContentKind, ErrorContent};

use super::{Document, ValidationError};

#[derive(Clone, Copy)]
pub(super) enum Encoding {
    Json,
    Form,
    Multipart,
    Text,
    Bytes,
    Any,
}

impl Encoding {
    fn name(self) -> &'static str {
        match self {
            Self::Json => "json",
            Self::Form => "form",
            Self::Multipart => "multipart",
            Self::Text => "text",
            Self::Bytes => "bytes",
            Self::Any => "declared",
        }
    }

    fn names(self, media: &MediaDescriptor) -> bool {
        let base = media.base();
        let json = base == "application/json" || base.ends_with("+json");
        match self {
            Self::Json => json,
            Self::Form => base == "application/x-www-form-urlencoded",
            Self::Multipart => base.starts_with("multipart/"),
            Self::Text => base.starts_with("text/"),
            Self::Bytes => {
                !(json
                    || base == "application/x-www-form-urlencoded"
                    || base.starts_with("multipart/"))
            }
            Self::Any => true,
        }
    }
}

pub(super) fn encoding(content: ContentKind) -> Encoding {
    match content {
        ContentKind::Json | ContentKind::ProblemJson => Encoding::Json,
        ContentKind::Form => Encoding::Form,
        ContentKind::Multipart => Encoding::Multipart,
        ContentKind::Text => Encoding::Text,
        ContentKind::Bytes => Encoding::Bytes,
        ContentKind::Stream | ContentKind::Empty => Encoding::Any,
    }
}

pub(super) fn error_encoding(content: ErrorContent) -> Encoding {
    match content {
        ErrorContent::Json | ErrorContent::ProblemJson => Encoding::Json,
        ErrorContent::Text => Encoding::Text,
        ErrorContent::Empty => Encoding::Any,
    }
}

impl Document {
    pub(super) fn validate_media(
        &self,
        owner: &str,
        label: &str,
        written: Encoding,
        media: &MediaDescriptor,
        errors: &mut Vec<ValidationError>,
    ) {
        if let Err(message) = media.validate() {
            errors.push(ValidationError::InvalidValue {
                path: owner.into(),
                message: format!("{label} has {message}"),
            });
        } else if !written.names(media) {
            errors.push(ValidationError::InvalidValue {
                path: owner.into(),
                message: format!(
                    "{label} has media type {media}, which does not name the {} body the client writes",
                    written.name()
                ),
            });
        }
    }
}
