use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fs;
use std::io::Write;
use std::path::Path;

use pons_emit_ts::ErrorStrategy;
use pons_ir::Document;
use pons_plugin::{DiagnosticSeverity, PluginDiagnostic};

use crate::CliError;

struct OutputFile {
    path: String,
    content: String,
}

pub(super) fn run(
    target: &str,
    document: &Document,
    output: &Path,
    coverage_path: Option<&Path>,
    strict: bool,
    error_strategy: ErrorStrategy,
) -> Result<(), CliError> {
    if target.is_empty()
        || !target
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err(CliError::Failure(format!(
            "invalid emitter target {target:?}"
        )));
    }

    let (mut files, coverage, diagnostics, warnings, logs) = if target == "ts" {
        let output = pons_emit_ts::emit_with_strategy(
            document,
            if strict {
                pons_emit_ts::EmitMode::Strict
            } else {
                pons_emit_ts::EmitMode::Lossy
            },
            error_strategy,
        )
        .map_err(|error| CliError::Failure(error.to_string()))?;
        let files = output
            .files
            .into_iter()
            .map(|file| OutputFile {
                path: file.path,
                content: file.content,
            })
            .collect();
        (
            files,
            Some(output.coverage),
            Vec::new(),
            output.warnings,
            Vec::new(),
        )
    } else if target == "openapi" {
        let output = pons_emit_openapi::emit_with_mode(
            document,
            if strict {
                pons_emit_openapi::EmitMode::Strict
            } else {
                pons_emit_openapi::EmitMode::Lossy
            },
        )
        .map_err(|error| CliError::Failure(error.to_string()))?;
        (
            vec![OutputFile {
                path: "openapi.json".into(),
                content: output.content,
            }],
            Some(output.coverage),
            Vec::new(),
            output.warnings,
            Vec::new(),
        )
    } else {
        let mut executable = OsString::from("pons-emit-");
        executable.push(target);
        let result = pons_plugin::invoke(&executable, target, document, BTreeMap::new(), strict)
            .map_err(|error| CliError::Failure(error.to_string()))?;
        let coverage = result.response.coverage.clone();
        let files = result
            .response
            .files
            .into_iter()
            .map(|file| OutputFile {
                path: file.path,
                content: file.content,
            })
            .collect();
        (
            files,
            coverage,
            result.response.diagnostics,
            result.capability_warnings,
            result.logs,
        )
    };

    eprint!("{}", String::from_utf8_lossy(&logs));
    for warning in warnings {
        eprintln!("warning: {warning}");
    }
    for diagnostic in &diagnostics {
        print_diagnostic(diagnostic);
    }
    if diagnostics
        .iter()
        .any(|diagnostic| diagnostic.severity == DiagnosticSeverity::Error)
    {
        return Err(CliError::Failure("emitter reported errors".into()));
    }
    if let Some(path) = coverage_path {
        let coverage = coverage.ok_or_else(|| {
            CliError::Failure("emitter did not return coverage for --coverage".into())
        })?;
        let relative = path
            .to_str()
            .ok_or_else(|| CliError::Failure("coverage path is not valid UTF-8".into()))?;
        if relative.is_empty()
            || relative.contains('\\')
            || relative
                .split('/')
                .any(|part| part.is_empty() || part == "." || part == "..")
        {
            return Err(CliError::Failure(format!(
                "coverage path is not a safe relative path: {relative}"
            )));
        }
        if files.iter().any(|file| file.path == relative) {
            return Err(CliError::Failure(format!(
                "coverage output conflicts with emitter output {relative}"
            )));
        }
        let mut content = serde_json::to_string_pretty(&coverage)
            .map_err(|error| CliError::Failure(format!("cannot serialize coverage: {error}")))?;
        content.push('\n');
        files.push(OutputFile {
            path: relative.to_owned(),
            content,
        });
    }
    write_files(output, files)
}

fn print_diagnostic(diagnostic: &PluginDiagnostic) {
    let severity = match diagnostic.severity {
        DiagnosticSeverity::Info => "info",
        DiagnosticSeverity::Warning => "warning",
        DiagnosticSeverity::Error => "error",
    };
    let origin = diagnostic
        .origin
        .as_deref()
        .map(|origin| format!(" ({origin})"))
        .unwrap_or_default();
    eprintln!(
        "{severity}[{}]{origin}: {}",
        diagnostic.code, diagnostic.message
    );
}

