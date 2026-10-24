mod operations;
mod responses;
mod schema;
mod security;

use std::collections::{BTreeMap, BTreeSet};

use pons_ir::{
    Coverage, Direction, Document, TypeDef, coverage_for_document, operation_subject,
    strict_violation_message,
};
use serde_json::{Map, Value, json};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmitError(pub String);

impl std::fmt::Display for EmitError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for EmitError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmitMode {
    Strict,
    Lossy,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmitOutput {
    pub content: String,
    pub warnings: Vec<String>,
    pub coverage: Coverage,
}

struct Builder<'a> {
    document: &'a Document,
    schema_names: BTreeMap<String, String>,
    security_names: BTreeMap<String, String>,
    selected_variants: std::cell::RefCell<BTreeSet<(String, String)>>,
}

pub fn emit(document: &Document) -> Result<String, EmitError> {
    emit_with_mode(document, EmitMode::Strict).map(|output| output.content)
}

pub fn emit_with_mode(document: &Document, mode: EmitMode) -> Result<EmitOutput, EmitError> {
    document.validate().map_err(|errors| {
        EmitError(
            errors
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join("\n"),
        )
    })?;
    let mut document = document.clone();
    document.canonicalize();
    let builder = Builder {
        document: &document,
        schema_names: unique_names(
            document
                .types
                .iter()
                .map(|(id, ty)| (pair_name(&document, ty), is_split_half(ty), id.clone()))
                .collect(),
        ),
        security_names: unique_names(
            document
                .auth_schemes
                .keys()
                .map(|id| (readable_name(std::slice::from_ref(id)), false, id.clone()))
                .collect(),
        ),
        selected_variants: std::cell::RefCell::new(BTreeSet::new()),
    };
    let mut output = Map::new();
    output.insert("openapi".into(), json!("3.1.1"));
    let mut info = Map::new();
    info.insert("title".into(), json!(document.info.title));
    info.insert("version".into(), json!(document.info.version));
    if let Some(description) = &document.info.description {
        info.insert("description".into(), json!(description));
    }
    if !document.info.ext.is_empty() {
        info.insert("x-pons".into(), json!({"ext":document.info.ext}));
    }
    output.insert("info".into(), Value::Object(info));
    if !document.info.servers.is_empty() {
        output.insert(
            "servers".into(),
            Value::Array(
                document
                    .info
                    .servers
                    .iter()
                    .map(|url| json!({"url": url}))
                    .collect(),
            ),
        );
    }
    output.insert("paths".into(), builder.paths()?);

    let schemas = builder.schemas()?;
    let security_schemes = builder.security_schemes()?;
    let mut components = Map::new();
    if !schemas.is_empty() {
        components.insert("schemas".into(), Value::Object(schemas));
    }
    if !security_schemes.is_empty() {
        components.insert("securitySchemes".into(), Value::Object(security_schemes));
    }
    if !components.is_empty() {
        output.insert("components".into(), Value::Object(components));
    }

    let metadata = json!({
        "ir_version": document.ir_version,
        "ext": document.ext,
        "auth_schemes": document.auth_schemes,
        "diagnostics": document.diagnostics,
    });
    output.insert("x-pons".into(), metadata);
    let mut content = serde_json::to_string_pretty(&Value::Object(output))
        .map_err(|error| EmitError(format!("cannot serialize OpenAPI document: {error}")))?;
    content.push('\n');
    let mut coverage =
        coverage_for_document(&document, mode == EmitMode::Lossy).map_err(EmitError)?;
    for operation in &document.operations {
        if operation.stream.is_some() || operation.provides.is_some() {
            let subject = format!("operations.{}", operation_subject(operation));
            merge_coverage(&mut coverage, subject, CoverageSeverity::Opaque);
        }
    }
    let warnings = coverage_warnings(mode, &coverage)?;
    Ok(EmitOutput {
        content,
        warnings,
        coverage,
    })
}

fn coverage_warnings(mode: EmitMode, coverage: &Coverage) -> Result<Vec<String>, EmitError> {
    if mode == EmitMode::Strict {
        if let Some(violations) = strict_violation_message(coverage) {
            return Err(EmitError(format!("OpenAPI emitter reported {violations}")));
        }
        return Ok(Vec::new());
    }
    Ok([
        ("opaque", &coverage.opaque),
        ("lossy", &coverage.lossy),
        ("unsupported", &coverage.unsupported),
    ]
    .into_iter()
    .filter(|(_, subjects)| !subjects.is_empty())
    .map(|(level, subjects)| {
        format!(
            "OpenAPI emits {level} coverage at {}",
            subjects.iter().cloned().collect::<Vec<_>>().join(", ")
        )
    })
    .collect())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum CoverageSeverity {
    Exact,
    Opaque,
    Lossy,
    Unsupported,
}

fn merge_coverage(coverage: &mut Coverage, subject: String, required: CoverageSeverity) {
    coverage.exact.remove(&subject);
    let current = if coverage.unsupported.remove(&subject) {
        CoverageSeverity::Unsupported
    } else if coverage.lossy.remove(&subject) {
        CoverageSeverity::Lossy
    } else if coverage.opaque.remove(&subject) {
        CoverageSeverity::Opaque
    } else {
        CoverageSeverity::Exact
    };
    match current.max(required) {
        CoverageSeverity::Exact => {
            coverage.exact.insert(subject);
        }
        CoverageSeverity::Opaque => {
            coverage.opaque.insert(subject);
        }
        CoverageSeverity::Lossy => {
            coverage.lossy.insert(subject);
        }
        CoverageSeverity::Unsupported => {
            coverage.unsupported.insert(subject);
        }
    }
}

fn pair_name(document: &Document, definition: &TypeDef) -> String {
    let own = readable_name(&definition.name);
    if !is_split_half(definition) {
        return own;
    }
    let base = definition
        .origin
        .as_deref()
        .and_then(|origin| document.types.get(origin))
        .filter(|partner| partner.direction == Direction::Output)
        .map_or(own, |partner| readable_name(&partner.name));
    format!("{base}Input")
}

fn is_split_half(definition: &TypeDef) -> bool {
    definition.origin.is_some() && definition.direction == Direction::Input
}

fn unique_names(mut items: Vec<(String, bool, String)>) -> BTreeMap<String, String> {
    items.sort();
    let reserved = items
        .iter()
        .map(|(base, _, _)| base.clone())
        .collect::<BTreeSet<_>>();
    let mut assigned = BTreeSet::new();
    let mut names = BTreeMap::new();
    for (base, _, id) in items {
        let mut suffix = 1;
        let candidate = loop {
            let candidate = if suffix == 1 {
                base.clone()
            } else {
                format!("{base}{suffix}")
            };
            if (suffix == 1 || !reserved.contains(&candidate)) && assigned.insert(candidate.clone())
            {
                break candidate;
            }
            suffix += 1;
        };
        names.insert(id, candidate);
    }
    names
}

fn capitalize(word: &str) -> String {
    let mut chars = word.chars();
    chars
        .next()
        .map(|first| first.to_uppercase().chain(chars).collect::<String>())
        .unwrap_or_default()
}

fn readable_name(words: &[String]) -> String {
    let value = words
        .iter()
        .map(|word| capitalize(word))
        .collect::<String>();
    let mut name = value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '_' | '.' | '-') {
                character
            } else {
                '_'
            }
        })
        .collect::<String>();
    if name.is_empty() || !name.chars().next().unwrap().is_ascii_alphanumeric() {
        name.insert(0, '_');
    }
    name
}

#[cfg(test)]
mod tests;
