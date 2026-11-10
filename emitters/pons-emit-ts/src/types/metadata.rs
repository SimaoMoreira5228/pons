use std::collections::BTreeMap;

use pons_ir::Document;

use crate::naming::camel;
use crate::{EmitError, FieldCase, TsOptions};

pub(super) fn type_definitions(
    document: &Document,
    options: &TsOptions,
) -> Result<String, EmitError> {
    let definitions = document
        .types
        .iter()
        .map(|(id, definition)| {
            let mut value = serde_json::to_value(&definition.definition)
                .map_err(|error| EmitError(format!("cannot encode type metadata: {error}")))?;
            value["direction"] = serde_json::to_value(definition.direction)
                .map_err(|error| EmitError(format!("cannot encode type direction: {error}")))?;
            crate::client::normalize_integer_constraints(&mut value);
            crate::client::tag_carriers(&mut value, options);
            tag_field_keys(&mut value, options);
            Ok((id.clone(), value))
        })
        .collect::<Result<BTreeMap<_, _>, _>>()?;
    let definitions = serde_json::to_string(&definitions)
        .map_err(|error| EmitError(format!("cannot encode type metadata: {error}")))?;
    let encoded = serde_json::to_string(&definitions)
        .map_err(|error| EmitError(format!("cannot encode type metadata: {error}")))?;
    Ok(format!(
        "export const typeDefinitions: Record<string, any> = Object.assign(Object.create(null), JSON.parse({encoded}));\n"
    ))
}

fn tag_field_keys(value: &mut serde_json::Value, options: &TsOptions) {
    if options.field_case != FieldCase::Camel {
        return;
    }
    match value {
        serde_json::Value::Object(fields) => {
            if fields.contains_key("presence") {
                let ident = fields
                    .get("ident")
                    .and_then(serde_json::Value::as_array)
                    .map(|ident| {
                        ident
                            .iter()
                            .filter_map(serde_json::Value::as_str)
                            .map(str::to_owned)
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                fields.insert("key".into(), serde_json::Value::from(camel(&ident)));
            }
            for child in fields.values_mut() {
                tag_field_keys(child, options);
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                tag_field_keys(item, options);
            }
        }
        _ => {}
    }
}
