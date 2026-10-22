use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MediaDescriptor {
    #[serde(rename = "type")]
    pub media_type: String,
    pub subtype: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub parameters: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub codec: Option<String>,
}

impl From<&str> for MediaDescriptor {
    fn from(value: &str) -> Self {
        Self::from_declared(value)
    }
}

impl From<String> for MediaDescriptor {
    fn from(value: String) -> Self {
        Self::from_declared(value)
    }
}

impl MediaDescriptor {
    pub fn new(media_type: impl Into<String>, subtype: impl Into<String>) -> Self {
        Self {
            media_type: media_type.into(),
            subtype: subtype.into(),
            parameters: BTreeMap::new(),
            codec: None,
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        let mut parts = value.split(';');
        let base = parts.next()?.trim_end();
        let (media_type, subtype) = base.split_once('/')?;
        if media_type != media_type.trim() || subtype != subtype.trim() {
            return None;
        }
        let media_type = media_type.trim();
        let subtype = subtype.trim();
        if !token(media_type) || !token(subtype) {
            return None;
        }
        let mut parameters = BTreeMap::new();
        for parameter in parts {
            let (name, value) = parameter.trim().split_once('=')?;
            let name = name.trim();
            let value = value.trim();
            if !token(name) || value.is_empty() || value.chars().any(char::is_control) {
                return None;
            }
            parameters.insert(name.to_ascii_lowercase(), value.to_owned());
        }
        Some(Self {
            media_type: media_type.to_owned(),
            subtype: subtype.to_owned(),
            parameters,
            codec: None,
        })
    }

    pub fn from_declared(value: impl Into<String>) -> Self {
        let value = value.into();
        Self::parse(&value).unwrap_or_else(|| Self::new(value, ""))
    }

    pub fn base(&self) -> String {
        format!(
            "{}/{}",
            self.media_type.to_ascii_lowercase(),
            self.subtype.to_ascii_lowercase()
        )
    }

    pub(crate) fn canonicalize(&mut self) {
        self.media_type.make_ascii_lowercase();
        self.subtype.make_ascii_lowercase();
        self.parameters = std::mem::take(&mut self.parameters)
            .into_iter()
            .map(|(name, value)| (name.to_ascii_lowercase(), value))
            .collect();
    }

    pub fn validate(&self) -> Result<(), String> {
        if !token(&self.media_type) || !token(&self.subtype) {
            return Err(format!(
                "invalid media type {:?}/{:?}",
                self.media_type, self.subtype
            ));
        }
        for (name, value) in &self.parameters {
            if !token(name) {
                return Err(format!("invalid media type parameter name {name:?}"));
            }
            if value.is_empty() || value.chars().any(char::is_control) {
                return Err(format!("media type parameter {name} has an invalid value"));
            }
        }
        if let Some(codec) = &self.codec
            && (codec.is_empty()
                || !codec.chars().all(|character| {
                    character.is_ascii_alphanumeric()
                        || matches!(character, '-' | '_' | '.' | ':' | '/')
                }))
        {
            return Err(format!("invalid media codec id {codec:?}"));
        }
        Ok(())
    }
}

impl fmt::Display for MediaDescriptor {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}/{}", self.media_type, self.subtype)?;
        for (name, value) in &self.parameters {
            write!(formatter, "; {name}={value}")?;
        }
        Ok(())
    }
}

pub fn media_type_matches(actual: &str, expected: &str) -> bool {
    if media_type_base(actual) != media_type_base(expected) {
        return false;
    }
    let actual_parameters = actual
        .split(';')
        .skip(1)
        .map(normalize_parameter)
        .collect::<BTreeMap<_, _>>();
    expected
        .split(';')
        .skip(1)
        .map(normalize_parameter)
        .all(|parameter| actual_parameters.get(&parameter.0) == Some(&parameter.1))
}

pub fn is_valid_media_type(value: &str) -> bool {
    MediaDescriptor::parse(value).is_some()
}

fn media_type_base(value: &str) -> String {
    value
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_lowercase()
}

fn normalize_parameter(value: &str) -> (String, String) {
    let normalized = value.trim().to_lowercase();
    let Some((name, value)) = normalized.split_once('=') else {
        return (normalized, String::new());
    };
    (
        name.trim_matches(is_ows).to_owned(),
        value.trim_matches(is_ows).to_owned(),
    )
}

fn is_ows(character: char) -> bool {
    matches!(character, ' ' | '\t')
}

fn token(value: &str) -> bool {
    !value.is_empty()
        && value.chars().all(|character| {
            character.is_ascii_alphanumeric()
                || matches!(
                    character,
                    '!' | '#'
                        | '$'
                        | '%'
                        | '&'
                        | '\''
                        | '*'
                        | '+'
                        | '-'
                        | '.'
                        | '^'
                        | '_'
                        | '`'
                        | '|'
                        | '~'
                )
        })
}

#[cfg(test)]
mod tests {
    use super::{MediaDescriptor, is_valid_media_type, media_type_matches};

    #[test]
    fn media_type_validation_allows_ows_before_parameters() {
        assert!(is_valid_media_type("application/json ; charset=utf-8"));
        assert!(is_valid_media_type(
            "application/json\t;\tcharset\t=\tutf-8"
        ));
        assert!(!is_valid_media_type("application /json; charset=utf-8"));
        assert!(!is_valid_media_type("application/json;"));
        assert!(!is_valid_media_type("application/json; charset=\n"));
    }

    #[test]
    fn media_type_matching_uses_runtime_normalization() {
        assert!(media_type_matches(
            "Application/PDF ; Version=ONE; Extra=Value",
            "application/pdf; version=one"
        ));
        assert!(media_type_matches(
            "application/pdf; version=one",
            "APPLICATION/PDF; VERSION=ONE"
        ));
        assert!(!media_type_matches(
            "application/pdf; version=two",
            "application/pdf; version=one"
        ));
        assert!(!media_type_matches(
            "application/pdf",
            "application/pdf; version=one"
        ));
        assert!(media_type_matches(
            "application/pdf ; version = 1",
            "application/pdf;version=1"
        ));
    }

    #[test]
    fn a_descriptor_parses_its_type_subtype_and_parameters() {
        let descriptor = MediaDescriptor::parse("Application/Problem+JSON; Charset=UTF-8").unwrap();
        assert_eq!(descriptor.base(), "application/problem+json");
        assert_eq!(descriptor.parameters["charset"], "UTF-8");
        assert_eq!(
            descriptor.to_string(),
            "Application/Problem+JSON; charset=UTF-8"
        );
        assert!(descriptor.validate().is_ok());
    }
}
