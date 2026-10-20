use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path};

use pons_ir::{
    AuthScheme, Coverage, Direction, Document, EnumRepr, ObjectOpenness, StreamKind, TypeKind,
    ValidationCoverage, coverage_for_document, operation_subject, strict_violation_message,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const PROTOCOL_VERSION: u32 = 3;
const STREAMING: &[&str] = &["sse", "ndjson", "bytes", "websocket"];
const ENUM_REPRS: &[&str] = &["string", "external", "internal", "adjacent", "untagged"];
const OBJECT_OPENNESS: &[&str] = &["reject", "preserve_typed", "preserve_opaque"];
const VALIDATION_LEVELS: &[&str] = &["exact", "lossy", "unsupported"];
const DEVICE_INDEXES: &[&str] = &["1", "2", "3", "4", "5", "6", "7", "8", "9", "¹", "²", "³"];

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EmitMode {
    Strict,
    Lossy,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct EmitRequest<'a> {
    pub protocol: u32,
    pub source_ir_version: String,
    pub mode: EmitMode,
    pub ir: &'a Document,
    pub options: BTreeMap<String, Value>,
    pub target: &'a str,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GeneratedFile {
    pub path: String,
    pub content: String,
    #[serde(default = "utf8_encoding")]
    pub encoding: String,
}

fn utf8_encoding() -> String {
    "utf8".into()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginDiagnostic {
    pub severity: DiagnosticSeverity,
    pub code: String,
    pub message: String,
    #[serde(default)]
    pub origin: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DiagnosticSeverity {
    Info,
    Warning,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EmitResponse {
    pub protocol: u32,
    #[serde(default)]
    pub files: Vec<GeneratedFile>,
    #[serde(default)]
    pub diagnostics: Vec<PluginDiagnostic>,
    #[serde(default)]
    pub coverage: Option<Coverage>,
    #[serde(default)]
    pub validation_coverage: Option<ValidationCoverage>,
    #[serde(default)]
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilityWarning(pub String);

impl EmitterManifest {
    pub fn validate(&self, ir_version: &str) -> Result<(), String> {
        let protocol_min = parse_version(&self.protocol.min)?;
        let protocol_max = parse_version(&self.protocol.max)?;
        let protocol_version = (PROTOCOL_VERSION, 0);
        if protocol_min > protocol_max
            || protocol_max < protocol_version
            || protocol_min > protocol_version
        {
            return Err(format!(
                "emitter {} does not support protocol {PROTOCOL_VERSION}",
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
            ("enum_reprs", &self.capabilities.enum_reprs, ENUM_REPRS),
            (
                "object_openness",
                &self.capabilities.object_openness,
                OBJECT_OPENNESS,
            ),
        ] {
            validate_capability_list(list, values, allowed)?;
        }
        self.capabilities.validation.validate()?;
        Ok(())
    }

    pub fn validation_gap(&self, options: &BTreeMap<String, Value>) -> Option<CapabilityWarning> {
        let RequestedValidation::Enabled { backend } = requested_validation(options) else {
            return None;
        };
        let validation = &self.capabilities.validation;
        let backend_declared = backend.map_or(!validation.backends.is_empty(), |backend| {
            validation.backends.iter().any(|id| id == backend)
        });
        let capability_declared = validation
            .capabilities
            .keys()
            .any(|id| id.starts_with("validation."));
        let message = match (backend_declared, capability_declared) {
            (true, true) => return None,
            (false, true) => format!(
                "{} does not declare {} validation backend",
                self.name,
                backend.map_or("a".to_owned(), |backend| format!("the {backend}"))
            ),
            (true, false) => format!("{} declares no validation capabilities", self.name),
            (false, false) => format!(
                "{} does not declare validation support for {}",
                self.name,
                backend.map_or("a validation backend".to_owned(), |backend| {
                    format!("the {backend} backend")
                })
            ),
        };
        Some(CapabilityWarning(message))
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

    pub fn unsupported_features(&self, document: &Document) -> Vec<CapabilityWarning> {
        let mut warnings = Vec::new();
        if !self.capabilities.wide_integers
            && coverage_for_document(document, true).map_or(true, |coverage| {
                coverage
                    .lossy
                    .iter()
                    .any(|subject| subject.starts_with("operations."))
            })
        {
            warnings.push(CapabilityWarning(format!(
                "{} does not declare lossless wide integer support",
                self.name
            )));
        }
        if !self.capabilities.custom_auth
            && document
                .auth_schemes
                .values()
                .any(|scheme| matches!(scheme, AuthScheme::Custom { .. }))
        {
            warnings.push(CapabilityWarning(format!(
                "{} does not declare custom authentication scheme support",
                self.name
            )));
        }
        if !self.capabilities.opaque
            && coverage_for_document(document, false)
                .map_or(true, |coverage| !coverage.opaque.is_empty())
        {
            warnings.push(CapabilityWarning(format!(
                "{} does not declare opaque value support",
                self.name
            )));
        }
        for operation in &document.operations {
            for kind in stream_kinds(operation) {
                if !self
                    .capabilities
                    .streaming
                    .iter()
                    .any(|value| value == kind)
                {
                    warnings.push(CapabilityWarning(format!(
                        "{} does not declare support for {kind} streams in {}",
                        self.name,
                        operation_subject(operation)
                    )));
                }
            }
            if operation.provides.is_some() && !self.capabilities.provides {
                warnings.push(CapabilityWarning(format!(
                    "{} does not declare credential capture support in {}",
                    self.name,
                    operation_subject(operation)
                )));
            }
        }
        if !self.capabilities.direction_split {
            let mut sides = BTreeMap::<&str, (bool, bool)>::new();
            for definition in document.types.values() {
                let Some(origin) = definition.origin.as_deref() else {
                    continue;
                };
                match definition.direction {
                    Direction::Input => sides.entry(origin).or_default().0 = true,
                    Direction::Output => sides.entry(origin).or_default().1 = true,
                    Direction::Both => {}
                }
            }
            for (origin, (input, output)) in sides {
                if input && output {
                    warnings.push(CapabilityWarning(format!(
                        "{} does not declare direction-split type support for {origin}",
                        self.name
                    )));
                }
            }
        }
        for (id, definition) in &document.types {
            match &definition.definition {
                TypeKind::Enum { repr, .. } => {
                    let name = enum_name(repr);
                    if !self
                        .capabilities
                        .enum_reprs
                        .iter()
                        .any(|value| value == name)
                    {
                        warnings.push(CapabilityWarning(format!(
                            "{} does not declare {name} enum support in {id}",
                            self.name
                        )));
                    }
                }
                TypeKind::Struct {
                    flattened,
                    openness,
                    ..
                } => {
                    if !flattened.is_empty() && !self.capabilities.flattened {
                        warnings.push(CapabilityWarning(format!(
                            "{} does not declare flattened struct support in {id}",
                            self.name
                        )));
                    }
                    let openness = object_openness_name(openness);
                    if !self
                        .capabilities
                        .object_openness
                        .iter()
                        .any(|value| value == openness)
                    {
                        warnings.push(CapabilityWarning(format!(
                            "{} does not declare {openness} object openness support in {id}",
                            self.name
                        )));
                    }
                }
                _ => {}
            }
        }
        warnings
    }
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

enum RequestedValidation<'a> {
    Disabled,
    Enabled { backend: Option<&'a str> },
}

fn requested_validation(options: &BTreeMap<String, Value>) -> RequestedValidation<'_> {
    let Some(value) = options.get("validation") else {
        return RequestedValidation::Disabled;
    };
    match value {
        Value::Null => RequestedValidation::Disabled,
        Value::Bool(false) => RequestedValidation::Disabled,
        Value::String(backend) if backend == "none" => RequestedValidation::Disabled,
        Value::String(backend) => RequestedValidation::Enabled {
            backend: Some(backend),
        },
        Value::Object(table) => match table.get("backend") {
            Some(Value::String(backend)) if backend == "none" => RequestedValidation::Disabled,
            Some(Value::String(backend)) => RequestedValidation::Enabled {
                backend: Some(backend),
            },
            _ => RequestedValidation::Enabled { backend: None },
        },
        _ => RequestedValidation::Enabled { backend: None },
    }
}

pub fn validation_enabled(options: &BTreeMap<String, Value>) -> bool {
    matches!(
        requested_validation(options),
        RequestedValidation::Enabled { .. }
    )
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

fn stream_kinds(operation: &pons_ir::Operation) -> Vec<&'static str> {
    let mut kinds = Vec::new();
    if let Some(stream) = &operation.stream {
        let kind = match stream.kind {
            StreamKind::Sse { .. } => "sse",
            StreamKind::Ndjson { .. } => "ndjson",
            StreamKind::Bytes { .. } => "bytes",
            StreamKind::Websocket { .. } => "websocket",
        };
        if !kinds.contains(&kind) {
            kinds.push(kind);
        }
    }
    kinds
}

pub fn validate_response(response: &EmitResponse) -> Result<(), String> {
    if response.protocol != PROTOCOL_VERSION {
        return Err(format!(
            "emitter returned protocol {}, expected {PROTOCOL_VERSION}",
            response.protocol
        ));
    }
    if let Some(error) = &response.error {
        return Err(error.clone());
    }
    let mut paths = std::collections::BTreeSet::new();
    for file in &response.files {
        validate_output_path(&file.path)?;
        if file.encoding != "utf8" {
            return Err(format!(
                "emitter returned unsupported encoding {} for {}",
                file.encoding, file.path
            ));
        }
        if !paths.insert(&file.path) {
            return Err(format!(
                "emitter returned duplicate output path {}",
                file.path
            ));
        }
    }
    for path in &paths {
        let prefix = path.as_str();
        if paths.iter().any(|other| {
            other.len() > prefix.len()
                && other.starts_with(prefix)
                && other.as_bytes().get(prefix.len()) == Some(&b'/')
        }) {
            return Err(format!(
                "emitter output path conflicts with a directory: {path}"
            ));
        }
    }
    Ok(())
}

pub(crate) fn validate_response_for_document(
    response: &EmitResponse,
    document: &Document,
    mode: EmitMode,
    opaque_supported: bool,
    validation_enabled: bool,
) -> Result<(), String> {
    validate_response(response)?;
    let Some(coverage) = &response.coverage else {
        return Err("emitter response is missing coverage".into());
    };
    let expected = pons_ir::coverage_subjects(document);
    let mut seen = BTreeSet::new();
    for (level, subjects) in [
        ("exact", &coverage.exact),
        ("opaque", &coverage.opaque),
        ("lossy", &coverage.lossy),
        ("unsupported", &coverage.unsupported),
    ] {
        for subject in subjects {
            if !expected.contains(subject) {
                return Err(format!(
                    "emitter coverage names unknown {level} subject {subject}"
                ));
            }
            if !seen.insert(subject.clone()) {
                return Err(format!("emitter coverage names {subject} more than once"));
            }
        }
    }
    if seen != expected {
        let missing = expected.difference(&seen).cloned().collect::<Vec<_>>();
        return Err(format!("emitter coverage is missing subjects {missing:?}"));
    }
    let canonical = pons_ir::canonical_coverage(document, mode == EmitMode::Lossy)?;
    for (subject, level) in coverage.levels() {
        if let Some(floor) = canonical.get(&subject)
            && level < *floor
        {
            return Err(format!(
                "emitter reported {subject} as {} coverage, better than the canonical {} coverage",
                level.as_str(),
                floor.as_str()
            ));
        }
    }
    if mode == EmitMode::Strict && !opaque_supported && !coverage.opaque.is_empty() {
        return Err("emitter reported opaque coverage without declaring opaque support".into());
    }
    if mode == EmitMode::Strict
        && let Some(violations) = strict_violation_message(coverage)
    {
        return Err(format!("emitter reported {violations}"));
    }
    validate_validation_coverage(response, document, mode, validation_enabled)
}

fn validate_validation_coverage(
    response: &EmitResponse,
    document: &Document,
    mode: EmitMode,
    validation_enabled: bool,
) -> Result<(), String> {
    let Some(coverage) = &response.validation_coverage else {
        return if validation_enabled {
            Err(
                "emitter response is missing validation coverage while validation is enabled"
                    .into(),
            )
        } else {
            Ok(())
        };
    };
    let expected = pons_ir::coverage_subjects(document);
    let mut seen = BTreeSet::new();
    for (level, subjects) in [
        ("exact", &coverage.exact),
        ("lossy", &coverage.lossy),
        ("unsupported", &coverage.unsupported),
    ] {
        for subject in subjects {
            if !expected.contains(subject) {
                return Err(format!(
                    "emitter validation coverage names unknown {level} subject {subject}"
                ));
            }
            if !seen.insert(subject.clone()) {
                return Err(format!(
                    "emitter validation coverage names {subject} more than once"
                ));
            }
        }
    }
    if seen != expected {
        let missing = expected.difference(&seen).cloned().collect::<Vec<_>>();
        return Err(format!(
            "emitter validation coverage is missing subjects {missing:?}"
        ));
    }
    if mode == EmitMode::Strict
        && let Some(violations) = coverage.strict_violation_message()
    {
        return Err(format!("emitter reported {violations}"));
    }
    Ok(())
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

fn validate_output_path(path: &str) -> Result<(), String> {
    let candidate = Path::new(path);
    if path.is_empty()
        || path.contains('\\')
        || path.contains(':')
        || path
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == ".." || is_reserved_device(part))
        || candidate.is_absolute()
        || candidate
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(format!("emitter returned unsafe output path {path:?}"));
    }
    Ok(())
}

fn is_reserved_device(segment: &str) -> bool {
    let trimmed = segment.trim_end_matches(' ');
    let name = trimmed
        .split_once('.')
        .map_or(trimmed, |(stem, _)| stem)
        .to_ascii_uppercase();
    matches!(name.as_str(), "NUL" | "CON" | "PRN" | "AUX")
        || ["COM", "LPT"].iter().any(|prefix| {
            name.strip_prefix(prefix)
                .is_some_and(|index| DEVICE_INDEXES.contains(&index))
        })
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

fn enum_name(repr: &EnumRepr) -> &'static str {
    match repr {
        EnumRepr::String => "string",
        EnumRepr::External => "external",
        EnumRepr::Internal { .. } => "internal",
        EnumRepr::Adjacent { .. } => "adjacent",
        EnumRepr::Untagged => "untagged",
    }
}

fn object_openness_name(openness: &ObjectOpenness) -> &'static str {
    match openness {
        ObjectOpenness::Reject => "reject",
        ObjectOpenness::PreserveTyped { .. } => "preserve_typed",
        ObjectOpenness::PreserveOpaque => "preserve_opaque",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pons_ir::{CoverageLevel, InlineType, TypeRef};

    fn manifest() -> EmitterManifest {
        EmitterManifest {
            name: "fixture".into(),
            version: "1.0.0".into(),
            protocol: VersionRange {
                min: "3.0".into(),
                max: "3.0".into(),
            },
            ir_version: VersionRange {
                min: "0.2".into(),
                max: "0.2".into(),
            },
            capabilities: EmitterCapabilities::default(),
            option_schema: None,
        }
    }

    #[test]
    fn wide_integers_require_declared_capability() {
        let mut value: serde_json::Value =
            serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
        value["types"]["User"]["fields"][0]["ty"]["wire"] = serde_json::json!("number");
        let document: Document = serde_json::from_value(value).unwrap();
        let manifest = manifest();
        assert!(
            manifest
                .unsupported_features(&document)
                .iter()
                .any(|warning| warning.0.contains("wide integer"))
        );
        let mut supported = manifest;
        supported.capabilities.wide_integers = true;
        assert!(
            !supported
                .unsupported_features(&document)
                .iter()
                .any(|warning| warning.0.contains("wide integer"))
        );
    }

    #[test]
    fn wide_integer_capability_counts_only_reachable_types_and_errors() {
        let manifest = manifest();
        let mut value: serde_json::Value =
            serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
        value["types"]["Orphan"] = serde_json::json!({
            "kind": "struct",
            "openness": "reject",
            "name": ["orphan"],
            "direction": "both",
            "fields": [{
                "wire": "id",
                "ident": ["id"],
                "ty": {"kind": "int", "bits": 64, "signed": false, "wire": "number"},
                "presence": "required"
            }]
        });
        value["common_errors"] = serde_json::json!([{
            "status": "5xx",
            "name": ["common", "error"],
            "content": "json",
            "body": {"ref": "Orphan"}
        }]);
        let mut document: Document = serde_json::from_value(value.clone()).unwrap();
        document
            .operations
            .iter_mut()
            .for_each(|operation| operation.errors_inherit = false);
        document.validate().unwrap();
        assert!(document.has_wide_integer());
        assert!(
            !manifest
                .unsupported_features(&document)
                .iter()
                .any(|warning| warning.0.contains("wide integer"))
        );

        document.operations[0].errors_inherit = true;
        assert!(
            manifest
                .unsupported_features(&document)
                .iter()
                .any(|warning| warning.0.contains("wide integer"))
        );

        value["types"]["User"]["fields"][0]["ty"]["wire"] = serde_json::json!("number");
        let reachable: Document = serde_json::from_value(value).unwrap();
        reachable.validate().unwrap();
        assert!(
            manifest
                .unsupported_features(&reachable)
                .iter()
                .any(|warning| warning.0.contains("wide integer"))
        );
    }

    #[test]
    fn streaming_contracts_require_a_declared_streaming_capability() {
        let mut sse = manifest();
        sse.capabilities.streaming = vec!["sse".into()];
        let document: Document =
            serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
        assert!(stream_warnings(&sse, &document).is_empty());

        let mut streaming_body: serde_json::Value =
            serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
        streaming_body["operations"][0]["body"]["content"] = serde_json::json!("stream");
        let streaming_body: Document = serde_json::from_value(streaming_body).unwrap();
        refuses_stream_body(&streaming_body);

        let mut bytes_only = manifest();
        bytes_only.capabilities.streaming = vec!["bytes".into()];
        assert_eq!(
            stream_warnings(&bytes_only, &document),
            vec!["fixture does not declare support for sse streams in events.watch"]
        );

        let mut framed = sse.clone();
        framed.capabilities.streaming = vec!["sse".into(), "bytes".into()];
        assert!(stream_warnings(&framed, &document).is_empty());

        let mut duplex: serde_json::Value =
            serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
        duplex["operations"][0]["body"]["content"] = serde_json::json!("stream");
        duplex["operations"][0]["responses"] =
            serde_json::json!([{ "status": 200, "content": "stream" }]);
        duplex["operations"][0]["stream"] = serde_json::json!({
            "kind": "sse",
            "events": [{ "event": "user", "data": { "ref": "User" } }]
        });
        let duplex: Document = serde_json::from_value(duplex).unwrap();
        refuses_stream_body(&duplex);
        assert_eq!(
            stream_warnings(&bytes_only, &duplex),
            vec![
                "fixture does not declare support for sse streams in auth.login",
                "fixture does not declare support for sse streams in events.watch",
            ],
            "a streaming request body must not hide the response stream"
        );
        assert!(stream_warnings(&framed, &duplex).is_empty());

        let mut byte_stream: serde_json::Value =
            serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
        byte_stream["operations"][1]["stream"] = serde_json::json!({
            "kind": "bytes",
            "content_type": "application/octet-stream"
        });
        let byte_stream: Document = serde_json::from_value(byte_stream).unwrap();
        byte_stream.validate().unwrap();
        assert_eq!(
            stream_warnings(&sse, &byte_stream),
            vec!["fixture does not declare support for bytes streams in events.watch"]
        );
        assert!(stream_warnings(&framed, &byte_stream).is_empty());
    }

    #[test]
    fn byte_responses_need_no_streaming_capability() {
        let mut value: serde_json::Value =
            serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
        value["operations"][1]
            .as_object_mut()
            .unwrap()
            .remove("stream");
        value["operations"][1]["responses"] = serde_json::json!([{
            "status": 200,
            "content": "bytes",
            "body": { "kind": "bytes", "encoding": "base64" }
        }]);
        let document: Document = serde_json::from_value(value).unwrap();
        document.validate().unwrap();

        let mut manifest = manifest();
        manifest.capabilities.streaming = vec!["sse".into(), "ndjson".into()];
        assert!(
            stream_warnings(&manifest, &document).is_empty(),
            "a byte response is not a streaming contract: {:?}",
            stream_warnings(&manifest, &document)
        );
    }

    fn refuses_stream_body(document: &Document) {
        let refusal = document
            .validate()
            .expect_err("the IR refuses a stream request body")
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            refusal.contains(
                "a request body cannot be stream content, because no built-in target reads one"
            ),
            "{refusal}"
        );
    }

    fn stream_warnings(manifest: &EmitterManifest, document: &Document) -> Vec<String> {
        manifest
            .unsupported_features(document)
            .into_iter()
            .map(|warning| warning.0)
            .filter(|message| message.contains(" streams in "))
            .collect()
    }

    #[test]
    fn the_ir_refuses_a_streamed_request_body() {
        let mut value: serde_json::Value =
            serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
        value["operations"][0]["body"]["content"] = serde_json::json!("stream");
        let document: Document = serde_json::from_value(value).unwrap();
        refuses_stream_body(&document);
    }

    #[test]
    fn custom_auth_schemes_require_declared_capability() {
        let mut value: serde_json::Value =
            serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
        let document: Document = serde_json::from_value(value.clone()).unwrap();
        let manifest = manifest();
        assert!(
            !manifest
                .unsupported_features(&document)
                .iter()
                .any(|warning| warning.0.contains("custom authentication scheme"))
        );

        value["auth_schemes"]["tenant"] =
            serde_json::json!({ "kind": "custom", "description": "tenant handshake" });
        let document: Document = serde_json::from_value(value).unwrap();
        assert!(
            manifest
                .unsupported_features(&document)
                .iter()
                .any(|warning| warning.0.contains("custom authentication scheme"))
        );
        let mut supported = manifest;
        supported.capabilities.custom_auth = true;
        assert!(
            !supported
                .unsupported_features(&document)
                .iter()
                .any(|warning| warning.0.contains("custom authentication scheme"))
        );
    }

    #[test]
    fn opaque_coverage_requires_declared_capability() {
        let mut value: serde_json::Value =
            serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
        value["types"]["User"]["fields"][0]["ty"] = serde_json::json!({
            "kind": "opaque",
            "codec": "example",
            "wire": "json"
        });
        let document: Document = serde_json::from_value(value).unwrap();
        let manifest = manifest();
        assert!(
            manifest
                .unsupported_features(&document)
                .iter()
                .any(|warning| warning.0.contains("opaque value support"))
        );
        let mut paginated: serde_json::Value =
            serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
        paginated["operations"][1]["semantics"]["pagination"] = serde_json::json!({
            "request": {"kind": "cursor", "field": "limit"},
            "items": "/items"
        });
        pons_ir::decode(paginated.to_string().as_bytes()).unwrap();
        let mut supported = manifest;
        supported.capabilities.opaque = true;
        assert!(
            !supported
                .unsupported_features(&document)
                .iter()
                .any(|warning| warning.0.contains("opaque value support"))
        );
    }

    #[test]
    fn protocol_three_round_trips_a_manifest_request_and_response() {
        let document: Document =
            serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
        let manifest = manifest();
        let encoded = serde_json::to_value(&manifest).unwrap();
        assert_eq!(encoded["protocol"]["min"], "3.0");
        assert_eq!(
            serde_json::from_value::<EmitterManifest>(encoded).unwrap(),
            manifest
        );

        let request = EmitRequest {
            protocol: PROTOCOL_VERSION,
            source_ir_version: document.ir_version.clone(),
            mode: EmitMode::Lossy,
            ir: &document,
            options: BTreeMap::new(),
            target: "fixture",
        };
        let value = serde_json::to_value(request).unwrap();
        assert_eq!(value["protocol"], 3);
        assert_eq!(value["source_ir_version"], document.ir_version);
        assert_eq!(value["mode"], "lossy");
        assert_eq!(
            serde_json::from_value::<Document>(value["ir"].clone()).unwrap(),
            document
        );

        let response = EmitResponse {
            protocol: PROTOCOL_VERSION,
            files: Vec::new(),
            diagnostics: Vec::new(),
            coverage: Some(coverage_for_document(&document, false).unwrap()),
            validation_coverage: Some(ValidationCoverage::exact_for_document(&document)),
            error: None,
        };
        let encoded = serde_json::to_value(&response).unwrap();
        assert_eq!(
            serde_json::from_value::<EmitResponse>(encoded).unwrap(),
            response
        );
    }

    fn options(source: &str) -> BTreeMap<String, Value> {
        match serde_json::from_str(source) {
            Ok(options) => options,
            Err(error) => panic!("{source}: {error}"),
        }
    }

    fn schema(source: &str) -> EmitterManifest {
        let mut manifest = manifest();
        manifest.option_schema = match serde_json::from_str(source) {
            Ok(schema) => Some(schema),
            Err(error) => panic!("{source}: {error}"),
        };
        manifest
    }

    #[test]
    fn an_option_the_manifest_does_not_declare_is_refused() {
        let declared = schema(
            r#"{"options":{"package":{"type":"string","required":true},"retries":{"type":"array"}}}"#,
        );
        assert!(
            declared
                .validate_options(&options(r#"{"package": "acme"}"#))
                .is_ok()
        );
        assert_eq!(
            declared
                .validate_options(&options(r#"{"package": "acme", "verboze": true}"#))
                .unwrap_err(),
            "option \"verboze\" is not one of the options fixture declares: package, retries"
        );
        assert_eq!(
            declared
                .validate_options(&options(r#"{"retries": []}"#))
                .unwrap_err(),
            "option \"package\" is required by fixture",
            "a required option is missing"
        );
        assert_eq!(
            declared
                .validate_options(&options(r#"{"package": 3}"#))
                .unwrap_err(),
            "option \"package\" is a number, but fixture declares it as a string"
        );
    }

    #[test]
    fn every_declared_json_type_is_held_to_its_own() {
        let declared = schema(
            r#"{"options":{"package":{"type":"string"},"retries":{"type":"array"},"attempts":{"type":"integer"},"ratio":{"type":"number"},"on":{"type":"boolean"},"settings":{"type":"object"}}}"#,
        );
        for (key, kind) in [
            ("package", "string"),
            ("retries", "array"),
            ("attempts", "integer"),
            ("ratio", "number"),
            ("on", "boolean"),
            ("settings", "object"),
        ] {
            assert!(
                declared
                    .validate_options(&options(&format!(r#"{{"{key}": null}}"#)))
                    .is_err(),
                "null is not a {kind}"
            );
        }
        assert!(
            declared
                .validate_options(&options(r#"{"attempts": 3}"#))
                .is_ok()
        );
        assert!(
            declared
                .validate_options(&options(r#"{"ratio": 3}"#))
                .is_ok()
        );
        assert!(
            declared
                .validate_options(&options(r#"{"ratio": 1.5}"#))
                .is_ok(),
            "an integer is a number"
        );
        assert!(
            declared
                .validate_options(&options(r#"{"attempts": 1.5}"#))
                .is_err()
        );
        assert!(
            declared
                .validate_options(&options(r#"{"attempts": "3"}"#))
                .is_err()
        );
        assert_eq!(
            declared
                .validate_options(&options(r#"{"retries": "three"}"#))
                .unwrap_err(),
            "option \"retries\" is a string, but fixture declares it as an array"
        );
    }

    #[test]
    fn a_manifest_with_no_option_schema_takes_an_empty_options_object() {
        let plain = manifest();
        assert!(plain.validate_options(&BTreeMap::new()).is_ok());
        assert_eq!(
            plain
                .validate_options(&options(r#"{"package": "acme"}"#))
                .unwrap_err(),
            "fixture declares no options, so it takes an empty options object; \"package\" was \
             given"
        );
    }

    #[test]
    fn an_option_schema_is_read_the_way_the_manifest_is() {
        let declared: EmitterManifest = serde_json::from_value(serde_json::json!({
            "name": "python",
            "version": "1.0.0",
            "protocol": { "min": "3.0", "max": "3.0" },
            "ir_version": { "min": "0.2", "max": "0.2" },
            "option_schema": { "options": { "package": { "type": "string" } } }
        }))
        .unwrap();
        assert!(
            declared
                .validate_options(&options(r#"{"package": "acme"}"#))
                .is_ok()
        );
        assert!(
            declared
                .validate_options(&options(r#"{"package": 3}"#))
                .is_err()
        );

        for (source, expected) in [
            (
                serde_json::json!({ "option_schema": { "option": {} } }),
                "unknown field `option`",
            ),
            (
                serde_json::json!({
                    "option_schema": { "options": { "package": { "type": "string", "reqired": true } } }
                }),
                "unknown field `reqired`",
            ),
            (
                serde_json::json!({
                    "option_schema": { "options": { "package": { "kind": "string" } } }
                }),
                "unknown field `kind`",
            ),
            (
                serde_json::json!({
                    "option_schema": { "options": { "package": { "type": "text" } } }
                }),
                "unknown variant `text`",
            ),
        ] {
            let mut value = serde_json::to_value(manifest()).unwrap();
            value["protocol"] = serde_json::json!({ "min": "3.0", "max": "3.0" });
            for (key, replacement) in source.as_object().unwrap() {
                value[key] = replacement.clone();
            }
            let error = match serde_json::from_value::<EmitterManifest>(value) {
                Ok(manifest) => panic!("{source} must be refused, got {manifest:?}"),
                Err(error) => error.to_string(),
            };
            assert!(error.contains(expected), "{error}");
        }
    }

    #[test]
    fn protocol_v3_rejects_v2_manifests_and_responses() {
        let mut v2 = manifest();
        v2.protocol = VersionRange {
            min: "2.0".into(),
            max: "2.0".into(),
        };
        assert_eq!(
            v2.validate("0.2").unwrap_err(),
            "emitter fixture does not support protocol 3"
        );
        assert!(manifest().validate("0.2").is_ok());

        for protocol in [1, 2] {
            let response = EmitResponse {
                protocol,
                files: Vec::new(),
                diagnostics: Vec::new(),
                coverage: None,
                validation_coverage: None,
                error: None,
            };
            assert_eq!(
                validate_response(&response).unwrap_err(),
                format!("emitter returned protocol {protocol}, expected 3")
            );
        }
    }

    fn renamed(value: &serde_json::Value, key: &str, typo: &str) -> serde_json::Value {
        let mut renamed = value.clone();
        let object = renamed.as_object_mut().unwrap();
        let moved = object.remove(key).unwrap();
        object.insert(typo.into(), moved);
        renamed
    }

    fn assert_unknown_field<T: serde::de::DeserializeOwned + std::fmt::Debug>(
        value: &serde_json::Value,
        typo: &str,
        key: &str,
    ) {
        let error = serde_json::from_value::<T>(value.clone())
            .unwrap_err()
            .to_string();
        assert!(
            error.contains(&format!("unknown field `{typo}`")),
            "a misspelled {key} must name the unknown field, got {error}"
        );
    }

    #[test]
    fn a_misspelled_key_is_refused_on_every_wire_struct() {
        let response = serde_json::json!({
            "protocol": PROTOCOL_VERSION,
            "files": [{ "path": "client.ts", "content": "generated", "encoding": "utf8" }],
            "diagnostics": [{ "severity": "warning", "code": "X-1", "message": "note" }],
            "coverage": { "exact": [], "opaque": [], "lossy": [], "unsupported": [] },
            "validation_coverage": { "exact": [], "lossy": [], "unsupported": [] },
            "error": null
        });
        for (key, typo) in [
            ("protocol", "protokol"),
            ("files", "filesx"),
            ("diagnostics", "diagnostcs"),
            ("coverage", "coverag"),
            ("validation_coverage", "validation_coverag"),
            ("error", "eror"),
        ] {
            assert_unknown_field::<EmitResponse>(&renamed(&response, key, typo), typo, key);
        }
        assert_unknown_field::<Coverage>(
            &renamed(&response["coverage"], "exact", "exactt"),
            "exactt",
            "coverage.exact",
        );
        assert_unknown_field::<GeneratedFile>(
            &renamed(&response["files"][0], "content", "conten"),
            "conten",
            "files.content",
        );
        assert_unknown_field::<PluginDiagnostic>(
            &renamed(&response["diagnostics"][0], "message", "messag"),
            "messag",
            "diagnostics.message",
        );

        let manifest = serde_json::json!({
            "name": "fixture",
            "version": "1.0.0",
            "protocol": { "min": "3.0", "max": "3.0" },
            "ir_version": { "min": "0.2", "max": "0.2" },
            "capabilities": {
                "streaming": ["sse"],
                "validation": { "backends": ["pydantic"] }
            }
        });
        for (key, typo) in [
            ("protocol", "protokol"),
            ("ir_version", "ir_versoin"),
            ("capabilities", "capabilties"),
        ] {
            assert_unknown_field::<EmitterManifest>(&renamed(&manifest, key, typo), typo, key);
        }
        assert_unknown_field::<VersionRange>(
            &renamed(&manifest["protocol"], "min", "minimum"),
            "minimum",
            "protocol.min",
        );
        assert_unknown_field::<EmitterCapabilities>(
            &renamed(&manifest["capabilities"], "streaming", "streamin"),
            "streamin",
            "capabilities.streaming",
        );
        assert_unknown_field::<ValidationCapabilities>(
            &renamed(
                &manifest["capabilities"]["validation"],
                "backends",
                "backendz",
            ),
            "backendz",
            "capabilities.validation.backends",
        );
    }

    #[test]
    fn capability_lists_reject_unknown_values_and_duplicates() {
        for (list, unknown, duplicate) in [
            ("streaming", "grpc", "sse"),
            ("enum_reprs", "internl", "string"),
            ("object_openness", "preserved", "reject"),
        ] {
            let error = manifest_with(list, serde_json::json!([unknown]))
                .validate("0.2")
                .unwrap_err();
            assert_eq!(
                error,
                format!("emitter capabilities name an unknown {list} value {unknown}")
            );

            let error = manifest_with(list, serde_json::json!([duplicate, duplicate]))
                .validate("0.2")
                .unwrap_err();
            assert_eq!(
                error,
                format!("emitter capabilities list {duplicate} more than once in {list}")
            );
        }

        let every_value = manifest_with(
            "streaming",
            serde_json::json!(["sse", "ndjson", "bytes", "websocket"]),
        );
        assert!(every_value.validate("0.2").is_ok());

        let error = manifest_with("streaming", serde_json::json!(["stream_body"]))
            .validate("0.2")
            .unwrap_err();
        assert_eq!(
            error, "emitter capabilities name an unknown streaming value stream_body",
            "a request body the IR refuses to describe has no capability to declare"
        );
    }

    fn manifest_with(list: &str, values: serde_json::Value) -> EmitterManifest {
        let mut value = serde_json::to_value(manifest()).unwrap();
        value["protocol"] = serde_json::json!({ "min": "3.0", "max": "3.0" });
        value["capabilities"][list] = values;
        serde_json::from_value(value).unwrap()
    }

    #[test]
    fn a_direction_pair_needs_a_direction_split_target() {
        let mut value: serde_json::Value =
            serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
        for id in ["User", "LoginRequest"] {
            value["types"][id]["origin"] = serde_json::json!("account");
        }
        let document: Document = serde_json::from_value(value).unwrap();
        document.validate().unwrap();

        let mut manifest = manifest();
        assert_eq!(
            manifest
                .unsupported_features(&document)
                .iter()
                .map(|warning| warning.0.as_str())
                .filter(|message| message.contains("direction-split"))
                .collect::<Vec<_>>(),
            vec!["fixture does not declare direction-split type support for account"],
            "one origin group holds an input type and an output type"
        );
        manifest.capabilities.direction_split = true;
        assert!(
            !manifest
                .unsupported_features(&document)
                .iter()
                .any(|warning| warning.0.contains("direction-split"))
        );
    }

    #[test]
    fn a_both_direction_type_asks_for_no_direction_split() {
        let mut value: serde_json::Value =
            serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
        for id in ["User", "LoginRequest"] {
            value["types"][id]["origin"] = serde_json::json!("account");
        }
        value["types"]["User"]["direction"] = serde_json::json!("both");
        let document: Document = serde_json::from_value(value).unwrap();
        document.validate().unwrap();

        assert!(
            !manifest()
                .unsupported_features(&document)
                .iter()
                .any(|warning| warning.0.contains("direction-split")),
            "a both-direction type is the whole origin group and serves both sides itself"
        );
    }

    #[test]
    fn coverage_must_cover_every_subject_once_and_respect_mode() {
        let document: Document =
            serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
        let response = EmitResponse {
            protocol: PROTOCOL_VERSION,
            files: Vec::new(),
            diagnostics: Vec::new(),
            coverage: Some(coverage_for_document(&document, false).unwrap()),
            validation_coverage: None,
            error: None,
        };
        assert!(
            validate_response_for_document(&response, &document, EmitMode::Strict, true, false)
                .is_ok()
        );

        let mut missing = response.clone();
        missing.coverage = None;
        assert!(
            validate_response_for_document(&missing, &document, EmitMode::Strict, true, false)
                .unwrap_err()
                .contains("missing coverage")
        );

        let mut duplicate = response.clone();
        duplicate
            .coverage
            .as_mut()
            .unwrap()
            .opaque
            .insert("types.EventQuery".into());
        assert!(
            validate_response_for_document(&duplicate, &document, EmitMode::Strict, true, false)
                .unwrap_err()
                .contains("more than once")
        );

        let mut opaque = response.clone();
        let coverage = opaque.coverage.as_mut().unwrap();
        coverage.exact.remove("types.EventQuery");
        coverage.opaque.insert("types.EventQuery".into());
        assert!(
            validate_response_for_document(&opaque, &document, EmitMode::Strict, false, false)
                .unwrap_err()
                .contains("opaque coverage without declaring opaque support")
        );
        assert!(
            validate_response_for_document(&opaque, &document, EmitMode::Strict, true, false)
                .is_ok()
        );
        assert!(
            validate_response_for_document(&opaque, &document, EmitMode::Lossy, false, false)
                .is_ok()
        );

        let mut unsupported = response.clone();
        let coverage = unsupported.coverage.as_mut().unwrap();
        coverage.exact.remove("types.EventQuery");
        coverage.unsupported.insert("types.EventQuery".into());
        assert!(
            validate_response_for_document(&unsupported, &document, EmitMode::Strict, true, false)
                .unwrap_err()
                .contains("unsupported coverage")
        );
        assert!(
            validate_response_for_document(&unsupported, &document, EmitMode::Lossy, true, false)
                .is_ok()
        );

        let mut lossy = response;
        let coverage = lossy.coverage.as_mut().unwrap();
        coverage.exact.remove("types.EventQuery");
        coverage.lossy.insert("types.EventQuery".into());
        assert!(
            validate_response_for_document(&lossy, &document, EmitMode::Strict, true, false)
                .unwrap_err()
                .contains("lossy coverage")
        );
        assert!(
            validate_response_for_document(&lossy, &document, EmitMode::Lossy, true, false).is_ok()
        );
    }

    #[test]
    fn validation_coverage_covers_every_subject_once_and_rejects_opaque() {
        let document: Document =
            serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
        let mut validation = ValidationCoverage::exact_for_document(&document);
        validation.exact.remove("types.EventQuery");
        validation.lossy.insert("types.EventQuery".into());
        validation.exact.remove("types.LoginError");
        validation.unsupported.insert("types.LoginError".into());
        let response = EmitResponse {
            protocol: PROTOCOL_VERSION,
            files: Vec::new(),
            diagnostics: Vec::new(),
            coverage: Some(coverage_for_document(&document, false).unwrap()),
            validation_coverage: Some(validation),
            error: None,
        };
        assert_eq!(
            validate_response_for_document(&response, &document, EmitMode::Strict, true, true)
                .unwrap_err(),
            "emitter reported unsupported validation coverage in strict mode: types.LoginError; \
             lossy validation coverage in strict mode: types.EventQuery"
        );
        assert!(
            validate_response_for_document(&response, &document, EmitMode::Lossy, true, true)
                .is_ok()
        );

        let mut missing = response.clone();
        missing
            .validation_coverage
            .as_mut()
            .unwrap()
            .exact
            .remove("types.User");
        assert!(
            validate_response_for_document(&missing, &document, EmitMode::Lossy, true, true)
                .unwrap_err()
                .contains("missing subjects [\"types.User\"]")
        );

        let mut duplicate = response.clone();
        duplicate
            .validation_coverage
            .as_mut()
            .unwrap()
            .lossy
            .insert("types.User".into());
        assert!(
            validate_response_for_document(&duplicate, &document, EmitMode::Lossy, true, true)
                .unwrap_err()
                .contains("more than once")
        );

        let mut absent = response;
        absent.validation_coverage = None;
        assert_eq!(
            validate_response_for_document(&absent, &document, EmitMode::Lossy, true, true)
                .unwrap_err(),
            "emitter response is missing validation coverage while validation is enabled"
        );
        assert!(
            validate_response_for_document(&absent, &document, EmitMode::Lossy, true, false)
                .is_ok()
        );

        let error = serde_json::from_value::<EmitResponse>(serde_json::json!({
            "protocol": PROTOCOL_VERSION,
            "validation_coverage": { "opaque": ["types.User"] }
        }))
        .unwrap_err();
        assert!(
            error.to_string().contains("unknown field `opaque`"),
            "{error}"
        );
    }

    #[test]
    fn validation_capabilities_name_backends_and_levels() {
        let supported = validation_manifest(
            &["pydantic"],
            &[
                ("validation.scalars", "exact"),
                ("validation.formats", "lossy"),
                ("validation.stream", "unsupported"),
            ],
        );
        assert!(supported.validate("0.2").is_ok());
        let custom = validation_manifest(&[], &[("validation.custom_thing", "exact")]);
        assert!(
            custom.validate("0.2").is_ok(),
            "a capability list is not fixed"
        );

        assert_eq!(
            validation_manifest(&["pydantic"], &[("validation.scalars", "opaque")])
                .validate("0.2")
                .unwrap_err(),
            "emitter validation capability validation.scalars has level \"opaque\"; it is exact, \
             lossy, or unsupported"
        );
        assert_eq!(
            validation_manifest(&[], &[("enum.external", "exact")])
                .validate("0.2")
                .unwrap_err(),
            "emitter validation capability \"enum.external\" is not a validation.* id"
        );
        assert_eq!(
            validation_manifest(&["py dantic"], &[])
                .validate("0.2")
                .unwrap_err(),
            "emitter validation backend \"py dantic\" is not a plain backend id"
        );
        assert_eq!(
            validation_manifest(&["pydantic", "pydantic"], &[])
                .validate("0.2")
                .unwrap_err(),
            "emitter validation lists the backend pydantic more than once"
        );
    }

    #[test]
    fn a_validation_option_needs_a_declared_backend_and_capability() {
        let requested = options(r#"{"validation":{"backend":"pydantic"}}"#);
        assert!(validation_enabled(&requested));
        assert!(
            manifest()
                .validation_gap(&requested)
                .unwrap()
                .0
                .contains("does not declare validation support for the pydantic backend")
        );

        let supported = validation_manifest(&["pydantic"], &[("validation.scalars", "exact")]);
        assert!(supported.validation_gap(&requested).is_none());
        assert!(
            supported
                .validation_gap(&options(r#"{"validation":{"backend":"none"}}"#))
                .is_none()
        );
        assert!(supported.validation_gap(&BTreeMap::new()).is_none());
        assert!(!validation_enabled(&options(
            r#"{"validation":{"backend":"none"}}"#
        )));

        let wrong = validation_manifest(&["zod"], &[("validation.scalars", "exact")]);
        assert_eq!(
            wrong.validation_gap(&requested).unwrap().0,
            "fixture does not declare the pydantic validation backend"
        );
        let no_capabilities = validation_manifest(&["pydantic"], &[]);
        assert_eq!(
            no_capabilities.validation_gap(&requested).unwrap().0,
            "fixture declares no validation capabilities"
        );
    }

    fn validation_manifest(backends: &[&str], capabilities: &[(&str, &str)]) -> EmitterManifest {
        let mut manifest = manifest();
        manifest.capabilities.validation = ValidationCapabilities {
            backends: backends.iter().map(|backend| (*backend).into()).collect(),
            capabilities: capabilities
                .iter()
                .map(|(id, level)| ((*id).into(), (*level).into()))
                .collect(),
        };
        manifest
    }

    #[test]
    fn reported_coverage_may_not_beat_canonical_coverage() {
        let document: Document =
            serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
        let canonical = coverage_for_document(&document, false).unwrap();
        assert!(canonical.opaque.contains("types.User"));

        let mut overclaimed = canonical.clone();
        overclaimed.opaque.remove("types.User");
        overclaimed.exact.insert("types.User".into());
        let response = EmitResponse {
            protocol: PROTOCOL_VERSION,
            files: Vec::new(),
            diagnostics: Vec::new(),
            coverage: Some(overclaimed),
            validation_coverage: None,
            error: None,
        };
        let error = "emitter reported types.User as exact coverage, better than the canonical \
                     opaque coverage";
        for mode in [EmitMode::Strict, EmitMode::Lossy] {
            assert_eq!(
                validate_response_for_document(&response, &document, mode, true, false)
                    .unwrap_err(),
                error,
                "{mode:?}"
            );
        }
    }

    #[test]
    fn canonical_coverage_permits_lowering_a_subject_to_lossy() {
        let document: Document =
            serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
        let mut coverage = coverage_for_document(&document, false).unwrap();
        coverage.opaque.remove("types.User");
        coverage.lossy.insert("types.User".into());
        let response = EmitResponse {
            protocol: PROTOCOL_VERSION,
            files: Vec::new(),
            diagnostics: Vec::new(),
            coverage: Some(coverage),
            validation_coverage: None,
            error: None,
        };
        assert_eq!(
            validate_response_for_document(&response, &document, EmitMode::Strict, true, false)
                .unwrap_err(),
            "emitter reported lossy coverage in strict mode: types.User"
        );
        assert!(
            validate_response_for_document(&response, &document, EmitMode::Lossy, true, false)
                .is_ok()
        );
    }

    #[test]
    fn strict_mode_names_every_violating_class_with_its_subjects() {
        let document: Document =
            serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
        let mut coverage = coverage_for_document(&document, false).unwrap();
        for (subject, level) in [
            ("types.EventQuery", CoverageLevel::Lossy),
            ("types.LoginError", CoverageLevel::Unsupported),
        ] {
            coverage.exact.remove(subject);
            match level {
                CoverageLevel::Lossy => {
                    coverage.lossy.insert(subject.into());
                }
                _ => {
                    coverage.unsupported.insert(subject.into());
                }
            }
        }
        let response = EmitResponse {
            protocol: PROTOCOL_VERSION,
            files: Vec::new(),
            diagnostics: Vec::new(),
            coverage: Some(coverage),
            validation_coverage: None,
            error: None,
        };
        assert_eq!(
            validate_response_for_document(&response, &document, EmitMode::Strict, true, false)
                .unwrap_err(),
            "emitter reported unsupported coverage in strict mode: types.LoginError; lossy \
             coverage in strict mode: types.EventQuery",
            "the worst class comes first and every subject is named"
        );
        assert!(
            validate_response_for_document(&response, &document, EmitMode::Lossy, true, false)
                .is_ok()
        );
    }

    #[test]
    fn the_plugin_floor_follows_the_mode_the_built_in_emits_with() {
        let mut value: serde_json::Value =
            serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
        value["types"]["User"]["fields"][0]["ty"] = serde_json::json!({
            "kind": "int", "bits": 64, "signed": false, "wire": "number"
        });
        let document: Document = serde_json::from_value(value).unwrap();
        document.validate().unwrap();

        let mut certified = EmitResponse {
            protocol: PROTOCOL_VERSION,
            files: Vec::new(),
            diagnostics: Vec::new(),
            coverage: Some(coverage_for_document(&document, true).unwrap()),
            validation_coverage: None,
            error: None,
        };
        assert!(
            validate_response_for_document(&certified, &document, EmitMode::Lossy, true, false)
                .is_ok(),
            "the built-in emitter's own lossy-mode coverage is accepted"
        );
        assert!(
            validate_response_for_document(&certified, &document, EmitMode::Strict, true, false)
                .is_err(),
            "the same coverage is lossy in strict mode"
        );

        let coverage = certified.coverage.as_mut().unwrap();
        coverage.lossy.remove("types.User");
        coverage.exact.insert("types.User".into());
        assert_eq!(
            validate_response_for_document(&certified, &document, EmitMode::Lossy, true, false)
                .unwrap_err(),
            "emitter reported types.User as exact coverage, better than the canonical lossy \
             coverage"
        );
    }

    #[test]
    fn plugin_validation_follows_the_canonical_coverage_of_the_fixture() {
        let document: Document =
            serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
        let mut manifest = manifest();
        assert!(
            manifest
                .unsupported_features(&document)
                .iter()
                .any(|warning| warning.0.contains("opaque value support")),
            "preserving openness is canonical opaque coverage"
        );
        manifest.capabilities.opaque = true;
        assert!(
            !manifest
                .unsupported_features(&document)
                .iter()
                .any(|warning| warning.0.contains("opaque value support"))
        );

        let coverage = coverage_for_document(&document, false).unwrap();
        let response = EmitResponse {
            protocol: PROTOCOL_VERSION,
            files: Vec::new(),
            diagnostics: Vec::new(),
            coverage: Some(coverage.clone()),
            validation_coverage: None,
            error: None,
        };
        assert!(
            validate_response_for_document(&response, &document, EmitMode::Strict, true, false)
                .is_ok()
        );
        assert!(
            validate_response_for_document(&response, &document, EmitMode::Lossy, false, false)
                .is_ok()
        );
        assert_eq!(
            validate_response_for_document(&response, &document, EmitMode::Strict, false, false)
                .unwrap_err(),
            "emitter reported opaque coverage without declaring opaque support"
        );

        for (level, error) in [
            (
                "lossy",
                "emitter reported lossy coverage in strict mode: types.EventQuery",
            ),
            (
                "unsupported",
                "emitter reported unsupported coverage in strict mode: types.EventQuery",
            ),
        ] {
            let mut coverage = coverage.clone();
            coverage.exact.remove("types.EventQuery");
            let target = match level {
                "lossy" => &mut coverage.lossy,
                _ => &mut coverage.unsupported,
            };
            target.insert("types.EventQuery".into());
            let narrowed = EmitResponse {
                protocol: PROTOCOL_VERSION,
                files: Vec::new(),
                diagnostics: Vec::new(),
                coverage: Some(coverage),
                validation_coverage: None,
                error: None,
            };
            assert_eq!(
                validate_response_for_document(&narrowed, &document, EmitMode::Strict, true, false)
                    .unwrap_err(),
                error,
                "{level}"
            );
            assert!(
                validate_response_for_document(&narrowed, &document, EmitMode::Lossy, true, false)
                    .is_ok(),
                "{level}"
            );
        }
    }

    #[test]
    fn document_coverage_propagates_opaque_and_lossy_features() {
        let mut opaque: Document =
            serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
        let user = opaque.types.get_mut("User").unwrap();
        let TypeKind::Struct { fields, .. } = &mut user.definition else {
            panic!()
        };
        fields[0].ty = TypeRef::Inline(InlineType::Opaque {
            codec: "example".into(),
            wire: pons_ir::OpaqueWire::Json,
            ext: Default::default(),
        });
        let coverage = coverage_for_document(&opaque, false).unwrap();
        assert!(coverage.opaque.contains("types.User"));
        assert!(coverage.opaque.contains("operations.auth.login"));
        assert!(coverage.opaque.contains("operations.events.watch"));

        let mut wide: Document =
            serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
        let user = wide.types.get_mut("User").unwrap();
        let TypeKind::Struct { fields, .. } = &mut user.definition else {
            panic!()
        };
        let TypeRef::Inline(InlineType::Int { wire, .. }) = &mut fields[0].ty else {
            panic!()
        };
        *wire = pons_ir::IntegerWire::Number;
        let coverage = coverage_for_document(&wide, true).unwrap();
        assert!(coverage.lossy.contains("types.User"));
        assert!(coverage.lossy.contains("operations.auth.login"));
        assert!(coverage.lossy.contains("operations.events.watch"));
    }

    fn exactly_representable_fixture() -> serde_json::Value {
        let mut raw: serde_json::Value =
            serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
        for id in ["User", "LoginRequest"] {
            raw["types"][id]["openness"] = serde_json::json!("reject");
        }
        raw
    }

    #[test]
    fn coverage_ignores_extensions_and_unselected_enum_variants() {
        let mut raw = exactly_representable_fixture();
        raw["types"]["User"]["fields"][1]["default"] = serde_json::json!({ "kind": "opaque" });
        let defaults: Document = serde_json::from_value(raw).unwrap();
        let coverage = coverage_for_document(&defaults, false).unwrap();
        assert!(coverage.exact.contains("types.User"));
        assert!(!coverage.opaque.contains("types.User"));

        let mut document: Document =
            serde_json::from_value(exactly_representable_fixture()).unwrap();
        document.types.get_mut("User").unwrap().ext.insert(
            "com.example.marker/v1".into(),
            serde_json::json!({ "kind": "opaque" }),
        );
        let coverage = coverage_for_document(&document, false).unwrap();
        assert!(coverage.exact.contains("types.User"));
        assert!(!coverage.opaque.contains("types.User"));

        let login_error = document.types.get_mut("LoginError").unwrap();
        let TypeKind::Enum { variants, .. } = &mut login_error.definition else {
            panic!()
        };
        let locked = variants
            .iter_mut()
            .find(|variant| variant.wire == "locked")
            .unwrap();
        let pons_ir::VariantPayload::Struct { fields, .. } = &mut locked.payload else {
            panic!()
        };
        fields[0].ty = TypeRef::Inline(InlineType::Opaque {
            codec: "example".into(),
            wire: pons_ir::OpaqueWire::Json,
            ext: Default::default(),
        });
        let coverage = coverage_for_document(&document, false).unwrap();
        assert!(coverage.opaque.contains("types.LoginError"));
        assert!(!coverage.opaque.contains("operations.auth.login"));
    }

    #[test]
    fn recursive_type_graphs_keep_reachable_opaque_and_lossy_levels() {
        let opaque = coverage_for_document(
            &recursive_document(serde_json::json!({
                "kind": "opaque",
                "codec": "example",
                "wire": "json"
            })),
            false,
        )
        .unwrap();
        assert!(opaque.opaque.contains("types.CycleHead"));
        assert!(opaque.opaque.contains("types.CycleLink"));
        assert!(opaque.opaque.contains("operations.events.watch"));
        assert!(!opaque.exact.contains("operations.events.watch"));

        let lossy = coverage_for_document(
            &recursive_document(serde_json::json!({
                "kind": "int",
                "bits": 64,
                "signed": false,
                "wire": "number"
            })),
            true,
        )
        .unwrap();
        assert!(lossy.lossy.contains("types.CycleHead"));
        assert!(lossy.lossy.contains("operations.events.watch"));
        assert!(!lossy.exact.contains("operations.events.watch"));
    }

    #[test]
    fn selected_enum_variants_keep_nested_classifications() {
        let mut value: serde_json::Value =
            serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
        value["types"]["Chain"] = serde_json::json!({
            "kind": "enum",
            "name": ["chain"],
            "direction": "both",
            "repr": { "kind": "internal", "tag": "code" },
            "variants": [
                {
                    "wire": "next",
                    "ident": ["next"],
                    "payload": {
                        "kind": "struct",
                        "fields": [
                            {
                                "wire": "again",
                                "ident": ["again"],
                                "ty": { "ref": "Chain", "variant": "next" },
                                "presence": "required"
                            },
                            {
                                "wire": "leaf",
                                "ident": ["leaf"],
                                "ty": { "ref": "Chain", "variant": "leaf" },
                                "presence": "required"
                            }
                        ]
                    }
                },
                {
                    "wire": "leaf",
                    "ident": ["leaf"],
                    "payload": {
                        "kind": "struct",
                        "fields": [{
                            "wire": "secret",
                            "ident": ["secret"],
                            "ty": { "kind": "opaque", "codec": "example", "wire": "json" },
                            "presence": "required"
                        }]
                    }
                }
            ]
        });
        value["operations"][1]["errors"] = serde_json::json!([{
            "status": "4xx",
            "name": ["chain", "error"],
            "content": "json",
            "body": { "ref": "Chain", "variant": "next" }
        }]);
        let document: Document = serde_json::from_value(value).unwrap();
        let coverage = coverage_for_document(&document, false).unwrap();
        assert!(coverage.opaque.contains("types.Chain"));
        assert!(coverage.opaque.contains("operations.events.watch"));
        assert!(!coverage.exact.contains("operations.events.watch"));
    }

    #[test]
    fn coverage_rejects_duplicate_subjects_within_one_level() {
        for level in ["exact", "opaque", "lossy", "unsupported"] {
            let mut coverage = serde_json::Map::new();
            coverage.insert(
                level.into(),
                serde_json::json!(["types.User", "types.User"]),
            );
            let error = serde_json::from_value::<EmitResponse>(serde_json::json!({
                "protocol": PROTOCOL_VERSION,
                "coverage": coverage
            }))
            .unwrap_err();
            assert!(error.to_string().contains("more than once"), "{level}");
        }

        let document: Document =
            serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
        let canonical = coverage_for_document(&document, false).unwrap();
        let response: EmitResponse = serde_json::from_value(serde_json::json!({
            "protocol": PROTOCOL_VERSION,
            "coverage": serde_json::to_value(&canonical).unwrap()
        }))
        .unwrap();
        assert!(
            validate_response_for_document(&response, &document, EmitMode::Strict, true, false)
                .is_ok()
        );

        let mut repeated: Vec<String> = pons_ir::coverage_subjects(&document).into_iter().collect();
        repeated.push("types.User".into());
        let error = serde_json::from_value::<EmitResponse>(serde_json::json!({
            "protocol": PROTOCOL_VERSION,
            "coverage": { "exact": repeated }
        }))
        .unwrap_err();
        assert!(error.to_string().contains("more than once"));
    }

    fn recursive_document(leaf: serde_json::Value) -> Document {
        let mut value: serde_json::Value =
            serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
        for (id, name, fields) in [
            (
                "CycleHead",
                "head",
                vec![("next", serde_json::json!({ "ref": "CycleLink" }))],
            ),
            (
                "CycleLink",
                "link",
                vec![
                    ("back", serde_json::json!({ "ref": "CycleHead" })),
                    ("leaf", serde_json::json!({ "ref": "CycleLeaf" })),
                ],
            ),
            ("CycleLeaf", "leaf", vec![("value", leaf)]),
        ] {
            value["types"][id] = serde_json::json!({
                "kind": "struct",
                "openness": "reject",
                "name": ["cycle", name],
                "direction": "both",
                "fields": fields
                    .into_iter()
                    .map(|(wire, ty)| serde_json::json!({
                        "wire": wire,
                        "ident": [wire],
                        "ty": ty,
                        "presence": "required"
                    }))
                    .collect::<Vec<_>>()
            });
        }
        value["operations"][1]["responses"][0]["body"] = serde_json::json!({ "ref": "CycleHead" });
        serde_json::from_value(value).unwrap()
    }

    #[test]
    fn operation_paths_do_not_collide() {
        let mut document: Document =
            serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
        document.operations[0].id = vec![vec!["a".into()], vec!["b".into()]];
        document.operations[1].id = vec![vec!["a.b".into()]];
        assert_ne!(
            operation_subject(&document.operations[0]),
            operation_subject(&document.operations[1])
        );
    }

    #[test]
    fn rejects_unsafe_or_duplicate_output_paths() {
        for path in [
            "",
            "/absolute",
            "../escape",
            "nested/../escape",
            "C:/escape",
            "nested\\escape",
            "client.ts:hidden",
            "nested/client.ts:hidden",
            "NUL",
            "con",
            "nested/Prn",
            "AUX",
            "COM1",
            "com1.ts",
            "nested/LPT9",
            "nul.ts/nested",
        ] {
            let response = EmitResponse {
                protocol: PROTOCOL_VERSION,
                files: vec![file(path)],
                diagnostics: Vec::new(),
                coverage: None,
                validation_coverage: None,
                error: None,
            };
            assert!(validate_response(&response).is_err(), "accepted {path:?}");
        }

        for path in [
            "client.ts",
            "console.ts",
            "nullable/auxiliary.ts",
            "COM10",
            "LPT0.ts",
        ] {
            let response = EmitResponse {
                protocol: PROTOCOL_VERSION,
                files: vec![file(path)],
                diagnostics: Vec::new(),
                coverage: None,
                validation_coverage: None,
                error: None,
            };
            assert!(validate_response(&response).is_ok(), "refused {path:?}");
        }

        let response = EmitResponse {
            protocol: PROTOCOL_VERSION,
            files: vec![file("client.ts"), file("client.ts")],
            diagnostics: Vec::new(),
            coverage: None,
            validation_coverage: None,
            error: None,
        };
        assert!(validate_response(&response).is_err());
    }

    #[test]
    fn windows_device_names_survive_superscripts_and_trailing_spaces() {
        for path in [
            "com¹",
            "COM³",
            "com2.txt",
            "LPT¹ ",
            "lpt³.log ",
            "NUL ",
            "con  ",
            "aux ",
        ] {
            let response = EmitResponse {
                protocol: PROTOCOL_VERSION,
                files: vec![file(path)],
                diagnostics: Vec::new(),
                coverage: None,
                validation_coverage: None,
                error: None,
            };
            assert!(validate_response(&response).is_err(), "accepted {path:?}");
        }

        for path in ["com¹⁰", "lpt1a", "null", "com10 ", "com⁴"] {
            let response = EmitResponse {
                protocol: PROTOCOL_VERSION,
                files: vec![file(path)],
                diagnostics: Vec::new(),
                coverage: None,
                validation_coverage: None,
                error: None,
            };
            assert!(validate_response(&response).is_ok(), "refused {path:?}");
        }
    }

    fn file(path: &str) -> GeneratedFile {
        GeneratedFile {
            path: path.into(),
            content: String::new(),
            encoding: "utf8".into(),
        }
    }
}
