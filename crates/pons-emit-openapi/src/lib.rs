mod operations;
mod schemas;
mod security;

use std::collections::{BTreeMap, BTreeSet};

use pons_ir::{
    Coverage, Document, coverage_for_document, operation_subject, strict_violation_message,
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
                .map(|(id, ty)| {
                    (
                        schema_name_base(id, &ty.name),
                        role(id).is_some(),
                        id.clone(),
                    )
                })
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

fn schema_name_base(id: &str, name: &[String]) -> String {
    let base = readable_name(name);
    match role(id) {
        Some(role) => format!("{base}{}", capitalize(role)),
        None => base,
    }
}

fn role(id: &str) -> Option<&str> {
    let role = id.rsplit_once('#')?.1;
    (!role.is_empty() && role.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'))
        .then_some(role)
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
mod tests {
    use pons_ir::{
        AuthScheme, ByteEncoding, ContentKind, Coverage, Direction, ErrorStatus, InlineType,
        NamedRef, OAuthFlow, OAuthFlowKind, ObjectOpenness, OpaqueWire, PathSegment, Presence,
        ResponseHeader, ResponseStatus, StreamKind, TypeDef, TypeKind, TypeRef,
    };
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
        let email = &output["components"]["schemas"]["LoginRequest"]["properties"]["email"];
        assert_eq!(email["x-pons-format"], json!("email"));
        assert!(email.get("format").is_none(), "{email:#}");
        assert!(
            output["x-pons"]["ext"]["test.example.source/v1"]["kept"]
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
            ext: Default::default(),
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
            ext: Default::default(),
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
    fn strict_mode_never_claims_more_capability_than_canonical_coverage() {
        let mut rejecting = fixture();
        for id in ["User", "LoginRequest"] {
            let TypeKind::Struct { openness, .. } =
                &mut rejecting.types.get_mut(id).unwrap().definition
            else {
                unreachable!()
            };
            *openness = ObjectOpenness::Reject;
        }
        let cases: [(pons_ir::Document, &[&str], &[&str]); 2] = [
            (
                fixture(),
                &[
                    "operations.auth.login",
                    "operations.events.watch",
                    "types.LoginRequest",
                    "types.User",
                ],
                &["types.EventQuery", "types.LoginError"],
            ),
            (
                rejecting,
                &["operations.auth.login", "operations.events.watch"],
                &[
                    "types.EventQuery",
                    "types.LoginError",
                    "types.LoginRequest",
                    "types.User",
                ],
            ),
        ];

        for (document, opaque, exact) in cases {
            let canonical = pons_ir::canonical_coverage(&document, false).unwrap();
            let emitted = super::emit_with_mode(&document, super::EmitMode::Strict).unwrap();
            let reported: BTreeMap<String, pons_ir::CoverageLevel> =
                emitted.coverage.levels().collect();

            assert_eq!(
                reported.keys().collect::<Vec<_>>(),
                canonical.keys().collect::<Vec<_>>()
            );
            for (subject, level) in &canonical {
                assert!(
                    reported[subject] >= *level,
                    "the emitter reports {subject} as {} where canonical coverage is {}",
                    reported[subject].as_str(),
                    level.as_str()
                );
            }
            assert_eq!(emitted.warnings, Vec::<String>::new());
            assert_eq!(
                emitted.coverage.opaque,
                opaque.iter().map(|s| (*s).to_owned()).collect()
            );
            assert_eq!(
                emitted.coverage.exact,
                exact.iter().map(|s| (*s).to_owned()).collect()
            );
        }
    }

    #[test]
    fn strict_mode_rejects_narrowed_coverage_but_carries_opaque_coverage() {
        for (coverage, expected) in [
            (
                Coverage {
                    lossy: ["types.User".into()].into_iter().collect(),
                    ..Coverage::default()
                },
                "lossy coverage",
            ),
            (
                Coverage {
                    unsupported: ["types.User".into()].into_iter().collect(),
                    ..Coverage::default()
                },
                "unsupported coverage",
            ),
        ] {
            let error = super::coverage_warnings(super::EmitMode::Strict, &coverage)
                .unwrap_err()
                .0;
            assert!(error.contains(expected), "{error}");
            assert!(error.contains("types.User"), "{error}");
            assert!(error.contains("in strict mode"), "{error}");
            assert!(
                !super::coverage_warnings(super::EmitMode::Lossy, &coverage)
                    .unwrap()
                    .is_empty()
            );
        }

        let carried = Coverage {
            opaque: ["types.User".into()].into_iter().collect(),
            ..Coverage::default()
        };
        assert!(super::coverage_warnings(super::EmitMode::Strict, &carried).is_ok());
    }

    #[test]
    fn strict_mode_names_every_violating_coverage_class() {
        let coverage = Coverage {
            lossy: ["types.User".into()].into_iter().collect(),
            unsupported: ["operations.auth.login".into()].into_iter().collect(),
            ..Coverage::default()
        };
        let error = super::coverage_warnings(super::EmitMode::Strict, &coverage)
            .unwrap_err()
            .0;
        assert_eq!(
            error,
            "OpenAPI emitter reported unsupported coverage in strict mode: operations.auth.login; \
             lossy coverage in strict mode: types.User",
            "every violating class is named, worst class first"
        );
        assert!(super::coverage_warnings(super::EmitMode::Lossy, &coverage).is_ok());
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
                    scopes: BTreeMap::from([("events:read".into(), "Read events".into())]),
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
        document.operations[login].errors[0].status = ErrorStatus::Exact(503);
        let output: Value = serde_json::from_str(&super::emit(&document).unwrap()).unwrap();
        let responses = &output["paths"]["/auth/login"]["post"]["responses"];
        assert_eq!(responses["default"]["x-pons"]["status"], "unknown");
        assert!(responses.get("2XX").is_none());
        assert!(responses["503"].is_object());
        document.operations[login].responses[0].status = ResponseStatus::Default;
        let output: Value = serde_json::from_str(&super::emit(&document).unwrap()).unwrap();
        let responses = &output["paths"]["/auth/login"]["post"]["responses"];
        assert_eq!(responses["default"]["x-pons"]["status"], "default");
        assert!(responses.get("2XX").is_none());
        assert!(responses["503"].is_object());
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

    fn component_names(document: &pons_ir::Document) -> BTreeMap<String, String> {
        let output: Value = serde_json::from_str(&super::emit(document).unwrap()).unwrap();
        output["components"]["schemas"]
            .as_object()
            .unwrap()
            .iter()
            .map(|(name, schema)| {
                (
                    name.clone(),
                    schema["x-pons"]["type"]["id"].as_str().unwrap().to_owned(),
                )
            })
            .collect()
    }

    #[test]
    fn names_each_role_of_one_type_after_the_role_its_id_carries() {
        fn carriers(ids: &[&str]) -> pons_ir::Document {
            let mut document = fixture();
            for id in ids {
                let mut definition = named_type("q");
                definition.direction = Direction::Input;
                document.types.insert((*id).into(), definition);
            }
            document
        }

        let both = component_names(&carriers(&["router::Q#input", "router::Q#query"]));
        assert_eq!(both["QInput"], "router::Q#input");
        assert_eq!(both["QQuery"], "router::Q#query");
        assert!(
            both.keys().all(|name| !name.contains('#')),
            "{both:?} leaks an id role"
        );
        assert_eq!(
            both["User"], "User",
            "a name one definition publishes keeps the plain name"
        );

        let reversed = component_names(&carriers(&["a::Q#query", "z::Q#input"]));
        assert_eq!(reversed["QInput"], "z::Q#input");
        assert_eq!(reversed["QQuery"], "a::Q#query");

        let lone = component_names(&carriers(&["router::Q#query"]));
        assert_eq!(
            lone["QQuery"], "router::Q#query",
            "pruning a sibling leaves the survivor's name alone"
        );
        assert!(!lone.contains_key("QInput"));
    }

    #[test]
    fn a_role_suffixed_name_never_takes_the_name_another_type_publishes() {
        let mut document = fixture();
        for (id, name) in [
            ("a::Q#input", "q"),
            ("b::Q#input", "q"),
            ("c::qInput", "qInput"),
        ] {
            let mut definition = named_type(name);
            definition.direction = Direction::Input;
            document.types.insert(id.into(), definition);
        }

        let names = component_names(&document);
        assert_eq!(names["QInput"], "c::qInput");
        assert_eq!(names["QInput2"], "a::Q#input");
        assert_eq!(names["QInput3"], "b::Q#input");
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
        document.operations[1].headers[0].wire = "Accept".into();
        assert!(
            super::emit(&document).unwrap_err().0.contains(
                "request header Accept is owned by the request body or the security model"
            )
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
                .contains("OAuth2 token URL for oauth must be an absolute HTTPS URL")
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
                .contains("OAuth2 token URL for oauth must be an absolute HTTPS URL")
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
