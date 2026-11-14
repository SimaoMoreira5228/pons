use pons_emit_ts::{ValidationBackend, ValidationOptions};

use crate::CliError;
use crate::config::{self, Target};

pub(crate) const VALIDATION_BACKENDS: &[&str] = &[
    "none",
    "builtin",
    "zod",
    "valibot",
    "arktype",
    "typebox",
    "yup",
    "superstruct",
];
const VALIDATION_KEYS: &[&str] = &["backend", "import_from"];

pub(crate) fn validation_backend(word: &str) -> Option<ValidationBackend> {
    match word {
        "none" => Some(ValidationBackend::None),
        "builtin" => Some(ValidationBackend::Builtin),
        "zod" => Some(ValidationBackend::Zod),
        "valibot" => Some(ValidationBackend::Valibot),
        "arktype" => Some(ValidationBackend::Arktype),
        "typebox" => Some(ValidationBackend::Typebox),
        "yup" => Some(ValidationBackend::Yup),
        "superstruct" => Some(ValidationBackend::Superstruct),
        _ => None,
    }
}

pub(crate) fn validation_options(
    target: &Target<'_>,
    backend_flag: Option<ValidationBackend>,
) -> Result<ValidationOptions, CliError> {
    let Some(value) = target.get("validation") else {
        return Ok(ValidationOptions::new(backend_flag.unwrap_or_default()));
    };
    let Some(table) = value.as_table() else {
        return Err(target.error(&format!(
            "validation is {}; it is a table, for example [emit.ts.validation] with backend = \"zod\"",
            config::kind(value)
        )));
    };
    for key in table.keys() {
        if !VALIDATION_KEYS.contains(&key.as_str()) {
            return Err(target.error(&format!(
                "validation has no key {key:?}; it is {}",
                VALIDATION_KEYS.join(", ")
            )));
        }
    }
    let file_backend = match table.get("backend") {
        None => ValidationBackend::Builtin,
        Some(value) => backend_word(target, value)?,
    };
    let import_from = match table.get("import_from") {
        None => None,
        Some(value) => Some(
            value
                .as_str()
                .ok_or_else(|| {
                    target.error(&format!(
                        "validation.import_from is {}; it is a module specifier",
                        config::kind(value)
                    ))
                })?
                .to_owned(),
        ),
    };
    let backend = backend_flag.unwrap_or(file_backend);
    let import_from = if backend.has_package() {
        import_from
    } else {
        if backend_flag.is_none() && import_from.is_some() {
            return Err(target.error(&format!(
                "validation.import_from is {}, but the {} backend imports no package",
                import_from.as_deref().unwrap_or_default(),
                backend.name()
            )));
        }
        None
    };
    Ok(ValidationOptions {
        backend,
        import_from,
    })
}

fn backend_word(target: &Target<'_>, value: &toml::Value) -> Result<ValidationBackend, CliError> {
    let Some(word) = value.as_str() else {
        return Err(target.error(&format!(
            "validation.backend is {}; it is {}",
            config::kind(value),
            VALIDATION_BACKENDS.join(" or ")
        )));
    };
    validation_backend(word).ok_or_else(|| {
        target.error(&format!(
            "validation.backend is {word:?}; it is {}",
            VALIDATION_BACKENDS.join(" or ")
        ))
    })
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use pons_plugin::EmitMode;

    use super::*;
    use crate::target_options::{check_values, ts_options, ts_words};

    fn table<'a>(file: &'a Path, name: &str, source: &str) -> Target<'a> {
        Target::with_values(Some(file), name, source.parse::<toml::Table>().unwrap())
    }

    fn the_file() -> &'static Path {
        Path::new("/project/pons.toml")
    }

    #[test]
    fn the_validation_table_reads_back_to_itself() {
        for (source, expected) in [
            ("", serde_json::json!({ "backend": "none" })),
            (
                "[validation]\n",
                serde_json::json!({ "backend": "builtin" }),
            ),
            (
                "[validation]\nbackend = \"zod\"\n",
                serde_json::json!({ "backend": "zod" }),
            ),
            (
                "[validation]\nbackend = \"zod\"\nimport_from = \"zod/v4\"\n",
                serde_json::json!({ "backend": "zod", "import_from": "zod/v4" }),
            ),
            (
                "[validation]\nbackend = \"none\"\n",
                serde_json::json!({ "backend": "none" }),
            ),
            (
                "[validation]\nbackend = \"valibot\"\n",
                serde_json::json!({ "backend": "valibot" }),
            ),
        ] {
            let target = table(the_file(), "ts", &format!("out_dir = \"out\"\n{source}"));
            check_values(&target).unwrap_or_else(|error| panic!("{source:?}: {}", error.message()));
            let options = ts_options(&target, None, EmitMode::Strict, None).unwrap();
            assert_eq!(ts_words(&options)["validation"], expected, "{source:?}");
        }
    }

    #[test]
    fn the_validation_table_refuses_what_it_does_not_understand() {
        for (source, expected) in [
            (
                "validation = \"zod\"\n",
                "validation is a string; it is a table",
            ),
            (
                "[validation]\nbackend = \"pydantic\"\n",
                "validation.backend is \"pydantic\"; it is none or builtin or zod",
            ),
            (
                "[validation]\nformat = \"zod\"\n",
                "validation has no key \"format\"; it is backend, import_from",
            ),
            (
                "[validation]\nbackend = 4\n",
                "validation.backend is an integer",
            ),
            (
                "[validation]\nimport_from = 4\n",
                "validation.import_from is an integer",
            ),
            (
                "[validation]\nimport_from = \"zod/v4\"\n",
                "the builtin backend imports no package",
            ),
            (
                "[validation]\nbackend = \"none\"\nimport_from = \"zod\"\n",
                "the none backend imports no package",
            ),
        ] {
            let target = table(the_file(), "ts", source);
            let error = check_values(&target).unwrap_err();
            assert!(
                error.message().contains(expected),
                "{source:?}: {}",
                error.message()
            );
        }
    }

    #[test]
    fn the_validation_flag_beats_the_file_and_keeps_its_import() {
        let target = table(
            the_file(),
            "ts",
            "out_dir = \"out\"\n[validation]\nbackend = \"builtin\"\n",
        );
        let options = ts_options(
            &target,
            None,
            EmitMode::Strict,
            Some(ValidationBackend::Zod),
        )
        .unwrap();
        assert_eq!(options.validation.backend, ValidationBackend::Zod);
        assert_eq!(options.validation.import_from, None);

        let target = table(
            the_file(),
            "ts",
            "out_dir = \"out\"\n[validation]\nbackend = \"zod\"\nimport_from = \"zod/v4\"\n",
        );
        let options = ts_options(
            &target,
            None,
            EmitMode::Strict,
            Some(ValidationBackend::Builtin),
        )
        .unwrap();
        assert_eq!(options.validation.backend, ValidationBackend::Builtin);
        assert_eq!(
            options.validation.import_from, None,
            "a flag that selects a backend with no package ignores the file's stale import"
        );

        let options = ts_options(
            &target,
            None,
            EmitMode::Strict,
            Some(ValidationBackend::Zod),
        )
        .unwrap();
        assert_eq!(options.validation.import_from.as_deref(), Some("zod/v4"));

        let broken = table(
            the_file(),
            "ts",
            "out_dir = \"out\"\n[validation]\nbackend = \"builtin\"\nimport_from = \"zod\"\n",
        );
        let error = ts_options(&broken, None, EmitMode::Strict, None).unwrap_err();
        assert!(
            error
                .message()
                .contains("the builtin backend imports no package"),
            "{}",
            error.message()
        );
    }
}