fn write_files(output: &Path, files: Vec<OutputFile>) -> Result<(), CliError> {
    validate_file_set(output, &files)?;
    fs::create_dir_all(output).map_err(|error| {
        CliError::Failure(format!("cannot create {}: {error}", output.display()))
    })?;
    let metadata = fs::symlink_metadata(output).map_err(|error| {
        CliError::Failure(format!("cannot inspect {}: {error}", output.display()))
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(CliError::Failure(format!(
            "{} is not a directory",
            output.display()
        )));
    }
    let root = fs::canonicalize(output).map_err(|error| {
        CliError::Failure(format!("cannot resolve {}: {error}", output.display()))
    })?;
    for file in &files {
        validate_destination(&root, &file.path)?;
    }
    for file in files {
        write_file(&root, &file)?;
    }
    Ok(())
}

fn write_file(root: &Path, file: &OutputFile) -> Result<(), CliError> {
    let relative = Path::new(&file.path);
    let mut parent = root.to_path_buf();
    let mut components = relative.components().peekable();
    while let Some(component) = components.next() {
        let std::path::Component::Normal(name) = component else {
            return Err(CliError::Failure(format!(
                "unsafe emitter output path {:?}",
                file.path
            )));
        };
        parent.push(name);
        if components.peek().is_some() {
            ensure_directory(&parent, root)?;
            continue;
        }
        let existing = fs::symlink_metadata(&parent);
        let permissions = match existing {
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
                return Err(CliError::Failure(format!(
                    "emitter output {} is not a regular file",
                    parent.display()
                )));
            }
            Ok(metadata) => Some(metadata.permissions()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => {
                return Err(CliError::Failure(format!(
                    "cannot inspect {}: {error}",
                    parent.display()
                )));
            }
        };
        let directory = parent.parent().unwrap_or(root);
        ensure_within_root(directory, root)?;
        let mut temporary = tempfile::NamedTempFile::new_in(directory).map_err(|error| {
            CliError::Failure(format!(
                "cannot create output beside {}: {error}",
                parent.display()
            ))
        })?;
        temporary
            .write_all(file.content.as_bytes())
            .map_err(|error| {
                CliError::Failure(format!("cannot write {}: {error}", parent.display()))
            })?;
        temporary.as_file().sync_all().map_err(|error| {
            CliError::Failure(format!("cannot sync {}: {error}", parent.display()))
        })?;
        if let Some(permissions) = permissions {
            temporary
                .as_file()
                .set_permissions(permissions)
                .map_err(|error| {
                    CliError::Failure(format!("cannot preserve permissions: {error}"))
                })?;
        }
        temporary.persist(&parent).map_err(|error| {
            CliError::Failure(format!(
                "cannot replace {}: {}",
                parent.display(),
                error.error
            ))
        })?;
    }
    Ok(())
}

fn validate_file_set(output: &Path, files: &[OutputFile]) -> Result<(), CliError> {
    let mut case_conflict = None;
    for (index, file) in files.iter().enumerate() {
        for other in &files[index + 1..] {
            if paths_conflict(&file.path, &other.path, false) {
                return Err(CliError::Failure(format!(
                    "emitter output path conflict between {} and {}",
                    file.path, other.path
                )));
            }
            if case_conflict.is_none() && paths_conflict(&file.path, &other.path, true) {
                case_conflict = Some((&file.path, &other.path));
            }
        }
    }
    if let Some((left, right)) = case_conflict
        && destination_is_case_insensitive(output)?
    {
        return Err(CliError::Failure(format!(
            "emitter output path conflict between {left} and {right}"
        )));
    }
    Ok(())
}

fn paths_conflict(left: &str, right: &str, fold_case: bool) -> bool {
    if fold_case {
        let left = left.to_lowercase();
        let right = right.to_lowercase();
        left == right
            || left
                .strip_prefix(&right)
                .is_some_and(|suffix| suffix.starts_with('/'))
            || right
                .strip_prefix(&left)
                .is_some_and(|suffix| suffix.starts_with('/'))
    } else {
        left == right
            || left
                .strip_prefix(right)
                .is_some_and(|suffix| suffix.starts_with('/'))
            || right
                .strip_prefix(left)
                .is_some_and(|suffix| suffix.starts_with('/'))
    }
}

fn destination_is_case_insensitive(output: &Path) -> Result<bool, CliError> {
    let parent = output
        .ancestors()
        .find(|path| {
            fs::symlink_metadata(path)
                .is_ok_and(|metadata| metadata.is_dir() && !metadata.file_type().is_symlink())
        })
        .map(Path::to_path_buf)
        .or_else(|| std::env::current_dir().ok())
        .ok_or_else(|| {
            CliError::Failure(format!(
                "cannot find a directory for output probe: {}",
                output.display()
            ))
        })?;
    let probe = tempfile::tempdir_in(parent).map_err(|error| {
        CliError::Failure(format!(
            "cannot probe output filesystem {}: {error}",
            output.display()
        ))
    })?;
    let upper = probe.path().join("PonsCaseProbe");
    let lower = probe.path().join("ponscaseprobe");
    fs::create_dir(&upper).map_err(|error| {
        CliError::Failure(format!(
            "cannot probe output filesystem {}: {error}",
            output.display()
        ))
    })?;
    match fs::create_dir(&lower) {
        Ok(()) => Ok(false),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => Ok(true),
        Err(error) => Err(CliError::Failure(format!(
            "cannot probe output filesystem {}: {error}",
            output.display()
        ))),
    }
}

fn validate_destination(root: &Path, relative: &str) -> Result<(), CliError> {
    let mut destination = root.to_path_buf();
    let mut components = Path::new(relative).components().peekable();
    while let Some(component) = components.next() {
        let std::path::Component::Normal(name) = component else {
            return Err(CliError::Failure(format!(
                "unsafe emitter output path {relative:?}"
            )));
        };
        destination.push(name);
        if components.peek().is_some() {
            ensure_directory(&destination, root)?;
        } else {
            match fs::symlink_metadata(&destination) {
                Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
                    return Err(CliError::Failure(format!(
                        "emitter output {} is not a regular file",
                        destination.display()
                    )));
                }
                Ok(_) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => {
                    return Err(CliError::Failure(format!(
                        "cannot inspect {}: {error}",
                        destination.display()
                    )));
                }
            }
        }
    }
    Ok(())
}

