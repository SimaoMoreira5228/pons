use pons_ir::{
    ContentKind, Direction, Doc, ErrorContent, ErrorResponse, ErrorStatus, Method, Name, Presence,
    Response, ResponseHeader, ResponseStatus,
};
use serde_json::Value;

use crate::components::{Components, Section};
use crate::document::{Importer, push_dropped};
use crate::error::ImportError;
use crate::media;
use crate::naming::{extensions, slug, words};

#[derive(Clone, Copy)]
enum StatusKey {
    Exact(u16),
    Class(u8),
    Default,
}

pub(crate) fn build(
    importer: &mut Importer,
    base: &str,
    label: &str,
    method: Method,
    value: Option<&Value>,
    components: &Components,
) -> Result<(Vec<Response>, Vec<ErrorResponse>), ImportError> {
    let Some(value) = value else {
        return Ok((Vec::new(), Vec::new()));
    };
    let Some(object) = value.as_object() else {
        return Err(ImportError::Document(format!(
            "operation {label}: the responses value is not an object"
        )));
    };
    let has_success = object.keys().any(|key| {
        matches!(
            status_key(key),
            Some(StatusKey::Exact(200..=299) | StatusKey::Class(2))
        )
    });
    let success_count = object
        .keys()
        .filter(|key| match status_key(key) {
            Some(StatusKey::Exact(status)) => (200..300).contains(&status),
            Some(StatusKey::Class(2)) => true,
            Some(StatusKey::Default) => !has_success,
            _ => false,
        })
        .count();

    let mut responses = Vec::new();
    let mut errors = Vec::new();
    for (key, value) in object {
        let Some(status) = status_key(key) else {
            push_dropped(
                &mut importer.diagnostics,
                Some(label),
                format!("the response key {key:?} is not a status and is dropped"),
            );
            continue;
        };
        let Some(response) = components
            .resolve(value, Section::Responses)
            .and_then(Value::as_object)
        else {
            push_dropped(
                &mut importer.diagnostics,
                Some(label),
                format!("the {key} response reference cannot be resolved and is dropped"),
            );
            continue;
        };
        if let Some(links) = response.get("links").and_then(Value::as_object)
            && !links.is_empty()
        {
            push_dropped(
                &mut importer.diagnostics,
                Some(label),
                format!(
                    "the {key} response links are dropped because Pons has no runtime link representation"
                ),
            );
        }
        let success = match status {
            StatusKey::Exact(status) if (200..300).contains(&status) => true,
            StatusKey::Exact(status) if (400..600).contains(&status) => false,
            StatusKey::Class(2) => true,
            StatusKey::Class(4 | 5) => false,
            StatusKey::Default => !has_success,
            StatusKey::Exact(status) => {
                push_dropped(
                    &mut importer.diagnostics,
                    Some(label),
                    format!(
                        "the {status} response is dropped because Pons imports 2xx, 4xx, and 5xx responses"
                    ),
                );
                continue;
            }
            StatusKey::Class(class) => {
                push_dropped(
                    &mut importer.diagnostics,
                    Some(label),
                    format!(
                        "the {class}xx response is dropped because Pons imports 2xx, 4xx, and 5xx responses"
                    ),
                );
                continue;
            }
        };
        let forbidden = method == Method::Head
            || matches!(
                status,
                StatusKey::Exact(204 | 205 | 304) | StatusKey::Class(1) | StatusKey::Class(3)
            );
        let headers = headers(
            importer,
            &format!(
                "{base}::{}",
                if success {
                    format!("response-{}-header", slug(key))
                } else {
                    format!("error-{}-header", slug(key))
                }
            ),
            label,
            key,
            response.get("headers"),
        )?;
        let selected = if forbidden {
            if response.get("content").is_some() {
                push_dropped(
                    &mut importer.diagnostics,
                    Some(label),
                    format!("the {key} response cannot carry a body, so its content is dropped"),
                );
            }
            None
        } else {
            media::select(
                response.get("content"),
                label,
                &format!("{key} response body"),
                &mut importer.diagnostics,
            )?
        };

        if success {
            let response_status = match status {
                StatusKey::Exact(status) => ResponseStatus::Exact(status),
                StatusKey::Class(class) => ResponseStatus::Class(class),
                StatusKey::Default => ResponseStatus::Default,
            };
            let (content, media_type, body) = match selected {
                Some(selected) => {
                    let kind = media::response_kind(selected.media);
                    let id = if success_count == 1 {
                        format!("{base}::response")
                    } else {
                        format!("{base}::response-{}", slug(key))
                    };
                    (
                        kind.content,
                        kind.media,
                        Some(importer.types.payload(
                            &id,
                            &format!("{label} response"),
                            selected.schema,
                            kind.bytes,
                            kind.content == ContentKind::Text,
                            Direction::Output,
                        )),
                    )
                }
                None => (ContentKind::Empty, None, None),
            };
            responses.push(Response {
                status: response_status,
                content,
                media: media_type,
                body,
                headers,
                ext: extensions(response),
            });
        } else {
            let error_status = match status {
                StatusKey::Exact(status) => ErrorStatus::Exact(status),
                StatusKey::Class(class) => ErrorStatus::Class(class),
                StatusKey::Default => ErrorStatus::Default,
            };
            let (content, media_type, body) = match selected {
                Some(selected) => {
                    let kind = media::error_kind(selected.media);
                    if let Some(message) = kind.dropped {
                        push_dropped(&mut importer.diagnostics, Some(label), message);
                    }
                    (
                        kind.content,
                        kind.media,
                        Some(importer.types.payload(
                            &format!("{base}::error-{}", slug(key)),
                            &format!("{label} {key} error"),
                            selected.schema,
                            false,
                            kind.content == ErrorContent::Text,
                            Direction::Output,
                        )),
                    )
                }
                None => (ErrorContent::Empty, None, None),
            };
            let name = error_name(label, &error_status);
            errors.push(ErrorResponse {
                status: error_status,
                name,
                content,
                media: media_type,
                body,
                headers,
                retryable: None,
                doc: response
                    .get("description")
                    .and_then(Value::as_str)
                    .map(|description| Doc {
                        summary: None,
                        description: Some(description.to_owned()),
                        deprecated: None,
                        tags: Vec::new(),
                    }),
                ext: extensions(response),
            });
        }
    }
    Ok((responses, errors))
}

