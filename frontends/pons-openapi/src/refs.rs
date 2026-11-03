use std::collections::BTreeSet;

use pons_ir::Diagnostic;
use serde_json::{Map, Value};

use crate::document::push_warning;

#[derive(Clone, Copy)]
pub(crate) enum Site<'a> {
    Operation(&'a str),
    Path(&'a str),
    Document,
}

impl std::fmt::Display for Site<'_> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Operation(operation) => write!(formatter, "operation {operation}"),
            Self::Path(path) => write!(formatter, "path {path:?}"),
            Self::Document => formatter.write_str("document"),
        }
    }
}

pub(crate) struct Resolver<'a> {
    root: &'a Map<String, Value>,
}

impl<'a> Resolver<'a> {
    pub(crate) fn new(root: &'a Map<String, Value>) -> Self {
        Self { root }
    }

    pub(crate) fn resolve<'v>(
        &'v self,
        value: &'v Value,
        site: Site<'_>,
        diagnostics: &mut Vec<Diagnostic>,
    ) -> Option<&'v Value> {
        let mut current = value;
        let mut in_flight = BTreeSet::new();
        loop {
            let Some(reference) = current.get("$ref").and_then(Value::as_str) else {
                return Some(current);
            };
            if !in_flight.insert(reference) {
                report(diagnostics, site, reference, "forms a reference cycle");
                return None;
            }
            if !reference.starts_with("#/") {
                report(
                    diagnostics,
                    site,
                    reference,
                    "is not a local #/ reference, and only local references are resolved",
                );
                return None;
            }
            let target = match self.pointer(reference) {
                Ok(target) => target,
                Err(PointerError::Malformed(reason)) => {
                    report(diagnostics, site, reference, &reason);
                    return None;
                }
                Err(PointerError::Missing) => {
                    report(
                        diagnostics,
                        site,
                        reference,
                        "does not resolve against the document",
                    );
                    return None;
                }
            };
            current = target;
        }
    }

    fn pointer<'v>(&'v self, reference: &str) -> Result<&'v Value, PointerError> {
        let mut tokens = reference
            .strip_prefix("#/")
            .ok_or(PointerError::Missing)?
            .split('/');
        let first = tokens.next().ok_or(PointerError::Missing)?;
        let mut current = self.root.get(&token(first)?).ok_or(PointerError::Missing)?;
        for part in tokens {
            current = step(current, &token(part)?).ok_or(PointerError::Missing)?;
        }
        Ok(current)
    }
}

enum PointerError {
    Malformed(String),
    Missing,
}

fn step<'v>(value: &'v Value, token: &str) -> Option<&'v Value> {
    match value {
        Value::Object(object) => object.get(token),
        Value::Array(items) => items.get(token.parse::<usize>().ok()?),
        _ => None,
    }
}

fn token(raw: &str) -> Result<String, PointerError> {
    let decoded = percent_decode(raw).map_err(PointerError::Malformed)?;
    unescape(&decoded).map_err(PointerError::Malformed)
}

fn percent_decode(raw: &str) -> Result<String, String> {
    let bytes = raw.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] != b'%' {
            decoded.push(bytes[index]);
            index += 1;
            continue;
        }
        let Some(escape) = bytes.get(index..index + 3) else {
            return Err("ends with an incomplete percent escape".into());
        };
        let [_, high, low] = escape else {
            return Err("ends with an incomplete percent escape".into());
        };
        let (Some(high), Some(low)) = (hex_digit(*high), hex_digit(*low)) else {
            return Err(format!(
                "contains the malformed percent escape {:?}",
                &raw[index..index + 3]
            ));
        };
        decoded.push(high << 4 | low);
        index += 3;
    }
    String::from_utf8(decoded).map_err(|_| "does not decode to UTF-8".to_owned())
}

fn hex_digit(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn unescape(token: &str) -> Result<String, String> {
    let mut result = String::with_capacity(token.len());
    let mut characters = token.chars();
    while let Some(character) = characters.next() {
        if character != '~' {
            result.push(character);
            continue;
        }
        match characters.next() {
            Some('0') => result.push('~'),
            Some('1') => result.push('/'),
            Some(escape) => {
                return Err(format!(
                    "contains the malformed pointer escape \"~{escape}\""
                ));
            }
            None => return Err("ends with an incomplete pointer escape \"~\"".into()),
        }
    }
    Ok(result)
}

fn report(diagnostics: &mut Vec<Diagnostic>, site: Site<'_>, reference: &str, reason: &str) {
    let message = format!("{site}: the reference {reference:?} {reason}, so it is dropped");
    push_warning(diagnostics, message);
}
