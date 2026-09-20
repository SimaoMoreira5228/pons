use std::collections::BTreeMap;
use std::path::{Component, Path};

use pons_ir::{Document, EnumRepr, StreamKind, TypeKind};
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const PROTOCOL_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VersionRange {
    pub min: String,
    pub max: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EmitterManifest {
    pub name: String,
    pub version: String,
    pub protocol: VersionRange,
    pub ir_version: VersionRange,
    #[serde(default)]
    pub capabilities: EmitterCapabilities,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
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
    pub flattened: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct EmitRequest<'a> {
    pub protocol: u32,
    pub ir: &'a Document,
    pub options: BTreeMap<String, Value>,
    pub target: &'a str,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
pub struct EmitResponse {
    pub protocol: u32,
    #[serde(default)]
    pub files: Vec<GeneratedFile>,
    #[serde(default)]
    pub diagnostics: Vec<PluginDiagnostic>,
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
        Ok(())
    }

    pub fn unsupported_features(&self, document: &Document) -> Vec<CapabilityWarning> {
        let mut warnings = Vec::new();
        for operation in &document.operations {
            if let Some(stream) = &operation.stream {
                let kind = match stream.kind {
                    StreamKind::Sse { .. } => "sse",
                    StreamKind::Ndjson { .. } => "ndjson",
                    StreamKind::Bytes { .. } => "bytes",
                    StreamKind::Websocket { .. } => "websocket",
                };
                if !self
                    .capabilities
                    .streaming
                    .iter()
                    .any(|value| value == kind)
                {
                    warnings.push(CapabilityWarning(format!(
                        "{} does not declare support for {kind} streams in {}",
                        self.name,
                        operation_path(operation)
                    )));
                }
            }
            if operation.provides.is_some() && !self.capabilities.provides {
                warnings.push(CapabilityWarning(format!(
                    "{} does not declare credential capture support in {}",
                    self.name,
                    operation_path(operation)
                )));
            }
        }
        let mut directions = BTreeMap::<&str, (bool, bool)>::new();
        for definition in document.types.values() {
            if let Some(origin) = definition.origin.as_deref() {
                let entry = directions.entry(origin).or_default();
                match definition.direction {
                    pons_ir::Direction::Input => entry.0 = true,
                    pons_ir::Direction::Output => entry.1 = true,
                    pons_ir::Direction::Both => {}
                }
            }
        }
        if !self.capabilities.direction_split {
            for (origin, (input, output)) in directions {
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
                TypeKind::Struct { flattened, .. }
                    if !flattened.is_empty() && !self.capabilities.flattened =>
                {
                    warnings.push(CapabilityWarning(format!(
                        "{} does not declare flattened struct support in {id}",
                        self.name
                    )));
                }
                _ => {}
            }
        }
        warnings
    }
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

fn validate_output_path(path: &str) -> Result<(), String> {
    let candidate = Path::new(path);
    if path.is_empty()
        || path.contains('\\')
        || path
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
        || path
            .split('/')
            .next()
            .is_some_and(|part| part.contains(':'))
        || candidate.is_absolute()
        || candidate
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(format!("emitter returned unsafe output path {path:?}"));
    }
    Ok(())
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

fn operation_path(operation: &pons_ir::Operation) -> String {
    operation
        .id
        .iter()
        .map(|name| name.join("_"))
        .collect::<Vec<_>>()
        .join(".")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_unsafe_or_duplicate_output_paths() {
        for path in [
            "",
            "/absolute",
            "../escape",
            "nested/../escape",
            "C:/escape",
            "nested\\escape",
        ] {
            let response = EmitResponse {
                protocol: PROTOCOL_VERSION,
                files: vec![file(path)],
                diagnostics: Vec::new(),
                error: None,
            };
            assert!(validate_response(&response).is_err(), "accepted {path:?}");
        }

        let response = EmitResponse {
            protocol: PROTOCOL_VERSION,
            files: vec![file("client.ts"), file("client.ts")],
            diagnostics: Vec::new(),
            error: None,
        };
        assert!(validate_response(&response).is_err());
    }

    fn file(path: &str) -> GeneratedFile {
        GeneratedFile {
            path: path.into(),
            content: String::new(),
            encoding: "utf8".into(),
        }
    }
}
