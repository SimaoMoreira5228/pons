use std::collections::BTreeMap;

use pons_ir::{
    Direction, Doc, HeaderParameter, Method, Name, Operation, OperationSemantics, PathSegment,
};
use serde_json::{Map, Value};

use crate::components::Components;
use crate::document::Importer;
use crate::error::ImportError;
use crate::naming::{extensions, label as operation_label, slug, words};
use crate::parameters::{self, DeclaredParameter, Location};
use crate::request_body;
use crate::responses;
use crate::security;

pub(crate) fn build(
    importer: &mut Importer,
    root: &Map<String, Value>,
) -> Result<Vec<Operation>, ImportError> {
    let Some(paths) = root.get("paths") else {
        return Ok(Vec::new());
    };
    let Some(paths) = paths.as_object() else {
        return Err(ImportError::Document(
            "the OpenAPI paths value is not an object".into(),
        ));
    };
    let components = Components::new(root);
    let root_security = root.get("security");
    let mut operations = Vec::new();
    for (path, item) in paths {
        let Some(item) = item.as_object() else {
            importer.note_dropped(
                None,
                format!("the path item {path:?} is not an object, so it is dropped"),
            );
            continue;
        };
        if item.contains_key("$ref") {
            importer.note_dropped(
                None,
                format!(
                    "the path item {path:?} is a reference, which this importer does not follow"
                ),
            );
            continue;
        }
        let shared = parameters::read(
            item.get("parameters"),
            path,
            &components,
            &mut importer.diagnostics,
        )?;
        for (method, name) in [
            (Method::Get, "get"),
            (Method::Put, "put"),
            (Method::Post, "post"),
            (Method::Delete, "delete"),
            (Method::Options, "options"),
            (Method::Head, "head"),
            (Method::Patch, "patch"),
        ] {
            let Some(value) = item.get(name) else {
                continue;
            };
            operations.push(build_operation(
                importer,
                path,
                method,
                value,
                &shared,
                root_security,
                &components,
            )?);
        }
        if item.contains_key("trace") {
            let name = format!("{path} trace");
            importer.note_dropped(
                Some(&name),
                "the trace operation is dropped because Pons has no TRACE method",
            );
        }
    }
    Ok(operations)
}

#[allow(clippy::too_many_arguments)]
fn build_operation(
    importer: &mut Importer,
    path: &str,
    method: Method,
    value: &Value,
    shared: &[DeclaredParameter],
    root_security: Option<&Value>,
    components: &Components,
) -> Result<Operation, ImportError> {
    let Some(object) = value.as_object() else {
        return Err(ImportError::Document(format!(
            "the {path} {} operation is not an object",
            method.as_str()
        )));
    };
    let id = operation_id(object, method, path);
    let label = operation_label(&id);
    let base = format!("openapi::{label}");

    let own = parameters::read(
        object.get("parameters"),
        &label,
        components,
        &mut importer.diagnostics,
    )?;
    let parameters = parameters::merge(shared, own);

    if let Some(callbacks) = object.get("callbacks").and_then(Value::as_object)
        && !callbacks.is_empty()
    {
        importer.note_dropped(
            Some(&label),
            format!(
                "the callbacks {} are dropped because Pons has no callback representation",
                callbacks.keys().cloned().collect::<Vec<_>>().join(", ")
            ),
        );
    }

    let pieces = split_path(path)?;
    let mut segments = Vec::new();
    let mut declared_path = Vec::new();
    for piece in pieces {
        match piece {
            Piece::Literal(literal) => segments.push(PathSegment::Literal { lit: literal }),
            Piece::Param(name) => {
                let Some(parameter) = parameters.iter().find(|parameter| {
                    parameter.location == Location::Path && parameter.name == name
                }) else {
                    return Err(ImportError::Document(format!(
                        "operation {label}: the path parameter {{{name}}} is not declared"
                    )));
                };
                if !parameter.required {
                    return Err(ImportError::Document(format!(
                        "operation {label}: the path parameter {name} is declared optional, and a path parameter must be required"
                    )));
                }
                declared_path.push(name.clone());
                let ty = importer.types.schema_type(
                    &format!("{base}::path-{}", slug(&name)),
                    &name,
                    &parameter.schema,
                    Direction::Input,
                );
                segments.push(PathSegment::Param { param: name, ty });
            }
        }
    }
    for parameter in &parameters {
        if parameter.location == Location::Path && !declared_path.contains(&parameter.name) {
            importer.note_dropped(
                Some(&label),
                format!(
                    "the path parameter {} is declared but does not appear in the path, so it is dropped",
                    parameter.name
                ),
            );
        }
    }

    let body = request_body::build(
        importer,
        &base,
        &label,
        object.get("requestBody"),
        components,
    )?;
    let has_body = body.is_some();

    let mut headers = Vec::new();
    for parameter in parameters
        .iter()
        .filter(|parameter| parameter.location == Location::Header)
    {
        if parameters::owned_header(&parameter.name, has_body) {
            importer.note_dropped(
                Some(&label),
                format!(
                    "the request header {} is written by the body or the security model, so the parameter is dropped",
                    parameter.name
                ),
            );
            continue;
        }
        let ty = importer.types.schema_type(
            &format!("{base}::request-header-{}", slug(&parameter.name)),
            &parameter.name,
            &parameter.schema,
            Direction::Input,
        );
        headers.push(HeaderParameter {
            wire: parameter.name.clone(),
            ident: words(&parameter.name),
            ty,
            presence: parameters::presence(parameter.required),
            ext: BTreeMap::new(),
        });
    }

    let query = parameters::query_type(importer, &base, &label, &parameters);
    let (responses, errors) = responses::build(
        importer,
        &base,
        &label,
        method,
        object.get("responses"),
        components,
    )?;
    let security_value = object.get("security").or(root_security);
    let auth = security::requirement(
        security_value,
        &importer.auth_schemes,
        &label,
        &mut importer.diagnostics,
    )?;

    Ok(Operation {
        id,
        method,
        path: segments,
        query,
        headers,
        body,
        responses,
        errors,
        errors_inherit: false,
        stream: None,
        auth,
        provides: None,
        semantics: OperationSemantics {
            safe: matches!(method, Method::Get | Method::Head | Method::Options),
            idempotent: matches!(
                method,
                Method::Get | Method::Put | Method::Delete | Method::Head | Method::Options
            ),
            pagination: None,
        },
        doc: operation_doc(object),
        origin: None,
        ext: extensions(object),
        diagnostics: Vec::new(),
    })
}

