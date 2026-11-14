use std::collections::BTreeSet;

use crate::operation::{ContentKind, Method, Operation, ResponseStatus, StreamKind};

use super::media::encoding;
use super::status::status_forbids_body;
use super::{Document, ValidationError};

impl Document {
    pub(super) fn validate_responses(
        &self,
        owner: &str,
        operation: &Operation,
        errors: &mut Vec<ValidationError>,
    ) {
        let mut response_statuses = BTreeSet::new();
        let mut stream_responses = 0;
        for response in &operation.responses {
            match response.status {
                ResponseStatus::Exact(status) if !(100..=599).contains(&status) => {
                    errors.push(ValidationError::InvalidValue {
                        path: owner.to_owned(),
                        message: format!("invalid response status {status}"),
                    });
                }
                ResponseStatus::Class(class) if !(1..=5).contains(&class) => {
                    errors.push(ValidationError::InvalidValue {
                        path: owner.to_owned(),
                        message: format!("invalid response status class {class}xx"),
                    });
                }
                _ => {}
            }
            if !response_statuses.insert(response.status) {
                errors.push(ValidationError::InvalidValue {
                    path: owner.to_owned(),
                    message: format!("duplicate response status {}", response.status),
                });
            }
            if response.content == ContentKind::Empty && response.media.is_some() {
                errors.push(ValidationError::InvalidValue {
                    path: owner.to_owned(),
                    message: format!(
                        "empty response {} cannot declare a media type",
                        response.status
                    ),
                });
            }
            if let Some(media) = &response.media {
                self.validate_media(
                    owner,
                    &format!("response {} body", response.status),
                    encoding(response.content),
                    media,
                    errors,
                );
            }
            if response.content == ContentKind::Stream
                && let Some(media) = &response.media
                && let Some(content_type) = operation
                    .stream
                    .as_ref()
                    .and_then(|stream| stream.kind.media_type())
                && !crate::media::media_type_matches(&media.to_string(), content_type)
            {
                errors.push(ValidationError::InvalidValue {
                    path: owner.to_owned(),
                    message: format!(
                        "response media type {media} does not match stream kind media type {content_type:?}"
                    ),
                });
            }
            let content_matches_body = match response.content {
                ContentKind::Empty | ContentKind::Stream => response.body.is_none(),
                _ => response.body.is_some(),
            };
            if !content_matches_body {
                errors.push(ValidationError::InvalidValue {
                    path: owner.to_owned(),
                    message: format!(
                        "response {} content {:?} does not match its body",
                        response.status, response.content
                    ),
                });
            }
            if (operation.method == Method::Head
                || matches!(response.status, ResponseStatus::Class(1))
                || matches!(response.status, ResponseStatus::Exact(status) if status_forbids_body(status)))
                && (response.content != ContentKind::Empty || response.body.is_some())
            {
                errors.push(ValidationError::InvalidValue {
                    path: owner.to_owned(),
                    message: format!("response {} cannot carry a body", response.status),
                });
            }
            if response.content == ContentKind::Stream
                || (operation
                    .stream
                    .as_ref()
                    .is_some_and(|stream| matches!(stream.kind, StreamKind::Websocket { .. }))
                    && response.status == ResponseStatus::Exact(101)
                    && response.content == ContentKind::Empty)
            {
                stream_responses += 1;
            }
            if response.content == ContentKind::Bytes
                && let Some(body) = &response.body
                && !self.is_byte_type(body, &mut BTreeSet::new())
            {
                errors.push(ValidationError::InvalidValue {
                    path: owner.to_owned(),
                    message: format!(
                        "raw byte response bodies must use a byte type, byte newtype, nullable bytes, or a bytes opaque (response {})",
                        response.status
                    ),
                });
            }
            let mut response_header_names = BTreeSet::new();
            for header in &response.headers {
                self.validate_header_name(
                    owner,
                    &format!("response {} header", response.status),
                    &header.wire,
                    &mut response_header_names,
                    errors,
                );
            }
        }
        if response_statuses.contains(&ResponseStatus::Default)
            && response_statuses.contains(&ResponseStatus::Unknown)
        {
            errors.push(ValidationError::InvalidValue {
                path: owner.to_owned(),
                message: format!("duplicate response status {}", ResponseStatus::Default),
            });
        }
        let stream_descriptions = usize::from(operation.stream.is_some());
        if stream_responses != stream_descriptions {
            errors.push(ValidationError::InvalidValue {
                path: owner.to_owned(),
                message: format!(
                    "operation has {stream_descriptions} stream descriptions and {stream_responses} stream responses"
                ),
            });
        }
    }
}
