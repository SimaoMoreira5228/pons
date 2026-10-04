mod emit;

use std::env;
use std::ffi::OsString;
use std::fs;
use std::io::Write;
use std::path::Path;
use std::process::ExitCode;

use pons_emit_ts::ErrorStrategy;
use pons_ir::{ChangeLevel, Document, ValidationError};

const HELP: &str = "Usage: pons <command> <ir.json> [arguments]\n\nCommands:\n  check <ir.json>                              Validate an IR document\n  fmt <ir.json>                                Rewrite an IR document in canonical form\n  diff <old.json> <new.json>                   Classify changes between two IR documents\n  emit --target <name> <ir.json> <output-dir> [--strict|--lossy] [--errors <neverthrow|throw>] [--coverage <relative-file>]\n                                                Generate files (built-in: ts, openapi)\n      --strict                                 Reject unsupported/lossy coverage; opaque is allowed on ts/openapi, and on a plugin that declares it (default)\n      --lossy                                  Emit non-exact coverage with warnings\n      --errors <neverthrow|throw>              TypeScript target only: neverthrow (default), throw\n      --coverage <relative-file>               Write emitter coverage as JSON\n\nExit codes:\n  0  success, and a diff that found nothing to report\n  1  diff only: the contract changed, but nothing in it is breaking\n  2  diff only: the new contract has a breaking change\n  3  diff only: the new contract is invalid\n  4  the arguments are wrong\n  5  the tool could not do its job: an unreadable or malformed document, a\n     check/fmt validation failure, or an emit failure\n";

fn main() -> ExitCode {
    match run(env::args_os().skip(1).collect()) {
        Ok(()) => ExitCode::SUCCESS,

        Err(CliError::Changed) => ExitCode::from(1),
        Err(CliError::Breaking(message)) => {
            eprintln!("pons: {message}");
            ExitCode::from(2)
        }
        Err(CliError::Invalid(message)) => {
            eprintln!("pons: {message}");
            ExitCode::from(3)
        }
        Err(CliError::Usage(message)) => {
            eprintln!("pons: {message}");
            ExitCode::from(4)
        }
        Err(CliError::Failure(message)) => {
            eprintln!("pons: {message}");
            ExitCode::from(5)
        }
    }
}

fn run(args: Vec<OsString>) -> Result<(), CliError> {
    let Some(command) = args.first().and_then(|arg| arg.to_str()) else {
        return Err(CliError::Usage(
            "no command given; run \"pons --help\" for usage".into(),
        ));
    };
    if matches!(command, "--help" | "-h" | "help") {
        print!("{HELP}");
        return Ok(());
    }
    if command == "diff" {
        if args.len() != 3 {
            return Err(CliError::Usage(format!(
                "diff expects two documents, got {}",
                args.len() - 1
            )));
        }
        let old = read_document(Path::new(&args[1]))?;
        let new = read_document(Path::new(&args[2]))?;
        let diff = old.diff(&new);
        if diff.changes.is_empty() {
            println!("No changes.");
        }
        for change in &diff.changes {
            let level = match change.level {
                ChangeLevel::Info => "INFO",
                ChangeLevel::NonBreaking => "NON_BREAKING",
                ChangeLevel::Warning => "WARNING",
                ChangeLevel::Breaking => "BREAKING",
            };
            println!("{level} {}: {}", change.path, change.detail);
        }
        if !diff.validity.new_is_valid() {
            return Err(CliError::Invalid(render(&diff.validity.new)));
        }
        if diff.has_breaking_changes() {
            return Err(CliError::Breaking("breaking changes found".into()));
        }
        return if diff.changes.is_empty() {
            Ok(())
        } else {
            Err(CliError::Changed)
        };
    }
    if command == "emit" {
        if args.len() < 5 {
            return Err(CliError::Usage(
                "emit expects --target <name> <ir.json> <output-dir> [options]".into(),
            ));
        }
        if args[1] != "--target" {
            return Err(CliError::Usage(format!(
                "emit expects --target as its first option, not {:?}",
                args[1]
            )));
        }
        let target = args[2]
            .to_str()
            .ok_or_else(|| CliError::Usage("--target expects a target name".into()))?;
        let mut mode: Option<bool> = None;
        let mut errors_flag = false;
        let mut error_strategy = ErrorStrategy::Neverthrow;
        let mut coverage_path: Option<OsString> = None;
        let mut index = 5;
        while index < args.len() {
            let Some(flag) = args[index].to_str() else {
                return Err(CliError::Usage(format!(
                    "emit option {index} is not valid UTF-8"
                )));
            };
            match flag {
                "--strict" => {
                    if mode == Some(false) {
                        return Err(CliError::Usage("--strict conflicts with --lossy".into()));
                    }
                    mode = Some(true);
                }
                "--lossy" => {
                    if mode == Some(true) {
                        return Err(CliError::Usage("--lossy conflicts with --strict".into()));
                    }
                    mode = Some(false);
                }
                "--errors" if errors_flag => {
                    return Err(CliError::Usage("--errors given more than once".into()));
                }
                "--errors" => {
                    if target != "ts" {
                        return Err(CliError::Usage(format!(
                            "--errors is only supported by the TypeScript target, not {target}"
                        )));
                    }
                    errors_flag = true;
                    index += 1;
                    error_strategy = match value_for("--errors", args.get(index))? {
                        "neverthrow" => ErrorStrategy::Neverthrow,
                        "throw" => ErrorStrategy::Throw,
                        value => {
                            return Err(CliError::Usage(format!(
                                "--errors expects neverthrow or throw, not {value:?}"
                            )));
                        }
                    };
                }
                "--coverage" if coverage_path.is_some() => {
                    return Err(CliError::Usage("--coverage given more than once".into()));
                }
                "--coverage" => {
                    index += 1;
                    coverage_path = Some(value_for("--coverage", args.get(index))?.into());
                }
                other => return Err(CliError::Usage(format!("unexpected emit option {other:?}"))),
            }
            index += 1;
        }
        let strict = mode.unwrap_or(true);
        let document = read_document(Path::new(&args[3]))?;
        validate(&document)?;
        return emit::run(
            target,
            &document,
            Path::new(&args[4]),
            coverage_path.as_deref().map(Path::new),
            strict,
            error_strategy,
        );
    }
    match command {
        "check" | "fmt" if args.len() == 2 => {
            let path = Path::new(&args[1]);
            let document = read_document(path)?;
            validate(&document)?;
            if command == "check" {
                println!("valid: {}", path.display());
                Ok(())
            } else {
                rewrite_canonical(path, &document)
            }
        }
        "check" | "fmt" => Err(CliError::Usage(format!(
            "{command} expects one document, got {}",
            args.len() - 1
        ))),
        _ => Err(CliError::Usage(format!(
            "unknown command {command:?}; run \"pons --help\" for usage"
        ))),
    }
}

