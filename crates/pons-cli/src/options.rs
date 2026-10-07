use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::PathBuf;

use pons_emit_ts::{
    DateMode, ErrorStrategy, FieldCase, Int64, Namespaces, Newtypes, Runtime, TsOptions, Validation,
};
use pons_plugin::EmitMode;
use serde_json::Value;

use crate::CliError;
use crate::config::{self, Config, DOCUMENT_TABLE, OUT_DIR, Target};

const MODE: &[&str] = &["strict", "lossy"];
const ERRORS: &[&str] = &["neverthrow", "throw"];
const NAMESPACES: &[&str] = &["nested", "flat"];
const FIELD_CASE: &[&str] = &["wire", "camel"];
const NEWTYPES: &[&str] = &["alias", "branded"];
const INT64: &[&str] = &["bigint", "string", "number"];
const DATE_MODE: &[&str] = &["string", "date"];
const RUNTIME: &[&str] = &["vendor", "package"];
const VALIDATION: &[&str] = &["none", "builtin", "zod"];

const TS: &[(&str, &[&str])] = &[
    ("mode", MODE),
    ("errors", ERRORS),
    ("namespaces", NAMESPACES),
    ("field_case", FIELD_CASE),
    ("newtypes", NEWTYPES),
    ("int64", INT64),
    ("date_mode", DATE_MODE),
    ("runtime", RUNTIME),
    ("validation", VALIDATION),
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

pub(crate) struct EmitArgs {
    pub(crate) target: String,
    pub(crate) mode: Option<EmitMode>,
    pub(crate) errors: Option<ErrorStrategy>,
    pub(crate) positionals: Vec<OsString>,
}

pub(crate) struct Resolved {
    pub(crate) target: String,
    pub(crate) document: PathBuf,
    pub(crate) out_dir: PathBuf,
    pub(crate) mode: EmitMode,
    pub(crate) built: Built,
    pub(crate) search: Vec<PathBuf>,
}

pub(crate) enum Built {
    Ts(TsOptions),
    OpenApi,
    External(BTreeMap<String, Value>),
}

impl Resolved {
    pub(crate) fn strict(&self) -> bool {
        self.mode == EmitMode::Strict
    }

    pub(crate) fn wire_options(&self) -> BTreeMap<String, Value> {
        match &self.built {
            Built::External(options) => options.clone(),
            Built::Ts(options) => ts_words(options),
            Built::OpenApi => BTreeMap::new(),
        }
    }
}

pub(crate) fn resolve(args: &EmitArgs, config: Option<&Config>) -> Result<Resolved, CliError> {
    let target = match config {
        Some(config) => config.target(&args.target),
        None => Target::empty(args.target.clone()),
    };

    let (document_argument, out_dir_argument) = positionals(
        &args.positionals,
        config.and_then(Config::document).is_some(),
    )?;

    let document = match (document_argument, config.and_then(Config::document)) {
        (Some(path), _) => PathBuf::from(path),
        (None, Some(path)) => path.to_path_buf(),
        (None, None) => {
            return Err(CliError::Usage(format!(
                "emit --target {} needs an IR document: pass it as the first argument, or set \
                 {DOCUMENT_TABLE}.{} in pons.toml",
                args.target,
                config::DOCUMENT
            )));
        }
    };
    let out_dir = match (out_dir_argument, target.get(OUT_DIR)) {
        (Some(path), _) => PathBuf::from(path),
        (None, Some(value)) => out_dir_path(&target, value)?,
        (None, None) => {
            return Err(CliError::Usage(format!(
                "emit --target {} needs an output directory: pass it as the second argument, or \
                 set {}.{OUT_DIR} in pons.toml",
                args.target,
                target.name()
            )));
        }
    };
    let mode = match (args.mode, target.get("mode")) {
        (Some(mode), _) => mode,
        (None, Some(value)) => match word(&target, "mode", Some(value), MODE)? {
            Some("lossy") => EmitMode::Lossy,
            _ => EmitMode::Strict,
        },
        (None, None) => EmitMode::Strict,
    };
    let errors = match args.errors {
        Some(errors) => Some(errors),
        None => match word(&target, "errors", target.get("errors"), ERRORS)? {
            Some("throw") => Some(ErrorStrategy::Throw),
            Some(_) => Some(ErrorStrategy::Neverthrow),
            None => None,
        },
    };
    let built = match args.target.as_str() {
        "ts" => Built::Ts(ts_options(&target, errors, mode)?),
        "openapi" => Built::OpenApi,
        _ => Built::External(wire_options(&target)?),
    };

    Ok(Resolved {
        target: args.target.clone(),
        document,
        out_dir,
        mode,
        built,
        search: config
            .map(|config| config.search().to_vec())
            .unwrap_or_default(),
    })
}

fn positionals(
    args: &[OsString],
    configured_document: bool,
) -> Result<(Option<OsString>, Option<OsString>), CliError> {
    match args {
        [] => Ok((None, None)),
        [only] => {
            let path = PathBuf::from(only);
            if path.is_dir() || (configured_document && !path.is_file()) {
                Ok((None, Some(only.clone())))
            } else {
                Ok((Some(only.clone()), None))
            }
        }
        [document, out_dir] => Ok((Some(document.clone()), Some(out_dir.clone()))),
        _ => Err(CliError::Usage(format!(
            "emit takes at most two positional arguments, {} given",
            args.len()
        ))),
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
                if *key != "mode" {
                    word(target, key, target.get(key), allowed)?;
                }
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

fn ts_options(
    target: &Target<'_>,
    errors: Option<ErrorStrategy>,
    mode: EmitMode,
) -> Result<TsOptions, CliError> {
    let mut options = TsOptions {
        mode: match mode {
            EmitMode::Strict => pons_emit_ts::EmitMode::Strict,
            EmitMode::Lossy => pons_emit_ts::EmitMode::Lossy,
        },
        errors: errors.unwrap_or_default(),
        ..TsOptions::default()
    };
    for (key, allowed) in TS {
        if *key == "mode" || *key == "errors" {
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
            ("validation", "builtin") => options.validation = Validation::Builtin,
            ("validation", "zod") => options.validation = Validation::Zod,
            ("validation", _) => options.validation = Validation::None,
            _ => {}
        }
    }
    Ok(options)
}

fn wire_options(target: &Target<'_>) -> Result<BTreeMap<String, Value>, CliError> {
    let mut options = BTreeMap::new();
    for (key, value) in &target.values {
        if key != OUT_DIR && key != "mode" {
            options.insert(key.clone(), to_json(target, key, value)?);
        }
    }
    Ok(options)
}

fn ts_words(options: &TsOptions) -> BTreeMap<String, Value> {
    [
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
        (
            "validation",
            match options.validation {
                Validation::None => "none",
                Validation::Builtin => "builtin",
                Validation::Zod => "zod",
            },
        ),
    ]
    .into_iter()
    .map(|(key, word)| (key.to_owned(), Value::String(word.to_owned())))
    .collect()
}

fn out_dir_path(target: &Target<'_>, value: &toml::Value) -> Result<PathBuf, CliError> {
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
    use std::ffi::OsStr;
    use std::fs;
    use std::path::Path;

    use super::*;

    fn args(target: &str, positionals: &[&str]) -> EmitArgs {
        EmitArgs {
            target: target.into(),
            mode: None,
            errors: None,
            positionals: positionals.iter().map(OsString::from).collect(),
        }
    }

    fn table<'a>(file: &'a Path, name: &str, source: &str) -> Target<'a> {
        Target::with_values(Some(file), name, source.parse::<toml::Table>().unwrap())
    }

    fn the_file() -> &'static Path {
        Path::new("/project/pons.toml")
    }

    #[test]
    fn a_lone_positional_is_the_document_unless_the_file_supplies_one() {
        let directory = tempfile::tempdir().unwrap();
        let document = directory.path().join("api.json");
        let out_dir = directory.path().join("generated");
        fs::create_dir(&out_dir).unwrap();
        fs::write(&document, "{}").unwrap();

        let (document_argument, out_dir_argument) =
            positionals(&[OsString::from("api.json")], false).unwrap();
        assert_eq!(document_argument.as_deref(), Some(OsStr::new("api.json")));
        assert_eq!(out_dir_argument, None);
        let (document_argument, _) = positionals(&[OsString::from(&document)], true).unwrap();
        assert_eq!(
            document_argument.as_deref(),
            Some(document.as_os_str()),
            "an existing file is the document, so the file's own document loses to it"
        );

        let (document_argument, out_dir_argument) =
            positionals(&[OsString::from("generated")], true).unwrap();
        assert_eq!(document_argument, None);
        assert_eq!(
            out_dir_argument.as_deref(),
            Some(OsStr::new("generated")),
            "a name that is not a file is the output directory"
        );

        let (document_argument, out_dir_argument) =
            positionals(&[OsString::from("api.json")], true).unwrap();
        assert_eq!(document_argument, None);
        assert_eq!(
            out_dir_argument.as_deref(),
            Some(OsStr::new("api.json")),
            "a name that is not a file is the output directory, whatever it is called"
        );

        let (document_argument, out_dir_argument) =
            positionals(&[OsString::from(&out_dir)], false).unwrap();
        assert_eq!(document_argument, None);
        assert_eq!(out_dir_argument, Some(out_dir.into_os_string()));

        assert_eq!(positionals(&[], true).unwrap(), (None, None));

        let error = positionals(
            &[
                OsString::from("a"),
                OsString::from("b"),
                OsString::from("c"),
            ],
            false,
        )
        .unwrap_err();
        assert_eq!(
            error.message(),
            "emit takes at most two positional arguments, 3 given"
        );
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
            if matches!(*key, "mode" | "errors") {
                continue;
            }
            for value in *allowed {
                let source = format!("{key} = {value:?}");
                let target = table(the_file(), "ts", &source);
                check_values(&target)
                    .unwrap_or_else(|error| panic!("{source:?}: {}", error.message()));
                let options = ts_options(&target, None, EmitMode::Strict).unwrap();
                assert_eq!(
                    ts_words(&options)[*key],
                    Value::String((*value).into()),
                    "{source:?}"
                );
            }
        }
    }

    #[test]
    fn a_flag_beats_the_file_and_the_file_beats_the_default() {
        let directory = tempfile::tempdir().unwrap();
        let file = directory.path().join(crate::config::FILE_NAME);
        fs::write(&file, "[emit.ts]\nout_dir = \"out\"\nerrors = \"throw\"\n").unwrap();
        let config = crate::config::load(file).ok();

        let errors = |mode, errors| match &resolve(
            &EmitArgs {
                target: "ts".into(),
                mode,
                errors,
                positionals: vec![OsString::from("api.json")],
            },
            config.as_ref(),
        )
        .unwrap()
        .built
        {
            Built::Ts(options) => options.errors,
            Built::OpenApi | Built::External(_) => panic!("the built-in target builds TsOptions"),
        };

        assert_eq!(
            errors(None, None),
            ErrorStrategy::Throw,
            "the file wins over the default"
        );
        assert_eq!(
            errors(None, Some(ErrorStrategy::Neverthrow)),
            ErrorStrategy::Neverthrow,
            "the flag wins over the file"
        );
    }

    #[test]
    fn the_mode_follows_the_same_three_steps() {
        let directory = tempfile::tempdir().unwrap();
        let file = directory.path().join(crate::config::FILE_NAME);
        fs::write(&file, "[emit.ts]\nout_dir = \"out\"\nmode = \"lossy\"\n").unwrap();
        let config = crate::config::load(file).ok();
        let mode = |flag| {
            resolve(
                &EmitArgs {
                    target: "ts".into(),
                    mode: flag,
                    errors: None,
                    positionals: vec![OsString::from("api.json")],
                },
                config.as_ref(),
            )
            .unwrap()
            .mode
        };

        assert_eq!(
            mode(None),
            EmitMode::Lossy,
            "the file wins over the default"
        );
        assert_eq!(
            mode(Some(EmitMode::Strict)),
            EmitMode::Strict,
            "the flag wins over the file"
        );
    }

    #[test]
    fn a_command_that_supplies_nothing_is_told_what_to_supply() {
        let directory = tempfile::tempdir().unwrap();
        let file = directory.path().join(crate::config::FILE_NAME);
        fs::write(&file, "[emit.ts]\nfield_case = \"camel\"\n").unwrap();
        let config = crate::config::load(file).ok();

        let error = resolve(&args("ts", &[]), config.as_ref()).err().unwrap();
        assert_eq!(
            error.message(),
            "emit --target ts needs an IR document: pass it as the first argument, or set \
             project.ir in pons.toml"
        );

        let error = resolve(&args("ts", &["api.json"]), config.as_ref())
            .err()
            .unwrap();
        assert_eq!(
            error.message(),
            "emit --target ts needs an output directory: pass it as the second argument, or set \
             emit.ts.out_dir in pons.toml"
        );
    }

    #[test]
    fn an_out_dir_the_file_supplies_spares_the_command() {
        let directory = tempfile::tempdir().unwrap();
        let file = directory.path().join(crate::config::FILE_NAME);
        fs::write(
            &file,
            "[project]\nir = \"contract/api.json\"\n\n[emit.ts]\nout_dir = \"generated\"\n",
        )
        .unwrap();
        let config = crate::config::load(file).ok();

        let resolved = resolve(&args("ts", &[]), config.as_ref()).unwrap();
        assert_eq!(
            resolved.document,
            directory.path().join("contract/api.json")
        );
        assert_eq!(resolved.out_dir, directory.path().join("generated"));
        assert_eq!(resolved.mode, EmitMode::Strict);
    }
}
