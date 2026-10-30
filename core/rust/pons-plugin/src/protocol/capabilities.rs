use std::collections::BTreeMap;

use pons_ir::{
    AuthScheme, Direction, Document, EnumRepr, Fidelity, ObjectOpenness, PaginationRequest,
    StreamKind, coverage_for_document, operation_subject,
};
use serde_json::Value;

use super::manifest::EmitterManifest;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilityWarning(pub String);

impl EmitterManifest {
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
        for namespace in document.extension_namespaces() {
            match self.capabilities.extensions.get(&namespace) {
                None => warnings.push(CapabilityWarning(format!(
                    "{} does not declare extension namespace {namespace}",
                    self.name
                ))),
                Some(Fidelity::Lossy) => warnings.push(CapabilityWarning(format!(
                    "{} does not preserve extension namespace {namespace} exactly",
                    self.name
                ))),
                Some(Fidelity::Unsupported) => warnings.push(CapabilityWarning(format!(
                    "{} does not support extension namespace {namespace}",
                    self.name
                ))),
                Some(Fidelity::Exact | Fidelity::Opaque) => {}
            }
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
            if let Some(pagination) = &operation.semantics.pagination {
                let capability = pagination_capability(&pagination.request);
                if !self
                    .capabilities
                    .pagination
                    .iter()
                    .any(|value| value == capability)
                {
                    warnings.push(CapabilityWarning(format!(
                        "{} does not declare support for {capability} in {}",
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
                pons_ir::TypeKind::Enum { repr, .. } => {
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
                pons_ir::TypeKind::Struct {
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

fn pagination_capability(request: &PaginationRequest) -> &'static str {
    match request {
        PaginationRequest::Cursor { .. } => "pagination.cursor",
        PaginationRequest::Offset { .. } => "pagination.offset",
    }
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
    use serde_json::json;

    use super::*;

    fn manifest(pagination: &[&str]) -> EmitterManifest {
        serde_json::from_value(json!({
            "name": "go",
            "version": "0.1.0",
            "protocol": {"min": "3.0", "max": "3.0"},
            "ir_version": {"min": "0.2", "max": "0.2"},
            "capabilities": {
                "pagination": pagination,
                "object_openness": ["reject"],
            }
        }))
        .unwrap()
    }

    fn paginated(kind: &str) -> Document {
        let document = json!({
            "ir_version": "0.2",
            "info": {"title": "Events", "version": "1"},
            "types": {
                "ListQuery": {
                    "kind": "struct",
                    "openness": "reject",
                    "name": ["list", "query"],
                    "direction": "input",
                    "fields": [{
                        "wire": kind,
                        "ident": [kind],
                        "ty": {"kind": "string"},
                        "presence": "optional"
                    }]
                }
            },
            "operations": [{
                "id": [["events"], ["list"]],
                "method": "GET",
                "path": [{"lit": "events"}],
                "query": {"ref": "ListQuery"},
                "responses": [{
                    "status": 200,
                    "content": "json",
                    "body": {"kind": "array", "item": {"kind": "string"}}
                }],
                "auth": {"alternatives": [], "optional": false},
                "semantics": {
                    "safe": true,
                    "idempotent": true,
                    "pagination": {
                        "request": {"kind": kind, "field": kind},
                        "items": "",
                        "next": "/next"
                    }
                }
            }]
        });
        pons_ir::decode(&serde_json::to_vec(&document).unwrap()).unwrap()
    }

    #[test]
    fn warns_for_undeclared_pagination_kinds() {
        let cursor = paginated("cursor");
        assert_eq!(
            manifest(&[]).unsupported_features(&cursor),
            vec![CapabilityWarning(
                "go does not declare support for pagination.cursor in events.list".into()
            )]
        );

        let offset = paginated("offset");
        assert_eq!(
            manifest(&["pagination.cursor"]).unsupported_features(&offset),
            vec![CapabilityWarning(
                "go does not declare support for pagination.offset in events.list".into()
            )]
        );
    }

    #[test]
    fn stays_quiet_for_declared_and_absent_pagination() {
        assert!(
            manifest(&["pagination.cursor"])
                .unsupported_features(&paginated("cursor"))
                .is_empty()
        );
        assert!(
            manifest(&["pagination.offset"])
                .unsupported_features(&paginated("offset"))
                .is_empty()
        );

        let mut unpaginated = paginated("cursor");
        unpaginated.operations[0].semantics.pagination = None;
        assert!(manifest(&[]).unsupported_features(&unpaginated).is_empty());
    }
}
