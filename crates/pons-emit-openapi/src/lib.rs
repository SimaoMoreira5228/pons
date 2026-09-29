mod operations;
mod schemas;
mod security;

use std::collections::{BTreeMap, BTreeSet};

use pons_ir::{Document, ObjectOpenness, TypeKind};
use pons_plugin::{Coverage, operation_subject};
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
                .map(|(id, ty)| (id.clone(), readable_name(&ty.name))),
        ),
        security_names: unique_names(
            document
                .auth_schemes
                .keys()
                .map(|id| (id.clone(), readable_name(std::slice::from_ref(id)))),
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
    let mut coverage = pons_plugin::coverage_for_document(&document, mode == EmitMode::Lossy)
        .map_err(EmitError)?;
    for operation in &document.operations {
        if operation.stream.is_some() || operation.provides.is_some() {
            let subject = format!("operations.{}", operation_subject(operation));
            merge_coverage(&mut coverage, subject, CoverageSeverity::Opaque);
        }
    }
    mark_preservation_coverage(&document, &mut coverage);
    let warnings = coverage_warnings(mode, &coverage)?;
    Ok(EmitOutput {
        content,
        warnings,
        coverage,
    })
}

fn coverage_warnings(mode: EmitMode, coverage: &Coverage) -> Result<Vec<String>, EmitError> {
    if mode == EmitMode::Strict {
        for (level, subjects) in [
            ("unsupported", &coverage.unsupported),
            ("lossy", &coverage.lossy),
        ] {
            if !subjects.is_empty() {
                return Err(EmitError(format!(
                    "OpenAPI emitter reported {level} coverage in strict mode: {}",
                    subjects.iter().cloned().collect::<Vec<_>>().join(", ")
                )));
            }
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

fn mark_preservation_coverage(document: &Document, coverage: &mut Coverage) {
    let serialized_types = document
        .types
        .iter()
        .filter_map(|(id, definition)| {
            serde_json::to_value(definition)
                .ok()
                .map(|value| (id.clone(), value))
        })
        .collect::<BTreeMap<_, _>>();
    let mut preserved = document
        .types
        .iter()
        .filter_map(|(id, definition)| {
            let TypeKind::Struct { openness, .. } = &definition.definition else {
                return None;
            };
            matches!(
                openness,
                ObjectOpenness::PreserveTyped { .. } | ObjectOpenness::PreserveOpaque
            )
            .then_some(id.clone())
        })
        .collect::<BTreeSet<_>>();
    let mut dependents = BTreeMap::<String, Vec<String>>::new();
    for (id, value) in &serialized_types {
        let mut references = BTreeSet::new();
        collect_type_references(value, &serialized_types, &mut references);
        for reference in references {
            dependents.entry(reference).or_default().push(id.clone());
        }
    }
    let mut pending = preserved.iter().cloned().collect::<Vec<_>>();
    while let Some(reference) = pending.pop() {
        let Some(dependents) = dependents.get(&reference) else {
            continue;
        };
        for dependent in dependents {
            if preserved.insert(dependent.clone()) {
                pending.push(dependent.clone());
            }
        }
    }
    if preserved.is_empty() {
        return;
    }
    for id in &preserved {
        merge_coverage(coverage, format!("types.{id}"), CoverageSeverity::Opaque);
    }
    let Ok(operations) = serde_json::to_value(&document.operations) else {
        return;
    };
    let Some(operations) = operations.as_array() else {
        return;
    };
    let common_errors = serde_json::to_value(&document.common_errors).ok();
    for (operation, value) in document.operations.iter().zip(operations) {
        let inherited = operation.errors_inherit
            && common_errors.as_ref().is_some_and(|errors| {
                references_preserved_type(errors, &serialized_types, &preserved)
            });
        if inherited || references_preserved_type(value, &serialized_types, &preserved) {
            let subject = format!("operations.{}", operation_subject(operation));
            merge_coverage(coverage, subject, CoverageSeverity::Opaque);
        }
    }
}

fn collect_type_references(
    value: &Value,
    types: &BTreeMap<String, Value>,
    references: &mut BTreeSet<String>,
) {
    collect_type_references_inner(value, types, references, &mut BTreeSet::new());
}

fn collect_type_references_inner(
    value: &Value,
    types: &BTreeMap<String, Value>,
    references: &mut BTreeSet<String>,
    active_variants: &mut BTreeSet<(String, String)>,
) {
    match value {
        Value::Object(fields) => {
            if let Some(reference) = fields.get("ref").and_then(Value::as_str)
                && fields
                    .keys()
                    .all(|key| matches!(key.as_str(), "ref" | "variant"))
            {
                if let Some(variant) = fields.get("variant").and_then(Value::as_str)
                    && let Some(payload) = types
                        .get(reference)
                        .and_then(|definition| definition.get("variants"))
                        .and_then(Value::as_array)
                        .and_then(|variants| {
                            variants.iter().find(|entry| {
                                entry.get("wire").and_then(Value::as_str) == Some(variant)
                            })
                        })
                        .and_then(|entry| entry.get("payload"))
                {
                    let active = (reference.to_owned(), variant.to_owned());
                    if active_variants.insert(active.clone()) {
                        collect_type_references_inner(payload, types, references, active_variants);
                        active_variants.remove(&active);
                    }
                } else {
                    references.insert(reference.to_owned());
                }
                return;
            }
            for (key, value) in fields {
                if key == "ext" || key == "default" || key == "pagination" {
                    continue;
                }
                collect_type_references_inner(value, types, references, active_variants);
            }
        }
        Value::Array(values) => {
            for value in values {
                collect_type_references_inner(value, types, references, active_variants);
            }
        }
        _ => {}
    }
}

fn references_preserved_type(
    value: &Value,
    types: &BTreeMap<String, Value>,
    preserved: &BTreeSet<String>,
) -> bool {
    let mut references = BTreeSet::new();
    collect_type_references(value, types, &mut references);
    references
        .iter()
        .any(|reference| preserved.contains(reference))
}

fn unique_names(items: impl Iterator<Item = (String, String)>) -> BTreeMap<String, String> {
    let mut names = BTreeMap::new();
    let mut used = std::collections::BTreeSet::new();
    for (id, name) in items {
        let mut candidate = name.clone();
        let mut suffix = 2;
        while !used.insert(candidate.clone()) {
            candidate = format!("{name}{suffix}");
            suffix += 1;
        }
        names.insert(id, candidate);
    }
    names
}

fn readable_name(words: &[String]) -> String {
    let value = words
        .iter()
        .map(|word| {
            let mut chars = word.chars();
            chars
                .next()
                .map(|first| first.to_uppercase().chain(chars).collect::<String>())
                .unwrap_or_default()
        })
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
mod tests {
    use pons_ir::{
        AuthScheme, ByteEncoding, ContentKind, Direction, ErrorStatus, InlineType, NamedRef,
        OAuthFlow, OAuthFlowKind, ObjectOpenness, OpaqueWire, PathSegment, Presence,
        ResponseHeader, ResponseStatus, StreamKind, TypeDef, TypeKind, TypeRef,
    };
    use pons_plugin::Coverage;
    use serde_json::{Value, json};
    use std::collections::BTreeMap;

    fn fixture() -> pons_ir::Document {
        serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap()
    }

    fn string() -> TypeRef {
        TypeRef::Inline(InlineType::String {
            format: None,
            min_length: None,
            max_length: None,
            pattern: None,
        })
    }

    fn named_type(name: &str) -> TypeDef {
        TypeDef {
            name: vec![name.into()],
            direction: Direction::Both,
            definition: TypeKind::Struct {
                fields: Vec::new(),
                flattened: Vec::new(),
                openness: ObjectOpenness::Reject,
            },
            origin: None,
            doc: None,
            diagnostics: Vec::new(),
            ext: Default::default(),
        }
    }

    #[test]
    fn emits_openapi_31_paths_schemas_security_and_pons_metadata() {
        let document = fixture();
        let encoded = super::emit(&document).unwrap();
        assert!(encoded.ends_with('\n'));
        let output: Value = serde_json::from_str(&encoded).unwrap();

        assert_eq!(output["openapi"], "3.1.1");
        assert_eq!(output["info"]["title"], "Example");
        assert_eq!(output["servers"][0]["url"], "https://api.example.test");
        assert_eq!(
            output["paths"]["/auth/login"]["post"]["operationId"],
            "auth.login"
        );
        assert!(
            output["paths"]["/auth/login"]["post"]["security"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            output["paths"]["/auth/login"]["post"]["requestBody"]["content"]["application/json"]["schema"]
                ["$ref"],
            "#/components/schemas/LoginRequest"
        );
        assert_eq!(
            output["paths"]["/events/{cursor}"]["get"]["parameters"][0]["in"],
            "path"
        );
        assert_eq!(
            output["paths"]["/events/{cursor}"]["get"]["parameters"][1]["name"],
            "limit"
        );
        assert_eq!(
            output["paths"]["/events/{cursor}"]["get"]["responses"]["200"]["content"]["text/event-stream"]
                ["schema"]["x-pons-events"][0]["event"],
            "user"
        );
        assert_eq!(
            output["paths"]["/events/{cursor}"]["get"]["responses"]["200"]["headers"]["X-Trace-ID"]
                ["required"],
            true
        );
        assert_eq!(
            output["paths"]["/events/{cursor}"]["get"]["x-pons"]["stream"]["kind"],
            "sse"
        );
        assert!(
            output["paths"]["/events/{cursor}"]["get"]["security"][0]["Tenant"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            output["paths"]["/auth/login"]["post"]["responses"]["4XX"]["content"]["application/json"]
                ["schema"]["properties"]["code"]["const"],
            "invalid"
        );
        assert_eq!(
            output["components"]["securitySchemes"]["Tenant"]["name"],
            "X-Tenant"
        );
        assert_eq!(
            output["components"]["schemas"]["User"]["properties"]["id"]["type"],
            "string"
        );
        assert_eq!(
            output["components"]["schemas"]["User"]["properties"]["id"]["x-pons"]["field"]["ident"]
                [0],
            "id"
        );
        assert_eq!(
            output["components"]["schemas"]["User"]["x-pons"]["type"]["name"][0],
            "user"
        );
        assert_eq!(
            output["components"]["schemas"]["User"]["properties"]["nickname"]["anyOf"][1]["type"],
            "null"
        );
        let locked = output["components"]["schemas"]["LoginError"]["oneOf"]
            .as_array()
            .unwrap()
            .iter()
            .find(|schema| schema["properties"]["code"]["const"] == "locked")
            .unwrap();
        assert!(locked["properties"]["retry_after"]["format"].is_null());
        assert!(
            output["x-pons"]["ext"]["source.note"]["kept"]
                .as_bool()
                .unwrap()
        );
    }

    #[test]
    fn preserves_operation_id_punctuation() {
        let mut document = fixture();
        document.operations[0].id = vec![
            vec!["under_score".into()],
            vec!["dot.name".into()],
            vec!["percent%name".into()],
        ];
        let output: Value = serde_json::from_str(&super::emit(&document).unwrap()).unwrap();
        assert_eq!(
            output["paths"]["/auth/login"]["post"]["operationId"],
            "under_score.dot.name.percent%name"
        );
    }

    #[test]
    fn rejects_operation_id_collisions() {
        let mut document = fixture();
        document.operations[0].id = vec![vec!["same".into()], vec!["part".into()]];
        document.operations[1].id = vec![vec!["same.part".into()]];
        assert!(
            super::emit(&document)
                .unwrap_err()
                .0
                .contains("duplicate OpenAPI operationId same.part")
        );
    }

    #[test]
    fn emits_opaque_metadata_without_inventing_a_schema() {
        let mut document = fixture();
        let user = document.types.get_mut("User").unwrap();
        let TypeKind::Struct { fields, .. } = &mut user.definition else {
            unreachable!()
        };
        fields[0].ty = TypeRef::Inline(InlineType::Opaque {
            codec: "example.codec".into(),
            wire: OpaqueWire::Json,
        });
        let emitted = super::emit_with_mode(&document, super::EmitMode::Strict).unwrap();
        assert!(emitted.coverage.opaque.contains("types.User"));
        assert!(emitted.coverage.opaque.contains("operations.auth.login"));
        let output: Value = serde_json::from_str(&emitted.content).unwrap();
        let field = &output["components"]["schemas"]["User"]["properties"]["id"];
        assert_eq!(field["x-pons-opaque"]["codec"], "example.codec");
        assert_eq!(field["x-pons-opaque"]["wire"], "json");
        assert!(field.get("type").is_none());
    }

    #[test]
    fn lossy_mode_warns_for_non_exact_coverage() {
        let mut raw: Value = serde_json::to_value(fixture()).unwrap();
        raw["types"]["LoginError"]["variants"][1]["payload"]["fields"][0]["ty"]["wire"] =
            json!("number");
        raw["types"]["LoginError"]["variants"][1]["payload"]["fields"][0]["ty"]["bits"] = json!(64);
        let document: pons_ir::Document = serde_json::from_value(raw).unwrap();

        let strict = super::emit_with_mode(&document, super::EmitMode::Strict).unwrap();
        assert!(strict.warnings.is_empty());
        assert!(strict.coverage.exact.contains("types.LoginError"));

        let lossy = super::emit_with_mode(&document, super::EmitMode::Lossy).unwrap();
        assert!(lossy.coverage.lossy.contains("types.LoginError"));
        assert!(
            lossy
                .warnings
                .iter()
                .any(|warning| warning.contains("lossy coverage")
                    && warning.contains("types.LoginError"))
        );
    }

    #[test]
    fn opaque_feature_marks_preserve_lossy_classification() {
        let mut raw: Value = serde_json::to_value(fixture()).unwrap();
        raw["types"]["User"]["fields"][0]["ty"]["wire"] = json!("number");
        raw["types"]["User"]["fields"][0]["ty"]["bits"] = json!(64);
        let document: pons_ir::Document = serde_json::from_value(raw).unwrap();

        let emitted = super::emit_with_mode(&document, super::EmitMode::Lossy).unwrap();
        for subject in [
            "types.User",
            "operations.auth.login",
            "operations.events.watch",
        ] {
            assert!(emitted.coverage.lossy.contains(subject), "{subject}");
            assert!(!emitted.coverage.opaque.contains(subject), "{subject}");
        }
        assert!(emitted.warnings.iter().any(|warning| {
            warning.contains("lossy coverage")
                && warning.contains("types.User")
                && warning.contains("operations.auth.login")
                && warning.contains("operations.events.watch")
        }));
    }

    #[test]
    fn rejects_mismatched_byte_stream_media_type_override() {
        let mut document = fixture();
        {
            let operation = document
                .operations
                .iter_mut()
                .find(|operation| operation.stream.is_some())
                .unwrap();
            operation.stream.as_mut().unwrap().kind = StreamKind::Bytes {
                content_type: "application/pdf".into(),
            };
            operation.responses[0].media_type = Some("application/json".into());
        }
        assert!(
            super::emit(&document)
                .unwrap_err()
                .0
                .contains("does not match stream kind media type")
        );

        document
            .operations
            .iter_mut()
            .find(|operation| operation.stream.is_some())
            .unwrap()
            .responses[0]
            .media_type = Some("application/pdf".into());
        assert!(super::emit(&document).is_ok());

        {
            let operation = document
                .operations
                .iter_mut()
                .find(|operation| operation.stream.is_some())
                .unwrap();
            operation.stream.as_mut().unwrap().kind = StreamKind::Bytes {
                content_type: "Application/PDF ; Version=1".into(),
            };
            operation.responses[0].media_type = Some("application/pdf; version=1".into());
        }
        assert!(super::emit(&document).is_ok());
    }

    #[test]
    fn rejects_sse_and_ndjson_media_type_overrides() {
        let mut document = fixture();
        for (kind, expected) in [
            (StreamKind::Sse { events: vec![] }, "text/event-stream"),
            (
                StreamKind::Ndjson {
                    item: TypeRef::Inline(InlineType::Bool),
                },
                "application/x-ndjson",
            ),
        ] {
            for (media_type, accepted) in [
                (expected.to_owned(), true),
                (format!("{expected}; charset=utf-8"), true),
                (format!("{expected} ; CharSet=UTF-8"), true),
                ("application/json".into(), false),
                ("application/octet-stream".into(), false),
            ] {
                let operation = document
                    .operations
                    .iter_mut()
                    .find(|operation| operation.stream.is_some())
                    .unwrap();
                operation.stream.as_mut().unwrap().kind = kind.clone();
                operation.responses[0].media_type = Some(media_type.clone());
                assert_eq!(
                    super::emit(&document).is_ok(),
                    accepted,
                    "{media_type:?} on an {expected} stream"
                );
            }

            let operation = document
                .operations
                .iter_mut()
                .find(|operation| operation.stream.is_some())
                .unwrap();
            operation.stream.as_mut().unwrap().kind = kind;
            operation.responses[0].media_type = None;
            let emitted: Value = serde_json::from_str(&super::emit(&document).unwrap()).unwrap();
            assert!(emitted.to_string().contains(expected));
        }
    }

    #[test]
    fn lossy_mode_warns_for_opaque_coverage_while_strict_allows_it() {
        let mut document = fixture();
        let user = document.types.get_mut("User").unwrap();
        let TypeKind::Struct { fields, .. } = &mut user.definition else {
            unreachable!()
        };
        fields[0].ty = TypeRef::Inline(InlineType::Opaque {
            codec: "example.codec".into(),
            wire: OpaqueWire::Json,
        });

        let strict = super::emit_with_mode(&document, super::EmitMode::Strict).unwrap();
        assert!(strict.warnings.is_empty());
        assert!(strict.coverage.opaque.contains("types.User"));

        let lossy = super::emit_with_mode(&document, super::EmitMode::Lossy).unwrap();
        assert!(
            lossy.warnings.iter().any(
                |warning| warning.contains("opaque coverage") && warning.contains("types.User")
            )
        );
    }

    #[test]
    fn strict_mode_rejects_lossy_and_unsupported_coverage() {
        let lossy = Coverage {
            lossy: ["types.User".into()].into_iter().collect(),
            ..Coverage::default()
        };
        assert!(
            super::coverage_warnings(super::EmitMode::Strict, &lossy)
                .unwrap_err()
                .0
                .contains("lossy coverage")
        );
        let unsupported = Coverage {
            unsupported: ["types.User".into()].into_iter().collect(),
            ..Coverage::default()
        };
        assert!(
            super::coverage_warnings(super::EmitMode::Strict, &unsupported)
                .unwrap_err()
                .0
                .contains("unsupported coverage")
        );
        let opaque = Coverage {
            opaque: ["types.User".into()].into_iter().collect(),
            ..Coverage::default()
        };
        assert!(super::coverage_warnings(super::EmitMode::Strict, &opaque).is_ok());
    }

    #[test]
    fn marks_extension_only_operations_as_opaque() {
        let document = fixture();
        let emitted = super::emit_with_mode(&document, super::EmitMode::Strict).unwrap();
        assert!(emitted.coverage.opaque.contains("operations.auth.login"));
        assert!(emitted.coverage.opaque.contains("operations.events.watch"));
        assert!(emitted.coverage.opaque.contains("types.User"));
        assert!(emitted.coverage.opaque.contains("types.LoginRequest"));
        assert!(!emitted.coverage.exact.contains("operations.auth.login"));
        assert!(!emitted.coverage.exact.contains("operations.events.watch"));
        assert!(!emitted.coverage.lossy.contains("operations.auth.login"));
        assert!(!emitted.coverage.lossy.contains("operations.events.watch"));
    }

    #[test]
    fn marks_preservation_through_type_references() {
        let mut raw = serde_json::to_value(fixture()).unwrap();
        raw["types"]["Wrapper"] = json!({
            "kind": "struct",
            "name": ["wrapper"],
            "direction": "both",
            "openness": "reject",
            "fields": [{
                "wire": "user",
                "ident": ["user"],
                "ty": { "ref": "User" },
                "presence": "required"
            }]
        });
        raw["operations"][1]["stream"] = Value::Null;
        raw["operations"][1]["responses"] = json!([{
            "status": 200,
            "content": "json",
            "body": { "ref": "Wrapper" }
        }]);
        let document: pons_ir::Document = serde_json::from_value(raw).unwrap();
        let emitted = super::emit_with_mode(&document, super::EmitMode::Strict).unwrap();
        assert!(emitted.coverage.opaque.contains("types.Wrapper"));
        assert!(emitted.coverage.opaque.contains("operations.events.watch"));
    }

    #[test]
    fn emits_recursive_selected_enum_variants() {
        let mut raw: Value = serde_json::to_value(fixture()).unwrap();
        raw["types"]["Chain"] = json!({
            "kind": "enum",
            "name": ["chain"],
            "direction": "both",
            "repr": {"kind": "external"},
            "variants": [
                {
                    "wire": "next",
                    "ident": ["next"],
                    "payload": {"kind": "newtype", "ty": {"ref": "Chain", "variant": "next"}}
                },
                {"wire": "leaf", "ident": ["leaf"], "payload": {"kind": "unit"}}
            ]
        });
        raw["types"]["User"]["fields"]
            .as_array_mut()
            .unwrap()
            .push(json!({
                "wire": "chain",
                "ident": ["chain"],
                "ty": {"ref": "Chain", "variant": "next"},
                "presence": "required"
            }));
        let document: pons_ir::Document = serde_json::from_value(raw).unwrap();
        let output = super::emit(&document).unwrap();
        let value: Value = serde_json::from_str(&output).unwrap();
        assert_eq!(value["openapi"], "3.1.1");
        assert!(
            value["components"]["schemas"]["Chain"]["oneOf"]
                .as_array()
                .unwrap()
                .iter()
                .any(|schema| schema
                    .to_string()
                    .contains("#/components/schemas/Chain/oneOf/"))
        );
    }

    #[test]
    fn emits_declared_error_response_headers() {
        let mut document = fixture();
        document.operations[0].errors[0]
            .headers
            .push(ResponseHeader {
                wire: "Retry-After".into(),
                ty: string(),
                presence: Presence::Required,
                ext: Default::default(),
            });
        let output: Value = serde_json::from_str(&super::emit(&document).unwrap()).unwrap();
        let header =
            &output["paths"]["/auth/login"]["post"]["responses"]["4XX"]["headers"]["Retry-After"];
        assert_eq!(header["required"], true);
        assert_eq!(header["schema"]["type"], "string");
    }

    #[test]
    fn emits_auth_scopes_only_for_oauth2_schemes() {
        let mut document = fixture();
        document.auth_schemes.insert(
            "oauth".into(),
            AuthScheme::OAuth2 {
                flows: vec![OAuthFlow {
                    kind: OAuthFlowKind::AuthorizationCode,
                    authorization_url: Some("https://auth.test/authorize".into()),
                    token_url: Some("https://auth.test/token".into()),
                    scopes: BTreeMap::new(),
                    ext: Default::default(),
                }],
            },
        );
        let operation = document
            .operations
            .iter_mut()
            .find(|operation| operation.id == vec![vec!["events"], vec!["watch"]])
            .unwrap();
        operation.auth.alternatives = vec![vec!["oauth".into()], vec!["tenant".into()]];
        operation.auth.scopes = BTreeMap::from([
            ("oauth".into(), vec!["events:read".into()]),
            ("tenant".into(), vec!["ignored".into()]),
        ]);
        let output: Value = serde_json::from_str(&super::emit(&document).unwrap()).unwrap();
        let security = &output["paths"]["/events/{cursor}"]["get"]["security"];
        assert_eq!(security[0]["Oauth"][0], "events:read");
        assert_eq!(security[1]["Tenant"], json!([]));
    }

    #[test]
    fn emits_object_openness_policies() {
        let mut document = fixture();
        let user = document.types.get_mut("User").unwrap();
        let TypeKind::Struct {
            fields, flattened, ..
        } = &user.definition
        else {
            unreachable!()
        };
        user.definition = TypeKind::Struct {
            fields: fields.clone(),
            flattened: flattened.clone(),
            openness: ObjectOpenness::PreserveTyped { value: string() },
        };
        let login = document.types.get_mut("LoginRequest").unwrap();
        let TypeKind::Struct {
            fields, flattened, ..
        } = &login.definition
        else {
            unreachable!()
        };
        login.definition = TypeKind::Struct {
            fields: fields.clone(),
            flattened: flattened.clone(),
            openness: ObjectOpenness::PreserveOpaque,
        };
        let output: Value = serde_json::from_str(&super::emit(&document).unwrap()).unwrap();
        assert_eq!(
            output["components"]["schemas"]["User"]["additionalProperties"]["type"],
            "string"
        );
        assert_eq!(
            output["components"]["schemas"]["LoginRequest"]["additionalProperties"],
            json!(true)
        );
        assert_eq!(
            output["components"]["schemas"]["EventQuery"]["additionalProperties"],
            json!(false)
        );
    }

    #[test]
    fn rejects_nullable_flatten_before_emitting_a_contract() {
        let mut raw: serde_json::Value =
            serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
        raw["types"]["LoginRequest"]["flattened"] = serde_json::json!([{
            "kind": "nullable",
            "inner": {"ref": "User"}
        }]);
        let document: pons_ir::Document = serde_json::from_value(raw).unwrap();
        for mode in [super::EmitMode::Strict, super::EmitMode::Lossy] {
            assert!(
                super::emit_with_mode(&document, mode)
                    .unwrap_err()
                    .0
                    .contains("nullable flattened")
            );
        }
    }

    #[test]
    fn emits_optional_request_bodies() {
        let mut document = fixture();
        let login = document
            .operations
            .iter_mut()
            .find(|operation| operation.id[0][0] == "auth")
            .unwrap();
        login.body.as_mut().unwrap().required = false;
        let output: Value = serde_json::from_str(&super::emit(&document).unwrap()).unwrap();
        assert_eq!(
            output["paths"]["/auth/login"]["post"]["requestBody"]["required"],
            false
        );
    }

    #[test]
    fn emits_success_status_variants() {
        let mut document = fixture();
        let login = document
            .operations
            .iter()
            .position(|operation| operation.id[0][0] == "auth")
            .unwrap();
        document.operations[login].responses[0].status = ResponseStatus::Class(2);
        let output: Value = serde_json::from_str(&super::emit(&document).unwrap()).unwrap();
        assert!(output["paths"]["/auth/login"]["post"]["responses"]["2XX"].is_object());
        document.operations[login].responses[0].status = ResponseStatus::Unknown;
        let output: Value = serde_json::from_str(&super::emit(&document).unwrap()).unwrap();
        assert_eq!(
            output["paths"]["/auth/login"]["post"]["responses"]["2XX"]["x-pons"]["status"],
            "unknown"
        );
        document.operations[login].errors[0].status = ErrorStatus::Default;
        let output: Value = serde_json::from_str(&super::emit(&document).unwrap()).unwrap();
        assert!(output["paths"]["/auth/login"]["post"]["responses"]["2XX"].is_object());
        assert!(output["paths"]["/auth/login"]["post"]["responses"]["default"].is_object());
    }

    #[test]
    fn escapes_path_literals_like_the_typescript_client() {
        let mut document = fixture();
        document.operations[0].path = vec![PathSegment::Literal {
            lit: "a b?c#dé%f".into(),
        }];
        document.operations[1].path = vec![PathSegment::Literal {
            lit: "a%20b".into(),
        }];
        let output: Value = serde_json::from_str(&super::emit(&document).unwrap()).unwrap();
        let paths = output["paths"].as_object().unwrap();
        assert!(paths.contains_key("/a%20b%3Fc%23d%C3%A9%25f"), "{paths:?}");
        assert!(paths.contains_key("/a%2520b"), "{paths:?}");
    }

    #[test]
    fn rejects_default_success_and_error_dispatch_collision() {
        let mut document = fixture();
        document.operations[0].responses[0].status = ResponseStatus::Default;
        document.operations[0].errors[0].status = ErrorStatus::Default;
        assert!(
            super::emit(&document)
                .unwrap_err()
                .0
                .contains("collides with a success response")
        );
    }

    #[test]
    fn rejects_opaque_query_objects() {
        let mut document = fixture();
        let query = document.types.get_mut("EventQuery").unwrap();
        let TypeKind::Struct {
            fields, flattened, ..
        } = &query.definition
        else {
            unreachable!()
        };
        query.definition = TypeKind::Struct {
            fields: fields.clone(),
            flattened: flattened.clone(),
            openness: ObjectOpenness::PreserveOpaque,
        };
        assert!(super::emit(&document).is_err());
    }

    #[test]
    fn emits_exact_custom_media_types() {
        let mut document = fixture();
        let login = document
            .operations
            .iter_mut()
            .find(|operation| operation.id[0][0] == "auth")
            .unwrap();
        login.body.as_mut().unwrap().media_type = Some("application/vnd.acme+json".into());
        login.responses[0].media_type = Some("application/vnd.acme.session+json".into());
        login.errors[0].media_type = Some("application/vnd.acme.error+json".into());
        let output: Value = serde_json::from_str(&super::emit(&document).unwrap()).unwrap();
        let login = &output["paths"]["/auth/login"]["post"];
        assert!(login["requestBody"]["content"]["application/vnd.acme+json"].is_object());
        assert!(
            login["responses"]["200"]["content"]["application/vnd.acme.session+json"].is_object()
        );
        assert!(
            login["responses"]["4XX"]["content"]["application/vnd.acme.error+json"].is_object()
        );
    }

    #[test]
    fn distinguishes_raw_byte_bodies_from_encoded_values() {
        let mut document = fixture();
        let user = document.types.get_mut("User").unwrap();
        let TypeKind::Struct { fields, .. } = &mut user.definition else {
            unreachable!()
        };
        fields.push(pons_ir::Field {
            wire: "avatar".into(),
            ident: vec!["avatar".into()],
            ty: TypeRef::Inline(InlineType::Bytes {
                encoding: ByteEncoding::Base64url,
            }),
            presence: pons_ir::Presence::Optional,
            default: Default::default(),
            access: None,
            doc: None,
            diagnostics: Vec::new(),
            ext: Default::default(),
        });
        let login = document
            .operations
            .iter_mut()
            .find(|operation| operation.id[0][0] == "auth")
            .unwrap();
        login.body.as_mut().unwrap().content = ContentKind::Bytes;
        login.body.as_mut().unwrap().ty = TypeRef::Inline(InlineType::Bytes {
            encoding: ByteEncoding::Base64,
        });
        login.responses[0].content = ContentKind::Bytes;
        login.responses[0].body = Some(TypeRef::Inline(InlineType::Bytes {
            encoding: ByteEncoding::Hex,
        }));
        let events = document
            .operations
            .iter_mut()
            .find(|operation| operation.stream.is_some())
            .unwrap();
        events.stream.as_mut().unwrap().kind = StreamKind::Bytes {
            content_type: "application/pdf".into(),
        };

        let output: Value = serde_json::from_str(&super::emit(&document).unwrap()).unwrap();
        let login = &output["paths"]["/auth/login"]["post"];
        assert_eq!(
            login["requestBody"]["content"]["application/octet-stream"]["schema"],
            json!({})
        );
        assert_eq!(login["requestBody"]["x-pons"]["ty"]["kind"], "bytes");
        assert_eq!(
            login["responses"]["200"]["content"]["application/octet-stream"]["schema"],
            json!({})
        );
        assert_eq!(login["responses"]["200"]["x-pons"]["body"]["kind"], "bytes");
        assert_eq!(
            output["paths"]["/events/{cursor}"]["get"]["responses"]["200"]["content"]["application/pdf"]
                ["schema"],
            json!({"x-pons-stream":{"kind":"bytes","content_type":"application/pdf"}})
        );
        assert_eq!(
            output["components"]["schemas"]["User"]["properties"]["avatar"]["contentEncoding"],
            "base64url"
        );

        let mut wrapped = document.clone();
        let mut byte_alias = named_type("rawBytes");
        byte_alias.definition = TypeKind::Newtype {
            inner: TypeRef::Inline(InlineType::Bytes {
                encoding: ByteEncoding::Base64,
            }),
        };
        wrapped.types.insert("RawBytes".into(), byte_alias);
        wrapped
            .operations
            .iter_mut()
            .find(|operation| operation.id[0][0] == "auth")
            .unwrap()
            .body
            .as_mut()
            .unwrap()
            .ty = TypeRef::Named(NamedRef {
            id: "RawBytes".into(),
            variant: None,
        });
        super::emit(&wrapped).unwrap();

        let mut invalid = document.clone();
        invalid
            .operations
            .iter_mut()
            .find(|operation| operation.id[0][0] == "auth")
            .unwrap()
            .body
            .as_mut()
            .unwrap()
            .ty = string();
        assert!(
            super::emit(&invalid)
                .unwrap_err()
                .0
                .contains("raw byte bodies must use a byte type")
        );

        let mut invalid = document;
        invalid
            .operations
            .iter_mut()
            .find(|operation| operation.stream.is_some())
            .unwrap()
            .stream
            .as_mut()
            .unwrap()
            .kind = StreamKind::Bytes {
            content_type: String::new(),
        };
        assert!(
            super::emit(&invalid)
                .unwrap_err()
                .0
                .contains("invalid byte stream content type")
        );
    }

    #[test]
    fn allocates_unique_schema_and_security_component_names() {
        let mut document = fixture();
        for (id, name) in [("A", "foo"), ("A2", "foo2"), ("Z", "foo")] {
            document.types.insert(id.into(), named_type(name));
        }
        document.auth_schemes.clear();
        for id in ["Tenant", "Tenant2", "session", "tenant"] {
            document
                .auth_schemes
                .insert(id.into(), AuthScheme::Bearer { format: None });
        }

        let output: Value = serde_json::from_str(&super::emit(&document).unwrap()).unwrap();
        let schemas = &output["components"]["schemas"];
        assert!(schemas.get("Foo").is_some());
        assert!(schemas.get("Foo2").is_some());
        assert!(schemas.get("Foo3").is_some());
        let security = &output["components"]["securitySchemes"];
        assert!(security.get("Tenant").is_some());
        assert!(security.get("Tenant2").is_some());
        assert!(security.get("Tenant3").is_some(), "{security:#}");
    }

    #[test]
    fn unwraps_transparent_query_types_and_rejects_ambiguous_fields() {
        let mut document = fixture();
        let mut wrapper = named_type("eventQueryInput");
        wrapper.direction = Direction::Input;
        wrapper.definition = TypeKind::Newtype {
            inner: TypeRef::Named(NamedRef {
                id: "EventQuery".into(),
                variant: None,
            }),
        };
        document.types.insert("EventQueryInput".into(), wrapper);
        let events = document
            .operations
            .iter_mut()
            .find(|operation| operation.stream.is_some())
            .unwrap();
        events.query = Some(TypeRef::Named(NamedRef {
            id: "EventQueryInput".into(),
            variant: None,
        }));

        let output: Value = serde_json::from_str(&super::emit(&document).unwrap()).unwrap();
        assert_eq!(
            output["paths"]["/events/{cursor}"]["get"]["parameters"][1]["name"],
            "limit"
        );

        let TypeKind::Struct { fields, .. } =
            &mut document.types.get_mut("EventQuery").unwrap().definition
        else {
            unreachable!()
        };
        fields[0].ty = TypeRef::Inline(InlineType::Array {
            item: Box::new(string()),
            unique: None,
        });
        assert!(
            super::emit(&document)
                .unwrap_err()
                .0
                .contains("query fields must use scalar types")
        );
    }

    #[test]
    fn rejects_auth_and_stream_contracts_openapi_cannot_describe() {
        let mut document = fixture();
        document.auth_schemes.insert(
            "tenant".into(),
            AuthScheme::Custom {
                description: "custom auth".into(),
                ext: Default::default(),
            },
        );
        assert!(
            super::emit(&document)
                .unwrap_err()
                .0
                .contains("custom auth scheme")
        );

        let mut document = fixture();
        let operation = document
            .operations
            .iter_mut()
            .find(|operation| operation.stream.is_some())
            .unwrap();
        operation.stream.as_mut().unwrap().kind = StreamKind::Websocket {
            client: string(),
            server: string(),
        };
        assert!(super::emit(&document).unwrap_err().0.contains("WebSocket"));

        let mut document = fixture();
        document
            .operations
            .iter_mut()
            .find(|operation| operation.stream.is_some())
            .unwrap()
            .responses
            .clear();
        assert!(
            super::emit(&document)
                .unwrap_err()
                .0
                .contains("1 stream descriptions and 0 stream responses")
        );

        let mut document = fixture();
        document
            .operations
            .iter_mut()
            .find(|operation| operation.id[0][0] == "auth")
            .unwrap()
            .body
            .as_mut()
            .unwrap()
            .content = ContentKind::Stream;
        assert!(
            super::emit(&document)
                .unwrap_err()
                .0
                .contains("OpenAPI cannot describe streaming request bodies")
        );
    }

    #[test]
    fn rejects_ambiguous_paths_headers_and_oauth_urls() {
        let mut document = fixture();
        document.operations[0].path[0] = PathSegment::Literal {
            lit: "auth/{login}".into(),
        };
        assert!(
            super::emit(&document)
                .unwrap_err()
                .0
                .contains("template syntax")
        );

        let mut document = fixture();
        document.operations[0].path = vec![
            PathSegment::Literal {
                lit: "events".into(),
            },
            PathSegment::Param {
                param: "id".into(),
                ty: string(),
            },
        ];
        document.operations[1].path = vec![
            PathSegment::Literal {
                lit: "events".into(),
            },
            PathSegment::Param {
                param: "name".into(),
                ty: string(),
            },
        ];
        assert!(
            super::emit(&document)
                .unwrap_err()
                .0
                .contains("OpenAPI route pattern /events/{} uses conflicting parameter names")
        );

        let mut document = fixture();
        document.operations[1].path = vec![PathSegment::Param {
            param: "bad/name".into(),
            ty: string(),
        }];
        assert!(
            super::emit(&document)
                .unwrap_err()
                .0
                .contains("not a valid OpenAPI path parameter name")
        );

        let mut document = fixture();
        document
            .operations
            .iter_mut()
            .find(|operation| operation.stream.is_some())
            .unwrap()
            .headers[0]
            .wire = "Content-Type".into();
        assert!(
            super::emit(&document)
                .unwrap_err()
                .0
                .contains("OpenAPI ignores request header Content-Type")
        );

        let mut document = fixture();
        document.auth_schemes.insert(
            "oauth".into(),
            AuthScheme::OAuth2 {
                flows: vec![OAuthFlow {
                    kind: OAuthFlowKind::ClientCredentials,
                    authorization_url: None,
                    token_url: Some("http://identity.example.test/token".into()),
                    scopes: Default::default(),
                    ext: Default::default(),
                }],
            },
        );
        assert!(
            super::emit(&document)
                .unwrap_err()
                .0
                .contains("absolute HTTPS URL")
        );

        let mut document = fixture();
        document.auth_schemes.insert(
            "oauth".into(),
            AuthScheme::OAuth2 {
                flows: vec![OAuthFlow {
                    kind: OAuthFlowKind::ClientCredentials,
                    authorization_url: None,
                    token_url: Some("https://?query".into()),
                    scopes: Default::default(),
                    ext: Default::default(),
                }],
            },
        );
        assert!(
            super::emit(&document)
                .unwrap_err()
                .0
                .contains("absolute HTTPS URL")
        );
    }

    #[test]
    fn canonicalizes_equivalent_documents_before_emitting() {
        let mut document = fixture();
        let operation = document
            .operations
            .iter_mut()
            .find(|operation| operation.stream.is_some())
            .unwrap();
        operation.auth.alternatives = vec![vec!["session".into()], vec!["tenant".into()]];
        let expected = super::emit(&document).unwrap();
        let mut shuffled = document.clone();
        shuffled.operations.reverse();
        shuffled
            .operations
            .iter_mut()
            .find(|operation| operation.stream.is_some())
            .unwrap()
            .auth
            .alternatives
            .reverse();

        assert_eq!(super::emit(&shuffled).unwrap(), expected);
    }
}
