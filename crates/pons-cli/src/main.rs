mod emit;

use std::env;
use std::ffi::OsString;
use std::fs;
use std::io::Write;
use std::path::Path;
use std::process::ExitCode;

use pons_ir::{ChangeLevel, Document};

const HELP: &str = "Usage: pons <command> <ir.json> [arguments]\n\nCommands:\n  check <ir.json>                              Validate an IR document\n  fmt <ir.json>                                Rewrite an IR document in canonical form\n  diff <old.json> <new.json>                   Classify changes between two IR documents\n  emit --target <name> <ir.json> <output-dir>  Generate client files with an emitter\n      --strict                                 Fail if the emitter lacks IR capabilities\n";

fn main() -> ExitCode {
    match run(env::args_os().skip(1).collect()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(CliError::Usage(message)) => {
            eprint!("{message}");
            ExitCode::from(2)
        }
        Err(CliError::Failure(message)) => {
            eprintln!("pons: {message}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: Vec<OsString>) -> Result<(), CliError> {
    let Some(command) = args.first().and_then(|arg| arg.to_str()) else {
        return Err(CliError::Usage(HELP.into()));
    };
    if matches!(command, "--help" | "-h" | "help") {
        print!("{HELP}");
        return Ok(());
    }
    if command == "diff" {
        if args.len() != 3 {
            return Err(CliError::Usage(HELP.into()));
        }
        let old = read_document(Path::new(&args[1]))?;
        let new = read_document(Path::new(&args[2]))?;
        validate(&old)?;
        validate(&new)?;
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
        return if diff.has_breaking_changes() {
            Err(CliError::Failure("breaking changes found".into()))
        } else {
            Ok(())
        };
    }
    if command == "emit" {
        if args.len() < 5 || args[1] != "--target" {
            return Err(CliError::Usage(HELP.into()));
        }
        let target = args[2]
            .to_str()
            .ok_or_else(|| CliError::Usage(HELP.into()))?;
        let strict = args.len() == 6 && args[5] == "--strict";
        if args.len() != 5 && !strict {
            return Err(CliError::Usage(HELP.into()));
        }
        let document = read_document(Path::new(&args[3]))?;
        validate(&document)?;
        return emit::run(target, &document, Path::new(&args[4]), strict);
    }
    if args.len() != 2 {
        return Err(CliError::Usage(HELP.into()));
    }

    let path = Path::new(&args[1]);
    match command {
        "check" => {
            let document = read_document(path)?;
            validate(&document)?;
            println!("valid: {}", path.display());
            Ok(())
        }
        "fmt" => {
            let document = read_document(path)?;
            validate(&document)?;
            rewrite_canonical(path, &document)?;
            Ok(())
        }
        _ => Err(CliError::Usage(HELP.into())),
    }
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
    document.validate().map_err(|errors| {
        CliError::Failure(
            errors
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join("\n"),
        )
    })
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

enum CliError {
    Usage(String),
    Failure(String),
}
