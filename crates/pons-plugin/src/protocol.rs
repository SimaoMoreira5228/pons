use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::{Component, Path};

use pons_ir::{AuthScheme, ContentKind, Document, EnumRepr, ObjectOpenness, StreamKind, TypeKind};
use serde::de::{self, Deserializer};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

pub const PROTOCOL_VERSION: u32 = 2;

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
    pub custom_auth: bool,
    #[serde(default)]
    pub flattened: bool,
    #[serde(default)]
    pub wide_integers: bool,
    #[serde(default)]
    pub opaque: bool,
    #[serde(default)]
    pub object_openness: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EmitMode {
    Strict,
    Lossy,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Coverage {
    pub exact: BTreeSet<String>,
    pub opaque: BTreeSet<String>,
    pub lossy: BTreeSet<String>,
    pub unsupported: BTreeSet<String>,
}

impl<'de> Deserialize<'de> for Coverage {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct Raw {
            #[serde(default)]
            exact: Vec<String>,
            #[serde(default)]
            opaque: Vec<String>,
            #[serde(default)]
            lossy: Vec<String>,
            #[serde(default)]
            unsupported: Vec<String>,
        }
        let raw = Raw::deserialize(deserializer)?;
        Ok(Coverage {
            exact: coverage_set("exact", raw.exact)?,
            opaque: coverage_set("opaque", raw.opaque)?,
            lossy: coverage_set("lossy", raw.lossy)?,
            unsupported: coverage_set("unsupported", raw.unsupported)?,
        })
    }
}

fn coverage_set<E: de::Error>(level: &str, values: Vec<String>) -> Result<BTreeSet<String>, E> {
    let mut set = BTreeSet::new();
    for value in values {
        if !set.insert(value.clone()) {
            return Err(E::custom(format!(
                "emitter coverage lists {value} more than once in {level} coverage"
            )));
        }
    }
    Ok(set)
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct EmitRequest<'a> {
    pub protocol: u32,
    pub mode: EmitMode,
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
    pub coverage: Option<Coverage>,
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

fn stream_kinds(operation: &pons_ir::Operation) -> Vec<&'static str> {
    if operation
        .body
        .as_ref()
        .is_some_and(|body| body.content == ContentKind::Stream)
    {
        return vec!["bytes"];
    }
    let Some(stream) = &operation.stream else {
        return if operation
            .responses
            .iter()
            .any(|response| matches!(response.content, ContentKind::Stream | ContentKind::Bytes))
        {
            vec!["bytes"]
        } else {
            Vec::new()
        };
    };
    vec![match stream.kind {
        StreamKind::Sse { .. } => "sse",
        StreamKind::Ndjson { .. } => "ndjson",
        StreamKind::Bytes { .. } => "bytes",
        StreamKind::Websocket { .. } => "websocket",
    }]
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
) -> Result<(), String> {
    validate_response(response)?;
    let Some(coverage) = &response.coverage else {
        return Err("emitter response is missing coverage".into());
    };
    let expected = coverage_subjects(document);
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
    if mode == EmitMode::Strict && !coverage.unsupported.is_empty() {
        return Err("emitter reported unsupported coverage in strict mode".into());
    }
    if mode == EmitMode::Strict && !coverage.lossy.is_empty() {
        return Err("emitter reported lossy coverage in strict mode".into());
    }
    if mode == EmitMode::Strict && !opaque_supported && !coverage.opaque.is_empty() {
        return Err("emitter reported opaque coverage without declaring opaque support".into());
    }
    Ok(())
}

fn coverage_subjects(document: &Document) -> BTreeSet<String> {
    document
        .types
        .keys()
        .map(|id| format!("types.{id}"))
        .chain(
            document
                .operations
                .iter()
                .map(|operation| format!("operations.{}", operation_subject(operation))),
        )
        .collect()
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord)]
enum CoverageLevel {
    #[default]
    Exact,
    Opaque,
    Lossy,
}

#[derive(Default)]
struct CoverageNode {
    level: CoverageLevel,
    dependents: Vec<usize>,
}

struct CoverageGraph<'a> {
    types: &'a Map<String, Value>,
    lossy_wide_integers: bool,
    nodes: Vec<CoverageNode>,
    indices: BTreeMap<usize, usize>,
    pending: Vec<(usize, &'a Value)>,
}

