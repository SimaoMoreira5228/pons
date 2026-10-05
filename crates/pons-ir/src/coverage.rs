use std::collections::{BTreeMap, BTreeSet, VecDeque};

use serde_json::{Map, Value};

use crate::Document;
use crate::reported_coverage::{operation_subject, type_subject};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum CoverageLevel {
    #[default]
    Exact,
    Opaque,
    Lossy,
    Unsupported,
}

impl CoverageLevel {
    pub fn as_str(self) -> &'static str {
        match self {
            CoverageLevel::Exact => "exact",
            CoverageLevel::Opaque => "opaque",
            CoverageLevel::Lossy => "lossy",
            CoverageLevel::Unsupported => "unsupported",
        }
    }
}

pub fn coverage_subjects(document: &Document) -> BTreeSet<String> {
    document
        .types
        .keys()
        .map(|id| type_subject(id))
        .chain(
            document
                .operations
                .iter()
                .map(|operation| format!("operations.{}", operation_subject(operation))),
        )
        .collect()
}

pub fn canonical_coverage(
    document: &Document,
    lossy_wide_integers: bool,
) -> Result<BTreeMap<String, CoverageLevel>, String> {
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

    let mut coverage = BTreeMap::new();
    for (id, definition) in types {
        coverage.insert(type_subject(id), graph.level(definition));
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
        coverage.insert(
            format!("operations.{}", operation_subject(document_operation)),
            level,
        );
    }
    Ok(coverage)
}

