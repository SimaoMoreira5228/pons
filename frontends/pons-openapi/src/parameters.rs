use pons_ir::{Diagnostic, Direction, Presence, TypeRef};
use serde_json::{Map, Value};

use crate::components::{Components, Section};
use crate::document::{Importer, push_dropped};
use crate::error::ImportError;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Location {
    Path,
    Query,
    Header,
}

pub(crate) struct DeclaredParameter {
    pub(crate) name: String,
    pub(crate) location: Location,
    pub(crate) required: bool,
    pub(crate) schema: Value,
}

pub(crate) fn read(
    list: Option<&Value>,
    operation: &str,
    components: &Components,
    diagnostics: &mut Vec<Diagnostic>,
) -> Result<Vec<DeclaredParameter>, ImportError> {
    let Some(list) = list else {
        return Ok(Vec::new());
    };
    let Some(list) = list.as_array() else {
        return Err(ImportError::Document(format!(
            "operation {operation}: the parameters value is not an array"
        )));
    };
    let mut parameters = Vec::new();
    for entry in list {
        let Some(value) = components.resolve(entry, Section::Parameters) else {
            push_dropped(
                diagnostics,
                Some(operation),
                "a parameter references a missing components.parameters entry and is dropped",
            );
            continue;
        };
        let Some(object) = value.as_object() else {
            push_dropped(
                diagnostics,
                Some(operation),
                "a parameter is not an object and is dropped",
            );
            continue;
        };
        let (Some(name), Some(location)) = (
            object.get("name").and_then(Value::as_str),
            object.get("in").and_then(Value::as_str),
        ) else {
            push_dropped(
                diagnostics,
                Some(operation),
                "a parameter has no name or location and is dropped",
            );
            continue;
        };
        if object.contains_key("content") {
            push_dropped(
                diagnostics,
                Some(operation),
                format!(
                    "the parameter {name:?} uses content instead of a schema, so it is dropped"
                ),
            );
            continue;
        }
        if let Some(style) = object.get("style").and_then(Value::as_str) {
            let default = match location {
                "path" | "header" => "simple",
                "query" => "form",
                _ => "",
            };
            if style != default {
                push_dropped(
                    diagnostics,
                    Some(operation),
                    format!(
                        "the {location} parameter {name:?} uses the unsupported style {style:?} and is dropped"
                    ),
                );
                continue;
            }
        }
        let location = match location {
            "path" => Location::Path,
            "query" => Location::Query,
            "header" => Location::Header,
            "cookie" => {
                push_dropped(
                    diagnostics,
                    Some(operation),
                    format!(
                        "the cookie parameter {name:?} is dropped because Pons has no cookie parameter carrier"
                    ),
                );
                continue;
            }
            other => {
                push_dropped(
                    diagnostics,
                    Some(operation),
                    format!(
                        "the parameter {name:?} has the unknown location {other:?} and is dropped"
                    ),
                );
                continue;
            }
        };
        parameters.push(DeclaredParameter {
            name: name.to_owned(),
            location,
            required: object
                .get("required")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            schema: object.get("schema").cloned().unwrap_or(Value::Bool(true)),
        });
    }
    Ok(parameters)
}

pub(crate) fn merge(
    shared: &[DeclaredParameter],
    own: Vec<DeclaredParameter>,
) -> Vec<DeclaredParameter> {
    let mut merged = std::collections::BTreeMap::new();
    for parameter in shared.iter().chain(own.iter()) {
        merged.insert(
            (parameter.name.clone(), parameter.location),
            DeclaredParameter {
                name: parameter.name.clone(),
                location: parameter.location,
                required: parameter.required,
                schema: parameter.schema.clone(),
            },
        );
    }
    merged.into_values().collect()
}

pub(crate) fn query_type(
    importer: &mut Importer<'_>,
    base: &str,
    label: &str,
    parameters: &[DeclaredParameter],
) -> Option<TypeRef> {
    let query = parameters
        .iter()
        .filter(|parameter| parameter.location == Location::Query)
        .collect::<Vec<_>>();
    if query.is_empty() {
        return None;
    }
    let mut properties = Map::new();
    let mut required = Vec::new();
    for parameter in query {
        properties.insert(parameter.name.clone(), parameter.schema.clone());
        if parameter.required {
            required.push(Value::String(parameter.name.clone()));
        }
    }
    let mut schema = Map::new();
    schema.insert("type".into(), Value::String("object".into()));
    schema.insert("properties".into(), Value::Object(properties));
    if !required.is_empty() {
        schema.insert("required".into(), Value::Array(required));
    }
    schema.insert("additionalProperties".into(), Value::Bool(false));
    Some(importer.types.inline(
        &format!("{base}::query"),
        &format!("{label} query"),
        &Value::Object(schema),
        Direction::Input,
    ))
}

pub(crate) fn owned_header(name: &str, has_body: bool) -> bool {
    match name.to_ascii_lowercase().as_str() {
        "content-type" => has_body,
        "accept" | "authorization" | "cookie" => true,
        _ => false,
    }
}

pub(crate) fn presence(required: bool) -> Presence {
    if required {
        Presence::Required
    } else {
        Presence::Optional
    }
}
