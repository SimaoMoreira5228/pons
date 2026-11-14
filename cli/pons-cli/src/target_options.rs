use std::collections::BTreeMap;
use std::path::PathBuf;

use pons_emit_ts::{
    DateMode, ErrorStrategy, FieldCase, Int64, Namespaces, Newtypes, Runtime, TsOptions,
    ValidationBackend,
};
use pons_plugin::EmitMode;
use serde_json::Value;

use crate::CliError;
use crate::config::{self, OUT_DIR, Target};
use crate::validation_options::{self, VALIDATION_BACKENDS};

const MODE: &[&str] = &["strict", "lossy"];
const ERRORS: &[&str] = &["neverthrow", "throw"];
const NAMESPACES: &[&str] = &["nested", "flat"];
const FIELD_CASE: &[&str] = &["wire", "camel"];
const NEWTYPES: &[&str] = &["alias", "branded"];
const INT64: &[&str] = &["bigint", "string", "number"];
const DATE_MODE: &[&str] = &["string", "date"];
const RUNTIME: &[&str] = &["vendor", "package"];

const TS: &[(&str, &[&str])] = &[
    ("mode", MODE),
    ("errors", ERRORS),
    ("namespaces", NAMESPACES),
    ("field_case", FIELD_CASE),
    ("newtypes", NEWTYPES),
    ("int64", INT64),
    ("date_mode", DATE_MODE),
    ("runtime", RUNTIME),
    ("validation", VALIDATION_BACKENDS),
];

const OPENAPI: &[(&str, &[&str])] = &[("mode", MODE)];

pub(crate) fn vocabulary(
    target: &str,
) -> Option<&'static [(&'static str, &'static [&'static str])]> {
    match target {
        "ts" => Some(TS),
        "openapi" => Some(OPENAPI),
        _ => None,
    }
}

pub(crate) fn check_values(target: &Target<'_>) -> Result<(), CliError> {
    word(target, "mode", target.get("mode"), MODE)?;
    if let Some(value) = target.get(OUT_DIR) {
        out_dir_path(target, value)?;
    }
    match vocabulary(&target.target) {
        Some(vocabulary) => {
            for (key, allowed) in vocabulary {
                if *key != "mode" && *key != "validation" {
                    word(target, key, target.get(key), allowed)?;
                }
            }
            if target.target == "ts" {
                validation_options::validation_options(target, None)?;
            }
        }
        None => {
            for (key, value) in &target.values {
                if key != OUT_DIR && key != "mode" {
                    to_json(target, key, value)?;
                }
            }
        }
    }
    Ok(())
}

pub(crate) fn mode(target: &Target<'_>, flag: Option<EmitMode>) -> Result<EmitMode, CliError> {
    if let Some(mode) = flag {
        return Ok(mode);
    }
    Ok(match word(target, "mode", target.get("mode"), MODE)? {
        Some("lossy") => EmitMode::Lossy,
        _ => EmitMode::Strict,
    })
}

pub(crate) fn errors(
    target: &Target<'_>,
    flag: Option<ErrorStrategy>,
) -> Result<Option<ErrorStrategy>, CliError> {
    if let Some(errors) = flag {
        return Ok(Some(errors));
    }
    Ok(
        match word(target, "errors", target.get("errors"), ERRORS)? {
            Some("throw") => Some(ErrorStrategy::Throw),
            Some(_) => Some(ErrorStrategy::Neverthrow),
            None => None,
        },
    )
}

pub(crate) fn ts_options(
    target: &Target<'_>,
    errors: Option<ErrorStrategy>,
    mode: EmitMode,
    backend: Option<ValidationBackend>,
) -> Result<TsOptions, CliError> {
    let mut options = TsOptions {
        mode: match mode {
            EmitMode::Strict => pons_emit_ts::EmitMode::Strict,
            EmitMode::Lossy => pons_emit_ts::EmitMode::Lossy,
        },
        errors: errors.unwrap_or_default(),
        validation: validation_options::validation_options(target, backend)?,
        ..TsOptions::default()
    };
    for (key, allowed) in TS {
        if matches!(*key, "mode" | "errors" | "validation") {
            continue;
        }
        let Some(word) = word(target, key, target.get(key), allowed)? else {
            continue;
        };
        match (*key, word) {
            ("namespaces", "flat") => options.namespaces = Namespaces::Flat,
            ("namespaces", _) => options.namespaces = Namespaces::Nested,
            ("field_case", "camel") => options.field_case = FieldCase::Camel,
            ("field_case", _) => options.field_case = FieldCase::Wire,
            ("newtypes", "branded") => options.newtypes = Newtypes::Branded,
            ("newtypes", _) => options.newtypes = Newtypes::Alias,
            ("int64", "string") => options.int64 = Int64::String,
            ("int64", "number") => options.int64 = Int64::Number,
            ("int64", _) => options.int64 = Int64::Bigint,
            ("date_mode", "date") => options.date_mode = DateMode::Date,
            ("date_mode", _) => options.date_mode = DateMode::String,
            ("runtime", "package") => options.runtime = Runtime::Package,
            ("runtime", _) => options.runtime = Runtime::Vendor,
            _ => {}
        }
    }
    Ok(options)
}

pub(crate) fn wire_options(target: &Target<'_>) -> Result<BTreeMap<String, Value>, CliError> {
    let mut options = BTreeMap::new();
    for (key, value) in &target.values {
        if key != OUT_DIR && key != "mode" {
            options.insert(key.clone(), to_json(target, key, value)?);
        }
    }
    Ok(options)
}