fn ensure_directory(path: &Path, root: &Path) -> Result<(), CliError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            return Err(CliError::Failure(format!(
                "emitter output directory {} is not a real directory",
                path.display()
            )));
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            fs::create_dir(path).map_err(|error| {
                CliError::Failure(format!("cannot create {}: {error}", path.display()))
            })?;
        }
        Err(error) => {
            return Err(CliError::Failure(format!(
                "cannot inspect {}: {error}",
                path.display()
            )));
        }
    }
    ensure_within_root(path, root)
}

fn ensure_within_root(path: &Path, root: &Path) -> Result<(), CliError> {
    let resolved = fs::canonicalize(path).map_err(|error| {
        CliError::Failure(format!("cannot resolve {}: {error}", path.display()))
    })?;
    if !resolved.starts_with(root) {
        return Err(CliError::Failure(format!(
            "emitter output escaped {}",
            root.display()
        )));
    }
    Ok(())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    #[test]
    fn refuses_output_through_symlinked_directories() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("output");
        let outside = directory.path().join("outside");
        fs::create_dir(&root).unwrap();
        fs::create_dir(&outside).unwrap();
        symlink(&outside, root.join("linked")).unwrap();

        let result = write_files(
            &root,
            vec![OutputFile {
                path: "linked/client.ts".into(),
                content: "bad".into(),
            }],
        );
        assert!(result.is_err());
        assert!(!outside.join("client.ts").exists());
    }

    #[test]
    fn validates_all_paths_before_writing_any_file() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("output");
        let outside = directory.path().join("outside");
        fs::create_dir(&root).unwrap();
        fs::create_dir(&outside).unwrap();
        symlink(&outside, root.join("linked")).unwrap();

        let result = write_files(
            &root,
            vec![
                OutputFile {
                    path: "first.ts".into(),
                    content: "first".into(),
                },
                OutputFile {
                    path: "linked/second.ts".into(),
                    content: "bad".into(),
                },
            ],
        );
        assert!(result.is_err());
        assert!(!root.join("first.ts").exists());
    }

    #[test]
    fn rejects_file_directory_conflicts_before_creating_output() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("output");
        let result = write_files(
            &root,
            vec![
                OutputFile {
                    path: "openapi.json".into(),
                    content: "document".into(),
                },
                OutputFile {
                    path: "openapi.json/coverage.json".into(),
                    content: "coverage".into(),
                },
            ],
        );
        assert!(result.is_err());
        assert!(!root.exists());
    }

    #[test]
    fn case_distinct_files_follow_destination_case_behavior() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("output");
        let files = vec![
            OutputFile {
                path: "Client.ts".into(),
                content: "upper".into(),
            },
            OutputFile {
                path: "client.ts".into(),
                content: "lower".into(),
            },
        ];
        let case_insensitive = match destination_is_case_insensitive(&root) {
            Ok(value) => value,
            Err(_) => panic!("could not determine output filesystem behavior"),
        };
        if case_insensitive {
            assert!(write_files(&root, files).is_err());
        } else {
            assert!(write_files(&root, files).is_ok());
            assert_eq!(fs::read_to_string(root.join("Client.ts")).unwrap(), "upper");
            assert_eq!(fs::read_to_string(root.join("client.ts")).unwrap(), "lower");
        }
    }

    #[test]
    fn path_conflicts_fold_case_only_when_needed() {
        assert!(!paths_conflict("Client.ts", "client.ts", false));
        assert!(paths_conflict("Client.ts", "client.ts", true));
        assert!(paths_conflict("Nested", "nested/file.ts", true));
    }
}
