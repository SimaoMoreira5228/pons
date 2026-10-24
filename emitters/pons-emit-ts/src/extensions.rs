use std::collections::BTreeSet;

use pons_ir::{Document, EXT_ALIAS};
use serde_json::Value;

const UNDERSTOOD: &[&str] = &[EXT_ALIAS];

pub(super) fn warnings(document: &Document) -> Result<Vec<String>, String> {
    let value = serde_json::to_value(document)
        .map_err(|error| format!("cannot read the document extensions: {error}"))?;
    let mut keys = BTreeSet::new();
    collect(&value, &mut keys);
    Ok(keys
        .into_iter()
        .filter(|key| !UNDERSTOOD.contains(&key.as_str()))
        .map(|key| {
            format!(
                "TypeScript does not understand the extension namespace {key}, so the emitted \
                 client ignores it"
            )
        })
        .collect())
}

fn collect(value: &Value, keys: &mut BTreeSet<String>) {
    match value {
        Value::Object(fields) => {
            if let Some(Value::Object(ext)) = fields.get("ext") {
                keys.extend(ext.keys().cloned());
            }
            for (name, child) in fields {
                if matches!(name.as_str(), "ext" | "default") {
                    continue;
                }
                collect(child, keys);
            }
        }
        Value::Array(items) => {
            for item in items {
                collect(item, keys);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::warnings;
    use pons_ir::Document;
    use serde_json::json;

    fn document(ext: serde_json::Value) -> Document {
        let mut raw: serde_json::Value =
            serde_json::from_str(include_str!("../../../spec/fixtures/login.json")).unwrap();
        raw["ext"] = ext;
        raw["types"]["User"]["ext"] = json!({});
        raw["types"]["User"]["fields"][0]["ext"] = json!({});
        serde_json::from_value(raw).unwrap()
    }

    #[test]
    fn reports_each_namespace_version_once_and_skips_the_input_aliases() {
        let mut ext = json!({"test.example.source/v1": {"kept": true}});
        ext["com.pons.alias/v1"] = json!({});
        let warnings = warnings(&document(ext)).unwrap();
        assert_eq!(
            warnings,
            vec![
                "TypeScript does not understand the extension namespace test.example.source/v1, \
                 so the emitted client ignores it"
                    .to_owned()
            ],
            "the fixture carries the same namespace at the document, the type, and the field, and \
             the reserved alias extension is one the client already honors"
        );
    }

    #[test]
    fn a_namespace_version_is_a_separate_warning() {
        let warnings = warnings(&document(json!({
            "test.example.source/v1": {},
            "test.example.source/v2": {},
            "other.example.audit/v1": {}
        })))
        .unwrap();
        assert_eq!(warnings.len(), 3, "{warnings:?}");
        assert!(warnings[0].contains("other.example.audit/v1"));
        assert!(warnings[1].contains("test.example.source/v1"));
        assert!(warnings[2].contains("test.example.source/v2"));
    }

    #[test]
    fn a_default_value_that_spells_ext_is_not_an_extension() {
        let mut raw = serde_json::to_value(document(json!({}))).unwrap();
        raw["types"]["User"]["fields"][0]["default"] =
            json!({"ext": {"test.example.source/v1": {}}});
        let document: Document = serde_json::from_value(raw).unwrap();
        assert!(warnings(&document).unwrap().is_empty());
    }
}