impl<'a> CoverageGraph<'a> {
    fn index(&mut self, value: &'a Value) -> usize {
        let key = value as *const Value as usize;
        if let Some(index) = self.indices.get(&key) {
            return *index;
        }
        let index = self.nodes.len();
        self.indices.insert(key, index);
        self.nodes.push(CoverageNode::default());
        self.pending.push((index, value));
        index
    }

    fn add_edge(&mut self, source: usize, target: &'a Value) {
        let target = self.index(target);
        self.nodes[target].dependents.push(source);
    }

    fn build(&mut self) {
        while let Some((index, value)) = self.pending.pop() {
            match value {
                Value::Object(fields) => {
                    let mut level = CoverageLevel::Exact;
                    if fields.get("kind").and_then(Value::as_str) == Some("opaque") {
                        level = CoverageLevel::Opaque;
                    }
                    if self.lossy_wide_integers
                        && fields.get("kind").and_then(Value::as_str) == Some("int")
                        && fields.get("wire").and_then(Value::as_str) == Some("number")
                        && fields
                            .get("bits")
                            .and_then(Value::as_u64)
                            .is_some_and(|bits| bits > 53)
                    {
                        level = CoverageLevel::Lossy;
                    }
                    if let Some(referenced) = referenced_value(fields, self.types) {
                        self.add_edge(index, referenced);
                    }
                    for (key, child) in fields {
                        match key.as_str() {
                            "ext" | "default" => continue,
                            "pagination" if !child.is_null() => {
                                level = level.max(CoverageLevel::Opaque);
                            }
                            "pagination" => {}
                            _ => self.add_edge(index, child),
                        }
                    }
                    self.nodes[index].level = level;
                }
                Value::Array(values) => {
                    for child in values {
                        self.add_edge(index, child);
                    }
                }
                _ => {}
            }
        }
    }

    fn propagate(&mut self) {
        let mut pending = self
            .nodes
            .iter()
            .enumerate()
            .filter_map(|(index, node)| (node.level != CoverageLevel::Exact).then_some(index))
            .collect::<VecDeque<_>>();
        while let Some(source) = pending.pop_front() {
            let level = self.nodes[source].level;
            for edge in 0..self.nodes[source].dependents.len() {
                let target = self.nodes[source].dependents[edge];
                if self.nodes[target].level < level {
                    self.nodes[target].level = level;
                    pending.push_back(target);
                }
            }
        }
    }

    fn level(&self, value: &Value) -> CoverageLevel {
        self.nodes[self.indices[&(value as *const Value as usize)]].level
    }
}

pub fn coverage_for_document(
    document: &Document,
    lossy_wide_integers: bool,
) -> Result<Coverage, String> {
    let value = serde_json::to_value(document)
        .map_err(|error| format!("cannot analyze emitter coverage: {error}"))?;
    let types = value
        .get("types")
        .and_then(Value::as_object)
        .ok_or_else(|| "cannot analyze emitter coverage: document types are missing".to_owned())?;
    let operations = value
        .get("operations")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            "cannot analyze emitter coverage: document operations are missing".to_owned()
        })?;
    let mut graph = CoverageGraph {
        types,
        lossy_wide_integers,
        nodes: Vec::new(),
        indices: BTreeMap::new(),
        pending: Vec::new(),
    };
    for definition in types.values() {
        graph.index(definition);
    }
    for operation in operations {
        graph.index(operation);
    }
    if document
        .operations
        .iter()
        .any(|operation| operation.errors_inherit)
        && let Some(common_errors) = value.get("common_errors")
    {
        graph.index(common_errors);
    }
    graph.build();
    graph.propagate();

    let mut coverage = Coverage::default();
    for (id, definition) in types {
        insert_coverage(
            &mut coverage,
            format!("types.{id}"),
            graph.level(definition),
        );
    }
    for (index, operation) in operations.iter().enumerate() {
        let Some(document_operation) = document.operations.get(index) else {
            return Err("cannot analyze emitter coverage: operation count mismatch".into());
        };
        let mut level = graph.level(operation);
        if document_operation.errors_inherit {
            level = level.max(
                value
                    .get("common_errors")
                    .map_or(CoverageLevel::Exact, |errors| graph.level(errors)),
            );
        }
        insert_coverage(
            &mut coverage,
            format!("operations.{}", operation_subject(document_operation)),
            level,
        );
    }
    Ok(coverage)
}

fn insert_coverage(coverage: &mut Coverage, subject: String, level: CoverageLevel) {
    match level {
        CoverageLevel::Exact => {
            coverage.exact.insert(subject);
        }
        CoverageLevel::Opaque => {
            coverage.opaque.insert(subject);
        }
        CoverageLevel::Lossy => {
            coverage.lossy.insert(subject);
        }
    }
}

