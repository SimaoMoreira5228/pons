use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fs;
use std::io::Write;
use std::path::Path;

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
    strict: bool,
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

    let (files, diagnostics, warnings, logs) = if target == "ts" {
        let output = pons_emit_ts::emit_with_mode(
            document,
            if strict {
                pons_emit_ts::EmitMode::Strict
            } else {
                pons_emit_ts::EmitMode::Lossy
            },
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
        (files, Vec::new(), output.warnings, Vec::new())
    } else if target == "openapi" {
        let content = pons_emit_openapi::emit(document)
            .map_err(|error| CliError::Failure(error.to_string()))?;
        (
            vec![OutputFile {
                path: "openapi.json".into(),
                content,
            }],
            Vec::new(),
            Vec::new(),
            Vec::new(),
        )
    } else {
        let mut executable = OsString::from("pons-emit-");
        executable.push(target);
        let result = pons_plugin::invoke(&executable, target, document, BTreeMap::new(), strict)
            .map_err(|error| CliError::Failure(error.to_string()))?;
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
}
