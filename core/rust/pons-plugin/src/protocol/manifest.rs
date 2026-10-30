use std::collections::{BTreeMap, BTreeSet};

use pons_ir::{Fidelity, validate_extension_key};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{LEGACY_PROTOCOL_VERSION, PROTOCOL_VERSION};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VersionRange {
    pub min: String,
    pub max: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EmitterManifest {
    pub name: String,
    pub version: String,
    pub protocol: VersionRange,
    pub ir_version: VersionRange,
    #[serde(default)]
    pub capabilities: EmitterCapabilities,
    #[serde(default)]
    pub option_schema: Option<OptionSchema>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OptionSchema {
    #[serde(default)]
    pub options: BTreeMap<String, OptionSpec>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OptionSpec {
    #[serde(rename = "type")]
    pub kind: OptionKind,
    #[serde(default)]
    pub required: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OptionKind {
    String,
    Boolean,
    Integer,
    Number,
    Array,
    Object,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EmitterCapabilities {
    #[serde(default)]
    pub streaming: Vec<String>,
    #[serde(default)]
    pub pagination: Vec<String>,
    #[serde(default)]
    pub enum_reprs: Vec<String>,
    #[serde(default)]
    pub direction_split: bool,
    #[serde(default)]
    pub provides: bool,
    #[serde(default)]
    pub custom_auth: bool,
    #[serde(default)]
    pub flattened: bool,
    #[serde(default)]
    pub wide_integers: bool,
    #[serde(default)]
    pub opaque: bool,
    #[serde(default)]
    pub object_openness: Vec<String>,
    #[serde(default)]
    pub extensions: BTreeMap<String, Fidelity>,
    #[serde(default)]
    pub validation: ValidationCapabilities,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ValidationCapabilities {
    #[serde(default)]
    pub backends: Vec<String>,
    #[serde(default)]
    pub capabilities: BTreeMap<String, String>,
}

const STREAMING: &[&str] = &["sse", "ndjson", "bytes", "websocket"];
const PAGINATION: &[&str] = &["pagination.cursor", "pagination.offset"];
const ENUM_REPRS: &[&str] = &["string", "external", "internal", "adjacent", "untagged"];
const OBJECT_OPENNESS: &[&str] = &["reject", "preserve_typed", "preserve_opaque"];
const VALIDATION_LEVELS: &[&str] = &["exact", "lossy", "unsupported"];

impl EmitterManifest {
    pub fn validate(&self, ir_version: &str) -> Result<(), String> {
        let protocol_min = parse_version(&self.protocol.min)?;
        let protocol_max = parse_version(&self.protocol.max)?;
        if protocol_min > protocol_max
            || !(supports(protocol_min, protocol_max, LEGACY_PROTOCOL_VERSION)
                || supports(protocol_min, protocol_max, PROTOCOL_VERSION))
        {
            return Err(format!(
                "emitter {} does not support protocol {LEGACY_PROTOCOL_VERSION} or \
                 {PROTOCOL_VERSION}",
                self.name
            ));
        }
        let ir_min = parse_version(&self.ir_version.min)?;
        let ir_max = parse_version(&self.ir_version.max)?;
        let current = parse_version(ir_version)?;
        if ir_min > ir_max || current < ir_min || current > ir_max {
            return Err(format!(
                "emitter {} does not support IR version {ir_version}",
                self.name
            ));
        }
        for (list, values, allowed) in [
            ("streaming", &self.capabilities.streaming, STREAMING),
            ("pagination", &self.capabilities.pagination, PAGINATION),
            ("enum_reprs", &self.capabilities.enum_reprs, ENUM_REPRS),
            (
                "object_openness",
                &self.capabilities.object_openness,
                OBJECT_OPENNESS,
            ),
        ] {
            validate_capability_list(list, values, allowed)?;
        }
        validate_extensions(&self.capabilities.extensions)?;
        self.capabilities.validation.validate()?;
        Ok(())
    }

    pub fn protocol_version(&self, validation: bool) -> Result<u32, String> {
        let min = parse_version(&self.protocol.min)?;
        let max = parse_version(&self.protocol.max)?;
        if min > max {
            return Err(format!(
                "emitter {} has an inverted protocol range",
                self.name
            ));
        }
        if validation && !supports(min, max, PROTOCOL_VERSION) {
            return Err(format!(
                "emitter {} supports protocol {LEGACY_PROTOCOL_VERSION} only; validation requires \
                 protocol {PROTOCOL_VERSION}",
                self.name
            ));
        }
        if supports(min, max, PROTOCOL_VERSION) {
            Ok(PROTOCOL_VERSION)
        } else if supports(min, max, LEGACY_PROTOCOL_VERSION) {
            Ok(LEGACY_PROTOCOL_VERSION)
        } else {
            Err(format!(
                "emitter {} does not support protocol {LEGACY_PROTOCOL_VERSION} or \
                 {PROTOCOL_VERSION}",
                self.name
            ))
        }
    }

    pub fn validate_options(&self, options: &BTreeMap<String, Value>) -> Result<(), String> {
        let Some(schema) = &self.option_schema else {
            if let Some(key) = options.keys().next() {
                return Err(format!(
                    "{} declares no options, so it takes an empty options object; {key:?} was \
                     given",
                    self.name
                ));
            }
            return Ok(());
        };
        for (key, spec) in &schema.options {
            if spec.required && !options.contains_key(key) {
                return Err(format!("option {key:?} is required by {}", self.name));
            }
        }
        for (key, value) in options {
            let Some(spec) = schema.options.get(key) else {
                return Err(format!(
                    "option {key:?} is not one of the options {} declares: {}",
                    self.name,
                    schema
                        .options
                        .keys()
                        .cloned()
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            };
            if !spec.accepts(value) {
                return Err(format!(
                    "option {key:?} is {}, but {} declares it as {}",
                    json_kind(value),
                    self.name,
                    spec.kind.as_str()
                ));
            }
        }
        Ok(())
    }
}

fn validate_extensions(extensions: &BTreeMap<String, Fidelity>) -> Result<(), String> {
    for namespace in extensions.keys() {
        validate_extension_key(namespace)
            .map_err(|error| format!("emitter extension capability {namespace:?}: {error}"))?;
    }
    Ok(())
}

impl ValidationCapabilities {
    fn validate(&self) -> Result<(), String> {
        let mut seen = BTreeSet::new();
        for backend in &self.backends {
            if backend.is_empty()
                || !backend
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
            {
                return Err(format!(
                    "emitter validation backend {backend:?} is not a plain backend id"
                ));
            }
            if !seen.insert(backend) {
                return Err(format!(
                    "emitter validation lists the backend {backend} more than once"
                ));
            }
        }
        for (id, level) in &self.capabilities {
            if id.strip_prefix("validation.").is_none_or(str::is_empty) {
                return Err(format!(
                    "emitter validation capability {id:?} is not a validation.* id"
                ));
            }
            if !VALIDATION_LEVELS.contains(&level.as_str()) {
                return Err(format!(
                    "emitter validation capability {id} has level {level:?}; it is exact, lossy, \
                     or unsupported"
                ));
            }
        }
        Ok(())
    }
}

impl OptionKind {
    fn as_str(self) -> &'static str {
        match self {
            OptionKind::String => "a string",
            OptionKind::Boolean => "a boolean",
            OptionKind::Integer => "an integer",
            OptionKind::Number => "a number",
            OptionKind::Array => "an array",
            OptionKind::Object => "an object",
        }
    }
}

impl OptionSpec {
    fn accepts(&self, value: &Value) -> bool {
        match self.kind {
            OptionKind::String => value.is_string(),
            OptionKind::Boolean => value.is_boolean(),
            OptionKind::Integer => value.is_i64() || value.is_u64(),
            OptionKind::Number => value.is_number(),
            OptionKind::Array => value.is_array(),
            OptionKind::Object => value.is_object(),
        }
    }
}

fn json_kind(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "a boolean",
        Value::Number(_) => "a number",
        Value::String(_) => "a string",
        Value::Array(_) => "an array",
        Value::Object(_) => "an object",
    }
}

fn validate_capability_list(list: &str, values: &[String], allowed: &[&str]) -> Result<(), String> {
    let mut seen = BTreeSet::new();
    for value in values {
        if !allowed.contains(&value.as_str()) {
            return Err(format!(
                "emitter capabilities name an unknown {list} value {value}"
            ));
        }
        if !seen.insert(value) {
            return Err(format!(
                "emitter capabilities list {value} more than once in {list}"
            ));
        }
    }
    Ok(())
}

fn supports(min: (u32, u32), max: (u32, u32), version: u32) -> bool {
    let version = (version, 0);
    min <= version && version <= max
}

fn parse_version(version: &str) -> Result<(u32, u32), String> {
    let Some((major, minor)) = version.split_once('.') else {
        return Err(format!("invalid version {version:?}"));
    };
    let major = major
        .parse()
        .map_err(|_| format!("invalid version {version:?}"))?;
    let minor = minor
        .parse()
        .map_err(|_| format!("invalid version {version:?}"))?;
    Ok((major, minor))
}