fn referenced_value<'a>(
    fields: &Map<String, Value>,
    types: &'a Map<String, Value>,
) -> Option<&'a Value> {
    let reference = fields.get("ref").and_then(Value::as_str)?;
    if !fields
        .keys()
        .all(|key| matches!(key.as_str(), "ref" | "variant"))
    {
        return None;
    }
    let definition = types.get(reference)?;
    match fields.get("variant").and_then(Value::as_str) {
        Some(variant) => definition
            .get("variants")
            .and_then(Value::as_array)
            .and_then(|variants| {
                variants
                    .iter()
                    .find(|entry| entry.get("wire").and_then(Value::as_str) == Some(variant))
            })
            .and_then(|entry| entry.get("payload")),
        None => Some(definition),
    }
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

fn object_openness_name(openness: &ObjectOpenness) -> &'static str {
    match openness {
        ObjectOpenness::Reject => "reject",
        ObjectOpenness::PreserveTyped { .. } => "preserve_typed",
        ObjectOpenness::PreserveOpaque => "preserve_opaque",
    }
}

pub fn operation_subject(operation: &pons_ir::Operation) -> String {
    pons_ir::operation_path(&operation.id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pons_ir::{InlineType, TypeRef};

    fn manifest() -> EmitterManifest {
        EmitterManifest {
            name: "fixture".into(),
            version: "1.0.0".into(),
            protocol: VersionRange {
                min: "1.0".into(),
                max: "1.0".into(),
            },
            ir_version: VersionRange {
                min: "0.2".into(),
                max: "0.2".into(),
            },
            capabilities: EmitterCapabilities::default(),
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
        streaming_body.validate().unwrap();
        assert_eq!(
            stream_warnings(&sse, &streaming_body),
            vec!["fixture does not declare support for bytes streams in auth.login"]
        );

        let mut unframed: serde_json::Value =
            serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
        unframed["operations"][1]
            .as_object_mut()
            .unwrap()
            .remove("stream");
        unframed["operations"][1]["responses"][0]["content"] = serde_json::json!("stream");
        let unframed: Document = serde_json::from_value(unframed).unwrap();
        assert_eq!(
            stream_warnings(&sse, &unframed),
            vec!["fixture does not declare support for bytes streams in events.watch"]
        );

        let mut framed = sse.clone();
        framed.capabilities.streaming = vec!["sse".into(), "bytes".into()];
        assert!(stream_warnings(&framed, &streaming_body).is_empty());
        assert!(stream_warnings(&framed, &unframed).is_empty());
        assert!(stream_warnings(&framed, &document).is_empty());
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
        paginated["operations"][1]["semantics"]["pagination"] =
            serde_json::json!({ "kind": "opaque" });
        let paginated: Document = serde_json::from_value(paginated).unwrap();
        assert!(!paginated.has_opaque());
        assert!(
            manifest
                .unsupported_features(&paginated)
                .iter()
                .any(|warning| warning.0.contains("opaque value support"))
        );
        let mut supported = manifest;
        supported.capabilities.opaque = true;
        assert!(
            !supported
                .unsupported_features(&document)
                .iter()
                .any(|warning| warning.0.contains("opaque value support"))
        );
        assert!(
            !supported
                .unsupported_features(&paginated)
                .iter()
                .any(|warning| warning.0.contains("opaque value support"))
        );
    }

    #[test]
    fn emit_requests_carry_the_selected_mode() {
        let document: Document =
            serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
        let request = EmitRequest {
            protocol: PROTOCOL_VERSION,
            mode: EmitMode::Lossy,
            ir: &document,
            options: BTreeMap::new(),
            target: "fixture",
        };
        let value = serde_json::to_value(request).unwrap();
        assert_eq!(value["mode"], "lossy");
    }

    #[test]
    fn protocol_v2_rejects_v1_manifests_and_responses() {
        let manifest = manifest();
        assert!(manifest.validate("0.2").is_err());

        let response = EmitResponse {
            protocol: 1,
            files: Vec::new(),
            diagnostics: Vec::new(),
            coverage: None,
            error: None,
        };
        assert!(validate_response(&response).is_err());
    }

    #[test]
    fn coverage_must_cover_every_subject_once_and_respect_mode() {
        let document: Document =
            serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
        let exact = coverage_subjects(&document);
        let response = EmitResponse {
            protocol: PROTOCOL_VERSION,
            files: Vec::new(),
            diagnostics: Vec::new(),
            coverage: Some(Coverage {
                exact: exact.clone(),
                ..Coverage::default()
            }),
            error: None,
        };
        assert!(
            validate_response_for_document(&response, &document, EmitMode::Strict, true).is_ok()
        );

        let mut missing = response.clone();
        missing.coverage = None;
        assert!(
            validate_response_for_document(&missing, &document, EmitMode::Strict, true)
                .unwrap_err()
                .contains("missing coverage")
        );

        let mut duplicate = response.clone();
        duplicate
            .coverage
            .as_mut()
            .unwrap()
            .opaque
            .insert("types.User".into());
        assert!(
            validate_response_for_document(&duplicate, &document, EmitMode::Strict, true)
                .unwrap_err()
                .contains("more than once")
        );

        let mut opaque = response.clone();
        let coverage = opaque.coverage.as_mut().unwrap();
        coverage.exact.remove("types.User");
        coverage.opaque.insert("types.User".into());
        assert!(
            validate_response_for_document(&opaque, &document, EmitMode::Strict, false)
                .unwrap_err()
                .contains("opaque coverage without declaring opaque support")
        );
        assert!(validate_response_for_document(&opaque, &document, EmitMode::Strict, true).is_ok());
        assert!(validate_response_for_document(&opaque, &document, EmitMode::Lossy, false).is_ok());

        let mut unsupported = response.clone();
        let coverage = unsupported.coverage.as_mut().unwrap();
        coverage.exact.remove("types.User");
        coverage.unsupported.insert("types.User".into());
        assert!(
            validate_response_for_document(&unsupported, &document, EmitMode::Strict, true)
                .unwrap_err()
                .contains("unsupported coverage")
        );
        assert!(
            validate_response_for_document(&unsupported, &document, EmitMode::Lossy, true).is_ok()
        );

        let mut lossy = response;
        let coverage = lossy.coverage.as_mut().unwrap();
        coverage.exact.remove("types.User");
        coverage.lossy.insert("types.User".into());
        assert!(
            validate_response_for_document(&lossy, &document, EmitMode::Strict, true)
                .unwrap_err()
                .contains("lossy coverage")
        );
        assert!(validate_response_for_document(&lossy, &document, EmitMode::Lossy, true).is_ok());
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

    #[test]
    fn coverage_ignores_extensions_and_unselected_enum_variants() {
        let mut raw: serde_json::Value =
            serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
        raw["types"]["User"]["fields"][1]["default"] = serde_json::json!({ "kind": "opaque" });
        let defaults: Document = serde_json::from_value(raw).unwrap();
        let coverage = coverage_for_document(&defaults, false).unwrap();
        assert!(coverage.exact.contains("types.User"));
        assert!(!coverage.opaque.contains("types.User"));

        let mut paginated: serde_json::Value =
            serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
        paginated["operations"][1]["semantics"]["pagination"] =
            serde_json::json!({ "kind": "opaque" });
        let paginated: Document = serde_json::from_value(paginated).unwrap();
        let coverage = coverage_for_document(&paginated, false).unwrap();
        assert!(coverage.opaque.contains("operations.events.watch"));
        assert!(!coverage.exact.contains("operations.events.watch"));

        let mut document: Document =
            serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
        document
            .types
            .get_mut("User")
            .unwrap()
            .ext
            .insert("marker".into(), serde_json::json!({ "kind": "opaque" }));
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
        let subjects: Vec<String> = coverage_subjects(&document).into_iter().collect();
        let response: EmitResponse = serde_json::from_value(serde_json::json!({
            "protocol": PROTOCOL_VERSION,
            "coverage": { "exact": subjects }
        }))
        .unwrap();
        assert!(
            validate_response_for_document(&response, &document, EmitMode::Strict, true).is_ok()
        );

        let mut repeated = subjects;
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
        ] {
            let response = EmitResponse {
                protocol: PROTOCOL_VERSION,
                files: vec![file(path)],
                diagnostics: Vec::new(),
                coverage: None,
                error: None,
            };
            assert!(validate_response(&response).is_err(), "accepted {path:?}");
        }

        let response = EmitResponse {
            protocol: PROTOCOL_VERSION,
            files: vec![file("client.ts"), file("client.ts")],
            diagnostics: Vec::new(),
            coverage: None,
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