fn status_key(key: &str) -> Option<StatusKey> {
    if key == "default" {
        return Some(StatusKey::Default);
    }
    if let Some(class) = key.strip_suffix("XX").or_else(|| key.strip_suffix("xx")) {
        let class: u8 = class.parse().ok()?;
        return (1..=5).contains(&class).then_some(StatusKey::Class(class));
    }
    let status: u16 = key.parse().ok()?;
    (100..=599)
        .contains(&status)
        .then_some(StatusKey::Exact(status))
}

fn headers(
    importer: &mut Importer,
    prefix: &str,
    label: &str,
    status: &str,
    value: Option<&Value>,
) -> Result<Vec<ResponseHeader>, ImportError> {
    let Some(headers) = value else {
        return Ok(Vec::new());
    };
    let Some(headers) = headers.as_object() else {
        return Err(ImportError::Document(format!(
            "operation {label}: the {status} response headers are not an object"
        )));
    };
    let mut result = Vec::new();
    for (name, header) in headers {
        let Some(header) = header.as_object() else {
            push_dropped(
                &mut importer.diagnostics,
                Some(label),
                format!("the {status} response header {name:?} is not an object and is dropped"),
            );
            continue;
        };
        if name.eq_ignore_ascii_case("content-type") {
            push_dropped(
                &mut importer.diagnostics,
                Some(label),
                format!(
                    "the {status} response header {name:?} is defined by the response content and is dropped"
                ),
            );
            continue;
        }
        if header.contains_key("content") {
            push_dropped(
                &mut importer.diagnostics,
                Some(label),
                format!(
                    "the {status} response header {name:?} uses content instead of a schema, so it is dropped"
                ),
            );
            continue;
        }
        let schema = header.get("schema").cloned().unwrap_or(Value::Bool(true));
        let ty = importer.types.schema_type(
            &format!("{prefix}-{}", slug(name)),
            name,
            &schema,
            Direction::Output,
        );
        result.push(ResponseHeader {
            wire: name.clone(),
            ty,
            presence: Presence::Optional,
            ext: Default::default(),
        });
    }
    Ok(result)
}

fn error_name(label: &str, status: &ErrorStatus) -> Name {
    let mut name = label
        .rsplit('.')
        .next()
        .map(words)
        .unwrap_or_else(|| vec!["operation".to_owned()]);
    name.push("error".to_owned());
    name.push(match status {
        ErrorStatus::Exact(status) => reason(*status).to_owned(),
        ErrorStatus::Class(class) => format!("{class}xx"),
        ErrorStatus::Default => "default".to_owned(),
    });
    name
}

fn reason(status: u16) -> &'static str {
    match status {
        400 => "bad_request",
        401 => "unauthorized",
        403 => "forbidden",
        404 => "not_found",
        405 => "method_not_allowed",
        406 => "not_acceptable",
        408 => "request_timeout",
        409 => "conflict",
        410 => "gone",
        412 => "precondition_failed",
        413 => "payload_too_large",
        415 => "unsupported_media_type",
        422 => "unprocessable_entity",
        423 => "locked",
        429 => "too_many_requests",
        500 => "internal_server_error",
        501 => "not_implemented",
        502 => "bad_gateway",
        503 => "service_unavailable",
        504 => "gateway_timeout",
        _ => "error",
    }
}