fn value_for<'a>(flag: &str, value: Option<&'a OsString>) -> Result<&'a str, CliError> {
    let value = value.ok_or_else(|| CliError::Usage(format!("{flag} requires a value")))?;
    let value = value
        .to_str()
        .ok_or_else(|| CliError::Usage(format!("{flag} requires a UTF-8 value")))?;
    if value.starts_with('-') {
        return Err(CliError::Usage(format!(
            "{flag} expects a value, not the flag {value:?}"
        )));
    }
    Ok(value)
}

fn read_document(path: &Path) -> Result<Document, CliError> {
    let input = fs::read(path)
        .map_err(|error| CliError::Failure(format!("cannot read {}: {error}", path.display())))?;
    serde_json::from_slice(&input).map_err(|error| {
        CliError::Failure(format!(
            "{} is not a valid IR document: {error}",
            path.display()
        ))
    })
}

fn validate(document: &Document) -> Result<(), CliError> {
    document
        .validate()
        .map_err(|errors| CliError::Failure(render(&errors)))
}

fn render(errors: &[ValidationError]) -> String {
    errors
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("\n")
}

fn rewrite_canonical(path: &Path, document: &Document) -> Result<(), CliError> {
    let metadata = fs::symlink_metadata(path).map_err(|error| {
        CliError::Failure(format!("cannot inspect {}: {error}", path.display()))
    })?;
    if !metadata.is_file() {
        return Err(CliError::Failure(format!(
            "{} is not a regular file",
            path.display()
        )));
    }

    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let mut temporary = tempfile::NamedTempFile::new_in(parent).map_err(|error| {
        CliError::Failure(format!(
            "cannot create a temporary file beside {}: {error}",
            path.display()
        ))
    })?;
    let mut canonical = document
        .to_canonical_json()
        .map_err(|error| CliError::Failure(format!("cannot serialize IR: {error}")))?;
    canonical.push('\n');
    temporary
        .write_all(canonical.as_bytes())
        .map_err(|error| CliError::Failure(format!("cannot write canonical IR: {error}")))?;
    temporary
        .as_file()
        .sync_all()
        .map_err(|error| CliError::Failure(format!("cannot sync canonical IR to disk: {error}")))?;
    temporary
        .as_file()
        .set_permissions(metadata.permissions())
        .map_err(|error| CliError::Failure(format!("cannot preserve file permissions: {error}")))?;
    temporary.persist(path).map_err(|error| {
        CliError::Failure(format!(
            "cannot replace {}: {}",
            path.display(),
            error.error
        ))
    })?;
    Ok(())
}

#[derive(Debug)]
enum CliError {
    Changed,
    Breaking(String),
    Invalid(String),
    Usage(String),
    Failure(String),
}
