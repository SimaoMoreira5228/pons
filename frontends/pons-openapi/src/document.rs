use std::collections::BTreeMap;

use pons_ir::{AuthScheme, Diagnostic, DiagnosticSeverity, Document, Fidelity, IR_VERSION, Info};
use serde_json::{Map, Value};

use crate::error::ImportError;
use crate::naming::extensions;
use crate::operations;
use crate::schemas::TypeLowering;
use crate::security;

pub(crate) struct Importer<'a> {
    pub(crate) types: TypeLowering<'a>,
    pub(crate) diagnostics: Vec<Diagnostic>,
    pub(crate) auth_schemes: BTreeMap<String, AuthScheme>,
}

impl Importer<'_> {
    pub(crate) fn note_dropped(&mut self, operation: Option<&str>, construct: impl Into<String>) {
        push_dropped(&mut self.diagnostics, operation, construct);
    }
}

pub(crate) fn push_dropped(
    diagnostics: &mut Vec<Diagnostic>,
    operation: Option<&str>,
    construct: impl Into<String>,
) {
    let construct = construct.into();
    let message = match operation {
        Some(operation) => format!("operation {operation}: {construct}"),
        None => format!("document: {construct}"),
    };
    if diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "PONS-W204" && diagnostic.message == message)
    {
        return;
    }
    diagnostics.push(Diagnostic {
        severity: DiagnosticSeverity::Warning,
        code: "PONS-W204".into(),
        message,
        origin: None,
        fidelity: Some(Fidelity::Unsupported),
        capability: None,
        ext: BTreeMap::new(),
    });
}

pub(crate) fn import(bytes: &[u8]) -> Result<Document, ImportError> {
    let value: Value =
        serde_json::from_slice(bytes).map_err(|error| ImportError::Read(error.to_string()))?;
    let Value::Object(root) = value else {
        return Err(ImportError::NotDocument {
            found: json_kind(&value),
        });
    };
    let version = root
        .get("openapi")
        .and_then(Value::as_str)
        .ok_or(ImportError::MissingVersion)?;
    if !is_31(version) {
        return Err(ImportError::UnsupportedVersion {
            found: version.to_owned(),
        });
    }

    let info = root.get("info").and_then(Value::as_object).ok_or_else(|| {
        ImportError::Document(
            "the OpenAPI document has no info object, so it declares no title or version".into(),
        )
    })?;
    let title = info
        .get("title")
        .and_then(Value::as_str)
        .ok_or_else(|| ImportError::Document("the OpenAPI info object has no title".into()))?;
    let info_version = info
        .get("version")
        .and_then(Value::as_str)
        .ok_or_else(|| ImportError::Document("the OpenAPI info object has no version".into()))?;

    let empty = Map::new();
    let components = root.get("components").and_then(Value::as_object);
    let schemas = components
        .and_then(|components| components.get("schemas"))
        .and_then(Value::as_object)
        .unwrap_or(&empty);
    let security_schemes = components
        .and_then(|components| components.get("securitySchemes"))
        .and_then(Value::as_object);

    let mut diagnostics = Vec::new();
    let auth_schemes = security::schemes(security_schemes, &mut diagnostics)?;
    let mut importer = Importer {
        types: TypeLowering::new(schemas),
        diagnostics,
        auth_schemes,
    };
    let operations = operations::build(&mut importer, &root)?;
    let mut diagnostics = importer.diagnostics;

    if let Some(webhooks) = root.get("webhooks")
        && webhooks
            .as_object()
            .is_some_and(|webhooks| !webhooks.is_empty())
    {
        push_dropped(
            &mut diagnostics,
            None,
            "the document declares webhooks, which this importer does not read",
        );
    }

    let mut document = Document {
        ir_version: IR_VERSION.to_owned(),
        info: Info {
            title: title.to_owned(),
            version: info_version.to_owned(),
            description: info
                .get("description")
                .and_then(Value::as_str)
                .map(str::to_owned),
            servers: servers(root.get("servers"), &mut diagnostics),
            ext: extensions(info),
        },
        types: importer.types.into_types(),
        auth_schemes: importer.auth_schemes,
        common_errors: Vec::new(),
        operations,
        ext: extensions(&root),
        diagnostics,
    };
    document.canonicalize();
    document.validate().map_err(ImportError::Invalid)?;
    Ok(document)
}

fn servers(value: Option<&Value>, diagnostics: &mut Vec<Diagnostic>) -> Vec<String> {
    let Some(servers) = value else {
        return Vec::new();
    };
    let Some(servers) = servers.as_array() else {
        push_dropped(
            diagnostics,
            None,
            "the servers value is not an array, so it is dropped",
        );
        return Vec::new();
    };
    let mut urls = Vec::new();
    for server in servers {
        match server.get("url").and_then(Value::as_str) {
            Some(url) => urls.push(url.to_owned()),
            None => push_dropped(
                diagnostics,
                None,
                "a server entry has no url string, so it is dropped",
            ),
        }
    }
    urls
}

fn is_31(version: &str) -> bool {
    version == "3.1" || version.starts_with("3.1.")
}

fn json_kind(value: &Value) -> String {
    match value {
        Value::Null => "null".into(),
        Value::Bool(_) => "a JSON boolean".into(),
        Value::Number(_) => "a JSON number".into(),
        Value::String(_) => "a JSON string".into(),
        Value::Array(_) => "a JSON array".into(),
        Value::Object(_) => "a JSON object".into(),
    }
}