pub(crate) fn ts_words(options: &TsOptions) -> BTreeMap<String, Value> {
    let mut words = [
        (
            "mode",
            match options.mode {
                pons_emit_ts::EmitMode::Strict => "strict",
                pons_emit_ts::EmitMode::Lossy => "lossy",
            },
        ),
        (
            "errors",
            match options.errors {
                ErrorStrategy::Neverthrow => "neverthrow",
                ErrorStrategy::Throw => "throw",
            },
        ),
        (
            "namespaces",
            match options.namespaces {
                Namespaces::Nested => "nested",
                Namespaces::Flat => "flat",
            },
        ),
        (
            "field_case",
            match options.field_case {
                FieldCase::Wire => "wire",
                FieldCase::Camel => "camel",
            },
        ),
        (
            "newtypes",
            match options.newtypes {
                Newtypes::Alias => "alias",
                Newtypes::Branded => "branded",
            },
        ),
        (
            "int64",
            match options.int64 {
                Int64::Bigint => "bigint",
                Int64::String => "string",
                Int64::Number => "number",
            },
        ),
        (
            "date_mode",
            match options.date_mode {
                DateMode::String => "string",
                DateMode::Date => "date",
            },
        ),
        (
            "runtime",
            match options.runtime {
                Runtime::Vendor => "vendor",
                Runtime::Package => "package",
            },
        ),
    ]
    .into_iter()
    .map(|(key, word)| (key.to_owned(), Value::String(word.to_owned())))
    .collect::<BTreeMap<_, _>>();
    let mut validation = serde_json::Map::new();
    validation.insert(
        "backend".into(),
        Value::String(options.validation.backend.name().into()),
    );
    if let Some(specifier) = &options.validation.import_from {
        validation.insert("import_from".into(), Value::String(specifier.clone()));
    }
    words.insert("validation".into(), Value::Object(validation));
    words
}

pub(crate) fn out_dir_path(target: &Target<'_>, value: &toml::Value) -> Result<PathBuf, CliError> {
    match value.as_str() {
        Some(path) => Ok(PathBuf::from(path)),
        None => Err(target.error(&format!(
            "{OUT_DIR} is {}; it is a path to a directory",
            config::kind(value)
        ))),
    }
}

fn word<'a>(
    target: &Target<'_>,
    key: &str,
    value: Option<&'a toml::Value>,
    allowed: &[&str],
) -> Result<Option<&'a str>, CliError> {
    let Some(value) = value else {
        return Ok(None);
    };
    let Some(word) = value.as_str() else {
        return Err(target.error(&format!(
            "{key} is {}; it is {}",
            config::kind(value),
            allowed.join(" or ")
        )));
    };
    if !allowed.contains(&word) {
        return Err(target.error(&format!(
            "{key} is {word:?}; it is {}",
            allowed.join(" or ")
        )));
    }
    Ok(Some(word))
}

fn to_json(target: &Target<'_>, key: &str, value: &toml::Value) -> Result<Value, CliError> {
    let key_error = |value: &toml::Value| {
        target.error(&format!(
            "{key} is {}; an emitter reads options as JSON",
            config::kind(value)
        ))
    };
    Ok(match value {
        toml::Value::String(text) => Value::String(text.clone()),
        toml::Value::Integer(number) => Value::Number((*number).into()),
        toml::Value::Float(number) => serde_json::Number::from_f64(*number)
            .map(Value::Number)
            .ok_or_else(|| key_error(value))?,
        toml::Value::Boolean(flag) => Value::Bool(*flag),
        toml::Value::Array(entries) => Value::Array(
            entries
                .iter()
                .map(|entry| to_json(target, key, entry))
                .collect::<Result<_, _>>()?,
        ),
        toml::Value::Table(entries) => Value::Object(
            entries
                .iter()
                .map(|(entry, value)| Ok((entry.clone(), to_json(target, key, value)?)))
                .collect::<Result<serde_json::Map<String, Value>, CliError>>()?,
        ),
        toml::Value::Datetime(_) => return Err(key_error(value)),
    })
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    fn table<'a>(file: &'a Path, name: &str, source: &str) -> Target<'a> {
        Target::with_values(Some(file), name, source.parse::<toml::Table>().unwrap())
    }

    fn the_file() -> &'static Path {
        Path::new("/project/pons.toml")
    }

    #[test]
    fn the_cli_keys_never_reach_an_emitter() {
        let target = table(
            the_file(),
            "python",
            "out_dir = \"out\"\nmode = \"lossy\"\npackage = \"acme\"\nretries = [1, 2]\nnested = { deep = true }\n",
        );
        let options = wire_options(&target).unwrap();
        assert_eq!(options.len(), 3, "{options:?}");
        assert_eq!(options["package"], Value::String("acme".into()));
        assert_eq!(options["retries"], serde_json::json!([1, 2]));
        assert_eq!(options["nested"], serde_json::json!({ "deep": true }));
    }

    #[test]
    fn every_typescript_option_reads_back_to_itself() {
        for (key, allowed) in TS {
            if matches!(*key, "mode" | "errors" | "validation") {
                continue;
            }
            for value in *allowed {
                let source = format!("{key} = {value:?}");
                let target = table(the_file(), "ts", &source);
                check_values(&target)
                    .unwrap_or_else(|error| panic!("{source:?}: {}", error.message()));
                let options = ts_options(&target, None, EmitMode::Strict, None).unwrap();
                assert_eq!(
                    ts_words(&options)[*key],
                    Value::String((*value).into()),
                    "{source:?}"
                );
            }
        }
    }
}
