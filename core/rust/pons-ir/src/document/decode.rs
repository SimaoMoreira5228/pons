use serde_json::Value;

use crate::IR_VERSION;
use crate::migration::MigrationRegistry;

use super::Document;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecodeError {
    Json(String),
    UnsupportedVersion {
        found: String,
        supported: &'static str,
    },
    Migration {
        name: String,
        message: String,
    },
    Member {
        path: String,
        message: String,
    },
}

impl std::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Json(message) => write!(f, "invalid JSON: {message}"),
            Self::UnsupportedVersion { found, supported } => write!(
                f,
                "unsupported IR version {found}, this build reads {supported}"
            ),
            Self::Migration { name, message } => {
                write!(f, "migration {name} failed: {message}")
            }
            Self::Member { path, message } if path.is_empty() => write!(f, "{message}"),
            Self::Member { path, message } => write!(f, "{path}: {message}"),
        }
    }
}

impl std::error::Error for DecodeError {}

pub fn decode(bytes: &[u8]) -> Result<Document, DecodeError> {
    decode_with(bytes, &MigrationRegistry::empty())
}

pub fn decode_with(bytes: &[u8], migrations: &MigrationRegistry) -> Result<Document, DecodeError> {
    let value: Value =
        serde_json::from_slice(bytes).map_err(|error| DecodeError::Json(error.to_string()))?;
    let Some(found) = value
        .get("ir_version")
        .and_then(Value::as_str)
        .map(str::to_owned)
    else {
        return Err(DecodeError::Member {
            path: "ir_version".into(),
            message: "the document does not carry a string ir_version".into(),
        });
    };
    let encoded;
    let source = if found == IR_VERSION {
        bytes
    } else {
        let Some(value) = migrations.migrate(value, &found, IR_VERSION)? else {
            return Err(DecodeError::UnsupportedVersion {
                found,
                supported: IR_VERSION,
            });
        };
        encoded = serde_json::to_vec(&value).map_err(|error| DecodeError::Migration {
            name: "encode".into(),
            message: format!("cannot encode the migrated document: {error}"),
        })?;
        &encoded
    };
    serde_json::from_slice::<Document>(source).map_err(|error| {
        let text = error.to_string();
        let message = text.split(" at line ").next().unwrap_or(&text);
        DecodeError::Member {
            path: member_path(source, error.line(), error.column()),
            message: message.to_owned(),
        }
    })
}

fn member_path(bytes: &[u8], line: usize, column: usize) -> String {
    enum Part {
        Key(String),
        Index(usize),
    }

    struct Frame {
        array: bool,
        index: usize,
        key: Option<String>,
    }

    let mut frames: Vec<Frame> = Vec::new();
    let mut at = 0;
    let target = byte_offset(bytes, line, column);
    while at < target {
        match bytes[at] {
            b'{' | b'[' => frames.push(Frame {
                array: bytes[at] == b'[',
                index: 0,
                key: None,
            }),
            b'}' | b']' => {
                frames.pop();
            }
            b',' => {
                if let Some(frame) = frames.last_mut() {
                    frame.index += 1;
                    frame.key = None;
                }
            }
            b':' | b' ' | b'\t' | b'\n' | b'\r' => {}
            b'"' => {
                let (text, end) = string_at(bytes, at);
                let is_key = bytes[end..]
                    .iter()
                    .find(|byte| !byte.is_ascii_whitespace())
                    .is_some_and(|byte| *byte == b':');
                if is_key && let Some(frame) = frames.last_mut() {
                    frame.key = Some(text);
                }
                at = end;
                continue;
            }
            _ => {}
        }
        at += 1;
    }

    let mut parts: Vec<Part> = Vec::new();
    for frame in frames {
        if frame.array {
            parts.push(Part::Index(frame.index));
        } else if let Some(key) = frame.key {
            parts.push(Part::Key(key));
        }
    }

    if parts.len() > 2 && matches!(&parts[0], Part::Key(key) if key == "types") {
        parts.truncate(2);
    }

    let mut path = String::new();
    for part in parts {
        match part {
            Part::Index(index) => path.push_str(&format!("[{index}]")),
            Part::Key(key) => {
                if !path.is_empty() {
                    path.push('.');
                }
                path.push_str(&key);
            }
        }
    }
    path
}

fn byte_offset(bytes: &[u8], line: usize, column: usize) -> usize {
    let mut offset = 0;
    for _ in 1..line.max(1) {
        match bytes[offset..].iter().position(|byte| *byte == b'\n') {
            Some(at) => offset += at + 1,
            None => return bytes.len(),
        }
    }
    offset + column.saturating_sub(1).min(bytes.len() - offset)
}

fn string_at(bytes: &[u8], start: usize) -> (String, usize) {
    let mut at = start + 1;
    while let Some(byte) = bytes.get(at) {
        match byte {
            b'\\' => at += 2,
            b'"' => {
                let raw = String::from_utf8_lossy(&bytes[start..=at]).into_owned();
                let text = if raw.contains('\\') {
                    serde_json::from_str(&raw).unwrap_or_default()
                } else {
                    raw[1..raw.len() - 1].to_owned()
                };
                return (text, at + 1);
            }
            _ => at += 1,
        }
    }
    (String::new(), at)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValidationError {
    UnsupportedVersion { found: String },
    DuplicateOperation { id: Vec<Vec<String>> },
    UnknownType { owner: String, reference: String },
    UnknownAuthScheme { owner: String, scheme: String },
    InvalidValue { path: String, message: String },
}

impl std::fmt::Display for ValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnsupportedVersion { found } => write!(f, "unsupported IR version {found}"),
            Self::DuplicateOperation { id } => write!(f, "duplicate operation id {id:?}"),
            Self::UnknownType { owner, reference } => {
                write!(f, "{owner} refers to unknown type {reference}")
            }
            Self::UnknownAuthScheme { owner, scheme } => {
                write!(f, "{owner} refers to unknown auth scheme {scheme}")
            }
            Self::InvalidValue { path, message } => write!(f, "{path}: {message}"),
        }
    }
}

impl std::error::Error for ValidationError {}