pub fn strict_violations(
    levels: impl IntoIterator<Item = (String, CoverageLevel)>,
    carries_opaque: bool,
) -> Vec<(&'static str, Vec<String>)> {
    let mut counts: BTreeMap<CoverageLevel, Vec<String>> = BTreeMap::new();
    for (subject, level) in levels {
        if level == CoverageLevel::Opaque && carries_opaque {
            continue;
        }
        counts.entry(level).or_default().push(subject);
    }
    [
        CoverageLevel::Unsupported,
        CoverageLevel::Lossy,
        CoverageLevel::Opaque,
    ]
    .into_iter()
    .filter_map(|level| {
        let subjects = counts.remove(&level)?;
        (!subjects.is_empty()).then(|| (level.as_str(), subjects))
    })
    .collect()
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
                    let level = self.subject_level(fields);
                    if let Some(referenced) = referenced_value(fields, self.types) {
                        self.add_edge(index, referenced);
                    }
                    for (key, child) in fields {
                        if matches!(key.as_str(), "ext" | "default") {
                            continue;
                        }
                        self.add_edge(index, child);
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

    fn subject_level(&self, fields: &Map<String, Value>) -> CoverageLevel {
        let kind = fields.get("kind").and_then(Value::as_str);
        if kind == Some("opaque") || preserves_openness(fields.get("openness")) {
            return CoverageLevel::Opaque;
        }
        if self.lossy_wide_integers
            && kind == Some("int")
            && fields.get("wire").and_then(Value::as_str) == Some("number")
            && fields
                .get("bits")
                .and_then(Value::as_u64)
                .is_some_and(|bits| bits > 53)
        {
            return CoverageLevel::Lossy;
        }
        CoverageLevel::Exact
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

fn preserves_openness(openness: Option<&Value>) -> bool {
    match openness {
        Some(Value::String(name)) => name.starts_with("preserve"),
        Some(Value::Object(_)) => true,
        _ => false,
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn fixture() -> Document {
        serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap()
    }

    fn levels(document: &Document, lossy_wide_integers: bool) -> BTreeMap<String, CoverageLevel> {
        canonical_coverage(document, lossy_wide_integers).unwrap()
    }

    fn subjects_at(coverage: &BTreeMap<String, CoverageLevel>, level: CoverageLevel) -> Vec<&str> {
        coverage
            .iter()
            .filter(|(_, actual)| **actual == level)
            .map(|(subject, _)| subject.as_str())
            .collect()
    }

    #[test]
    fn canonical_coverage_pins_the_repository_fixture() {
        let document = fixture();
        let coverage = levels(&document, false);
        assert_eq!(
            coverage.keys().collect::<Vec<_>>(),
            vec![
                "operations.auth.login",
                "operations.events.watch",
                "types.EventQuery",
                "types.LoginError",
                "types.LoginRequest",
                "types.User",
            ]
        );
        assert_eq!(
            subjects_at(&coverage, CoverageLevel::Opaque),
            vec![
                "operations.auth.login",
                "operations.events.watch",
                "types.LoginRequest",
                "types.User",
            ]
        );
        assert_eq!(
            subjects_at(&coverage, CoverageLevel::Exact),
            vec!["types.EventQuery", "types.LoginError",]
        );
        assert!(subjects_at(&coverage, CoverageLevel::Lossy).is_empty());
        assert!(subjects_at(&coverage, CoverageLevel::Unsupported).is_empty());
        assert!(strict_violations(coverage.clone(), true).is_empty());
        assert_eq!(
            strict_violations(coverage, false),
            vec![(
                "opaque",
                vec![
                    "operations.auth.login".to_owned(),
                    "operations.events.watch".to_owned(),
                    "types.LoginRequest".to_owned(),
                    "types.User".to_owned(),
                ]
            )]
        );
    }

    #[test]
    fn canonical_coverage_reaches_types_through_inherited_errors() {
        let mut document = fixture();
        document.types.insert(
            "Orphan".into(),
            named_struct("orphan", serde_json::json!({"ref": "User"})),
        );
        document.common_errors = vec![
            serde_json::from_value(serde_json::json!({
                "status": "5xx",
                "name": ["common", "error"],
                "content": "json",
                "body": {"ref": "Orphan"}
            }))
            .unwrap(),
        ];
        document
            .operations
            .iter_mut()
            .for_each(|operation| operation.errors_inherit = false);
        document.validate().unwrap();

        let coverage = levels(&document, false);
        assert_eq!(coverage["types.Orphan"], CoverageLevel::Opaque);
        assert_eq!(coverage["operations.auth.login"], CoverageLevel::Opaque);

        document.operations[0].errors_inherit = true;
        let inherited = levels(&document, false);
        assert_eq!(inherited["types.Orphan"], CoverageLevel::Opaque);
        assert_eq!(inherited["operations.auth.login"], CoverageLevel::Opaque);
    }

    #[test]
    fn canonical_coverage_tracks_target_fidelity() {
        let mut raw: Value =
            serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
        raw["types"]["EventQuery"]["fields"][0]["ty"]["wire"] = json!("number");
        raw["types"]["EventQuery"]["fields"][0]["ty"]["bits"] = json!(64);
        let document: Document = serde_json::from_value(raw).unwrap();
        document.validate().unwrap();

        let preserving = levels(&document, false);
        assert_eq!(preserving["types.EventQuery"], CoverageLevel::Exact);
        assert_eq!(preserving["operations.events.watch"], CoverageLevel::Opaque);

        let narrowing = levels(&document, true);
        assert_eq!(narrowing["types.EventQuery"], CoverageLevel::Lossy);
        assert_eq!(narrowing["operations.events.watch"], CoverageLevel::Lossy);
        assert_eq!(
            strict_violations(narrowing, true),
            vec![(
                "lossy",
                vec![
                    "operations.events.watch".to_owned(),
                    "types.EventQuery".to_owned()
                ]
            )]
        );
    }

    #[test]
    fn strict_violations_report_the_worst_class_first() {
        assert!(strict_violations(BTreeMap::new(), false).is_empty());
        let coverage = BTreeMap::from([
            ("types.A".to_owned(), CoverageLevel::Lossy),
            ("types.B".to_owned(), CoverageLevel::Opaque),
            ("types.C".to_owned(), CoverageLevel::Unsupported),
            ("types.D".to_owned(), CoverageLevel::Exact),
        ]);
        assert_eq!(
            strict_violations(coverage.clone(), true),
            vec![
                ("unsupported", vec!["types.C".to_owned()]),
                ("lossy", vec!["types.A".to_owned()]),
            ]
        );
        assert_eq!(
            strict_violations(coverage, false),
            vec![
                ("unsupported", vec!["types.C".to_owned()]),
                ("lossy", vec!["types.A".to_owned()]),
                ("opaque", vec!["types.B".to_owned()]),
            ]
        );
    }

    fn named_struct(name: &str, field: Value) -> crate::TypeDef {
        serde_json::from_value(serde_json::json!({
            "kind": "struct",
            "openness": "reject",
            "name": [name],
            "direction": "both",
            "fields": [{
                "wire": "user",
                "ident": ["user"],
                "ty": field,
                "presence": "required"
            }]
        }))
        .unwrap()
    }
}