fn operation_id(object: &Map<String, Value>, method: Method, path: &str) -> Vec<Name> {
    if let Some(id) = object
        .get("operationId")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
    {
        let parts = id
            .split('.')
            .filter(|part| !part.is_empty())
            .map(|part| vec![part.to_owned()])
            .collect::<Vec<_>>();
        if !parts.is_empty() {
            return parts;
        }
    }
    let mut path_words = Vec::new();
    let rest = path.strip_prefix('/').unwrap_or(path);
    for segment in rest.split('/').filter(|segment| !segment.is_empty()) {
        let source = segment
            .strip_prefix('{')
            .and_then(|segment| segment.strip_suffix('}'))
            .unwrap_or(segment);
        path_words.extend(words(source));
    }
    if path_words.is_empty() {
        path_words.push("root".to_owned());
    }
    vec![path_words, vec![method.as_str().to_ascii_lowercase()]]
}

fn operation_doc(object: &Map<String, Value>) -> Option<Doc> {
    let summary = object
        .get("summary")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let description = object
        .get("description")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let deprecated = object.get("deprecated").and_then(Value::as_bool);
    let tags: Vec<String> = object
        .get("tags")
        .and_then(Value::as_array)
        .map(|tags| {
            tags.iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();
    if summary.is_none() && description.is_none() && deprecated.is_none() && tags.is_empty() {
        return None;
    }
    Some(Doc {
        summary,
        description,
        deprecated,
        tags,
    })
}

enum Piece {
    Literal(String),
    Param(String),
}

fn split_path(path: &str) -> Result<Vec<Piece>, ImportError> {
    let Some(rest) = path.strip_prefix('/') else {
        return Err(ImportError::Document(format!(
            "the OpenAPI path {path:?} does not start with '/'"
        )));
    };
    let mut pieces = Vec::new();
    if rest.is_empty() {
        return Ok(pieces);
    }
    for segment in rest.split('/') {
        if let Some(name) = segment
            .strip_prefix('{')
            .and_then(|segment| segment.strip_suffix('}'))
        {
            if name.is_empty() || name.contains('{') || name.contains('}') || name.contains('/') {
                return Err(ImportError::Document(format!(
                    "the OpenAPI path {path:?} has an invalid parameter segment {segment:?}"
                )));
            }
            pieces.push(Piece::Param(name.to_owned()));
        } else if segment.contains('{') || segment.contains('}') {
            return Err(ImportError::Document(format!(
                "the OpenAPI path {path:?} has a parameter that is not a whole segment: {segment:?}"
            )));
        } else {
            pieces.push(Piece::Literal(segment.to_owned()));
        }
    }
    Ok(pieces)
}
