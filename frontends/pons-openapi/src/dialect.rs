use std::collections::BTreeMap;

use pons_ir::{Diagnostic, DiagnosticSeverity, Fidelity};
use serde_json::{Map, Value};

const UNREPRESENTABLE: &str = "PONS-W205";

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Dialect {
    V31,
    V30,
}

impl Dialect {
    pub(crate) fn from_version(version: &str) -> Option<Self> {
        if version == "3.1" || version.starts_with("3.1.") {
            Some(Self::V31)
        } else if version == "3.0" || version.starts_with("3.0.") {
            Some(Self::V30)
        } else {
            None
        }
    }

    pub(crate) fn translate(
        self,
        schema: &mut Value,
        location: &str,
        diagnostics: &mut Vec<Diagnostic>,
    ) {
        if self == Self::V30 {
            translate_schema(schema, location, diagnostics);
        }
    }
}

fn translate_schema(schema: &mut Value, location: &str, diagnostics: &mut Vec<Diagnostic>) {
    let Value::Object(object) = schema else {
        return;
    };
    if object.get("type").and_then(Value::as_str) == Some("file") {
        warn(
            diagnostics,
            location,
            Fidelity::Opaque,
            "\"type: file\" has no OpenAPI 3.1 type and imports as an opaque value",
        );
        return;
    }
    translate_nullable(object, location, diagnostics);
    translate_exclusive(object, Bound::Minimum, location, diagnostics);
    translate_exclusive(object, Bound::Maximum, location, diagnostics);
    translate_subschemas(object, location, diagnostics);
}

fn translate_nullable(
    object: &mut Map<String, Value>,
    location: &str,
    diagnostics: &mut Vec<Diagnostic>,
) {
    match object.get("nullable") {
        Some(Value::Bool(true)) => {
            object.remove("nullable");
        }
        Some(Value::Bool(false)) => {
            object.remove("nullable");
            return;
        }
        _ => return,
    }
    match object.get("type") {
        Some(Value::String(name)) if name != "null" => {
            let name = Value::String(name.clone());
            object.insert(
                "type".into(),
                Value::Array(vec![name, Value::String("null".into())]),
            );
        }
        Some(_) => {}
        None => {
            let reference = object
                .get("$ref")
                .and_then(Value::as_str)
                .map(str::to_owned);
            match reference {
                Some(reference) => {
                    object.remove("$ref");
                    object.insert(
                        "anyOf".into(),
                        Value::Array(vec![
                            serde_json::json!({ "$ref": reference }),
                            serde_json::json!({ "type": "null" }),
                        ]),
                    );
                }
                None => {
                    object.insert("nullable".into(), Value::Bool(true));
                    warn(
                        diagnostics,
                        location,
                        Fidelity::Lossy,
                        "\"nullable: true\" has no type or $ref to make nullable and is left as written",
                    );
                }
            }
        }
    }
}

#[derive(Clone, Copy)]
enum Bound {
    Minimum,
    Maximum,
}

impl Bound {
    fn exclusive(self) -> &'static str {
        match self {
            Self::Minimum => "exclusiveMinimum",
            Self::Maximum => "exclusiveMaximum",
        }
    }

    fn plain(self) -> &'static str {
        match self {
            Self::Minimum => "minimum",
            Self::Maximum => "maximum",
        }
    }
}

fn translate_exclusive(
    object: &mut Map<String, Value>,
    bound: Bound,
    location: &str,
    diagnostics: &mut Vec<Diagnostic>,
) {
    match object.get(bound.exclusive()) {
        Some(Value::Bool(true)) => {}
        Some(Value::Bool(false)) => {
            object.remove(bound.exclusive());
            return;
        }
        _ => return,
    }
    object.remove(bound.exclusive());
    if !is_integer(object) {
        warn(
            diagnostics,
            location,
            Fidelity::Lossy,
            format!(
                "\"{}\" on a non-integer schema has no exact Pons bound, so the exclusivity is dropped",
                bound.exclusive()
            ),
        );
        return;
    }
    let Some(value) = object.get(bound.plain()).cloned() else {
        warn(
            diagnostics,
            location,
            Fidelity::Lossy,
            format!(
                "\"{}\" has no {} to adjust, so the exclusivity is dropped",
                bound.exclusive(),
                bound.plain()
            ),
        );
        return;
    };
    if !value.is_number() {
        warn(
            diagnostics,
            location,
            Fidelity::Lossy,
            format!(
                "\"{}\" has a non-numeric {}, so the exclusivity is dropped",
                bound.exclusive(),
                bound.plain()
            ),
        );
        return;
    }
    match adjusted(&value, bound) {
        Some(value) => {
            object.insert(bound.plain().into(), value);
        }
        None => warn(
            diagnostics,
            location,
            Fidelity::Lossy,
            format!(
                "\"{}\" has a {} that cannot become an inclusive bound, so the exclusivity is dropped",
                bound.exclusive(),
                bound.plain()
            ),
        ),
    }
}

fn is_integer(object: &Map<String, Value>) -> bool {
    let names = match object.get("type") {
        Some(Value::String(name)) => vec![name.as_str()],
        Some(Value::Array(types)) => types.iter().filter_map(Value::as_str).collect(),
        _ => return false,
    };
    names.contains(&"integer") && names.iter().all(|name| matches!(*name, "integer" | "null"))
}

fn adjusted(value: &Value, bound: Bound) -> Option<Value> {
    if let Some(integer) = value.as_i64() {
        return match bound {
            Bound::Minimum => integer.checked_add(1).map(Value::from),
            Bound::Maximum => integer.checked_sub(1).map(Value::from),
        };
    }
    if let Some(integer) = value.as_u64() {
        return match bound {
            Bound::Minimum => integer.checked_add(1).map(Value::from),
            Bound::Maximum => integer.checked_sub(1).map(Value::from),
        };
    }
    let float = value.as_f64()?;
    let adjusted = match bound {
        Bound::Minimum => float.floor() + 1.0,
        Bound::Maximum => float.ceil() - 1.0,
    };
    serde_json::Number::from_f64(adjusted).map(Value::Number)
}

fn translate_subschemas(
    object: &mut Map<String, Value>,
    location: &str,
    diagnostics: &mut Vec<Diagnostic>,
) {
    for key in ["items", "additionalProperties", "not"] {
        if let Some(schema) = object.get_mut(key) {
            translate_schema(schema, &format!("{location}.{key}"), diagnostics);
        }
    }
    for key in ["properties", "$defs", "definitions"] {
        let Some(Value::Object(entries)) = object.get_mut(key) else {
            continue;
        };
        for (name, schema) in entries.iter_mut() {
            translate_schema(schema, &format!("{location}.{key}.{name}"), diagnostics);
        }
    }
    for key in ["prefixItems", "allOf", "oneOf", "anyOf"] {
        let Some(Value::Array(items)) = object.get_mut(key) else {
            continue;
        };
        for (index, schema) in items.iter_mut().enumerate() {
            translate_schema(schema, &format!("{location}.{key}[{index}]"), diagnostics);
        }
    }
}

fn warn(
    diagnostics: &mut Vec<Diagnostic>,
    location: &str,
    fidelity: Fidelity,
    construct: impl Into<String>,
) {
    let message = format!("schema {location}: {}", construct.into());
    if diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == UNREPRESENTABLE && diagnostic.message == message)
    {
        return;
    }
    diagnostics.push(Diagnostic {
        severity: DiagnosticSeverity::Warning,
        code: UNREPRESENTABLE.into(),
        message,
        origin: None,
        fidelity: Some(fidelity),
        capability: None,
        ext: BTreeMap::new(),
    });
}
