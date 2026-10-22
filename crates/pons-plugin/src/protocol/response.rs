use std::collections::BTreeSet;
use std::path::{Component, Path};

use pons_ir::{Coverage, Document, ValidationCoverage};
use serde::{Deserialize, Serialize};

use super::diagnostic::PluginDiagnostic;
use super::{EmitMode, PROTOCOL_VERSION};

const DEVICE_INDEXES: &[&str] = &["1", "2", "3", "4", "5", "6", "7", "8", "9", "¹", "²", "³"];

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

pub fn validate_response(response: &EmitResponse, protocol: u32) -> Result<(), String> {
    if response.protocol != protocol {
        return Err(format!(
            "emitter returned protocol {}, expected {protocol}",
            response.protocol
        ));
    }
    if protocol < PROTOCOL_VERSION && response.validation_coverage.is_some() {
        return Err(format!(
            "protocol {protocol} emitter cannot return validation coverage"
        ));
    }
    validate_diagnostics(&response.diagnostics)?;
    if let Some(error) = &response.error {
        return Err(error.clone());
    }
    let mut paths = BTreeSet::new();
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
    protocol: u32,
    opaque_supported: bool,
    validation_enabled: bool,
) -> Result<(), String> {
    validate_response(response, protocol)?;
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
        && let Some(violations) = pons_ir::strict_violation_message(coverage)
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

fn validate_diagnostics(diagnostics: &[PluginDiagnostic]) -> Result<(), String> {
    for diagnostic in diagnostics {
        if let Some(capability) = &diagnostic.capability
            && !is_capability_id(capability)
        {
            return Err(format!(
                "emitter diagnostic {} names an invalid capability id {capability:?}",
                diagnostic.code
            ));
        }
    }
    Ok(())
}

fn is_capability_id(id: &str) -> bool {
    !id.is_empty()
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b'/'))
        && id
            .bytes()
            .next()
            .is_some_and(|byte| byte.is_ascii_alphanumeric())
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
