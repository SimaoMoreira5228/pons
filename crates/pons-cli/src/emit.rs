use std::ffi::OsString;
use std::fs;
use std::io::Write;
use std::path::{Component, Path};

use pons_emit_ts::ValidationBackend;
use pons_ir::{Coverage, Document, ValidationCoverage};
use pons_plugin::{DiagnosticSeverity, EmitRequest, PROTOCOL_VERSION, PluginDiagnostic, log_tail};

use crate::CliError;
use crate::file::{replace_file, set_default_permissions};
use crate::options::{Built, Resolved};

struct OutputFile {
    path: String,
    content: String,
}

struct Emitted {
    files: Vec<OutputFile>,
    coverage: Option<Coverage>,
    validation_coverage: Option<ValidationCoverage>,
    validation_enabled: bool,
    diagnostics: Vec<PluginDiagnostic>,
    warnings: Vec<String>,
    logs: Vec<u8>,
}

pub(super) fn run(
    resolved: &Resolved,
    document: &Document,
    coverage_path: Option<&Path>,
    dump_request: Option<&Path>,
) -> Result<(), CliError> {
    let target = resolved.target.as_str();
    if target.is_empty()
        || !target
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err(CliError::Failure(format!(
            "invalid emitter target {target:?}"
        )));
    }
    if let Some(path) = dump_request {
        return dump(path, resolved, document);
    }

    let Emitted {
        mut files,
        coverage,
        validation_coverage,
        validation_enabled,
        diagnostics,
        warnings,
        logs,
    } = match &resolved.built {
        Built::Ts(options) => {
            let validation_enabled = options.validation.backend != ValidationBackend::None;
            let output = pons_emit_ts::emit_with_options(document, options.clone())
                .map_err(|error| CliError::Failure(error.to_string()))?;
            Emitted {
                files: output
                    .files
                    .into_iter()
                    .map(|file| OutputFile {
                        path: file.path,
                        content: file.content,
                    })
                    .collect(),
                coverage: Some(output.coverage),
                validation_coverage: Some(output.validation_coverage),
                validation_enabled,
                diagnostics: Vec::new(),
                warnings: output.warnings,
                logs: Vec::new(),
            }
        }
        Built::OpenApi => {
            let output = pons_emit_openapi::emit_with_mode(
                document,
                if resolved.strict() {
                    pons_emit_openapi::EmitMode::Strict
                } else {
                    pons_emit_openapi::EmitMode::Lossy
                },
            )
            .map_err(|error| CliError::Failure(error.to_string()))?;
            Emitted {
                files: vec![OutputFile {
                    path: "openapi.json".into(),
                    content: output.content,
                }],
                coverage: Some(output.coverage),
                validation_coverage: None,
                validation_enabled: false,
                diagnostics: Vec::new(),
                warnings: output.warnings,
                logs: Vec::new(),
            }
        }
        Built::External(options) => {
            let mut executable = OsString::from("pons-emit-");
            executable.push(target);
            let validation_enabled = pons_plugin::validation_enabled(options);
            let result = pons_plugin::invoke(
                &executable,
                target,
                document,
                options.clone(),
                resolved.strict(),
                &resolved.search,
                resolved.pin.as_ref(),
            )
            .map_err(|error| CliError::Failure(error.to_string()))?;
            let coverage = result.response.coverage.clone();
            let validation_coverage = result.response.validation_coverage.clone();
            let files = result
                .response
                .files
                .into_iter()
                .map(|file| OutputFile {
                    path: file.path,
                    content: file.content,
                })
                .collect();
            let mut warnings = result.capability_warnings;
            if !resolved.strict() {
                warnings.extend(coverage_warnings(target, coverage.as_ref()));
            }
            Emitted {
                files,
                coverage,
                validation_coverage,
                validation_enabled,
                diagnostics: result.response.diagnostics,
                warnings,
                logs: result.logs,
            }
        }
    };

    eprint!("{}", log_tail(&String::from_utf8_lossy(&logs)));
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
        if !safe_relative_path(relative) {
            return Err(CliError::Failure(format!(
                "coverage path is not a safe relative path: {relative}"
            )));
        }
        if files.iter().any(|file| file.path == relative) {
            return Err(CliError::Failure(format!(
                "coverage output conflicts with emitter output {relative}"
            )));
        }
        let validation_coverage = match validation_coverage {
            Some(coverage) => coverage,
            None if validation_enabled => {
                return Err(CliError::Failure(format!(
                    "{target} returned no validation coverage while validation is enabled"
                )));
            }
            None => ValidationCoverage::default(),
        };
        let report = serde_json::json!({
            "coverage": coverage,
            "validation_coverage": validation_coverage,
        });
        let mut content = serde_json::to_string_pretty(&report)
            .map_err(|error| CliError::Failure(format!("cannot serialize coverage: {error}")))?;
        content.push('\n');
        files.push(OutputFile {
            path: relative.to_owned(),
            content,
        });
    }
    write_files(&resolved.out_dir, files)
}

fn dump(path: &Path, resolved: &Resolved, document: &Document) -> Result<(), CliError> {
    let relative = path
        .to_str()
        .ok_or_else(|| CliError::Failure("dump-request path is not valid UTF-8".into()))?;
    if !safe_relative_path(relative) {
        return Err(CliError::Failure(format!(
            "dump-request path is not a safe relative path: {relative}"
        )));
    }
    let mut canonical = document.clone();
    canonical.canonicalize();
    canonical.validate().map_err(|errors| {
        CliError::Failure(
            errors
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join("\n"),
        )
    })?;
    let request = EmitRequest {
        protocol: PROTOCOL_VERSION,
        source_ir_version: canonical.ir_version.clone(),
        mode: resolved.mode,
        ir: &canonical,
        options: resolved.wire_options(),
        target: &resolved.target,
    };
    let mut content = serde_json::to_string_pretty(&request).map_err(|error| {
        CliError::Failure(format!("cannot serialize the emit request: {error}"))
    })?;
    content.push('\n');
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(parent).map_err(|error| {
        CliError::Failure(format!("cannot create {}: {error}", parent.display()))
    })?;
    fs::write(path, content)
        .map_err(|error| CliError::Failure(format!("cannot write {}: {error}", path.display())))
}

fn coverage_warnings(target: &str, coverage: Option<&pons_plugin::Coverage>) -> Vec<String> {
    let Some(coverage) = coverage else {
        return Vec::new();
    };
    [
        ("opaque", &coverage.opaque),
        ("lossy", &coverage.lossy),
        ("unsupported", &coverage.unsupported),
    ]
    .into_iter()
    .filter(|(_, subjects)| !subjects.is_empty())
    .map(|(level, subjects)| {
        format!(
            "{target} emits {level} coverage at {}",
            subjects.iter().cloned().collect::<Vec<_>>().join(", ")
        )
    })
    .collect()
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
        } else {
            set_default_permissions(temporary.as_file())?;
        }
        replace_file(temporary, &parent)?;
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

fn safe_relative_path(path: &str) -> bool {
    !path.is_empty()
        && !path.contains('\\')
        && !path.contains(':')
        && !Path::new(path).is_absolute()
        && Path::new(path)
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
        && path
            .split('/')
            .all(|part| !part.is_empty() && part != "." && part != ".." && !reserved_device(part))
}

fn reserved_device(segment: &str) -> bool {
    let name = segment
        .split_once('.')
        .map_or(segment, |(stem, _)| stem)
        .to_ascii_uppercase();
    matches!(name.as_str(), "NUL" | "CON" | "PRN" | "AUX")
        || ["COM", "LPT"].iter().any(|prefix| {
            name.strip_prefix(prefix)
                .is_some_and(|n| n.len() == 1 && matches!(n.as_bytes()[0], b'1'..=b'9'))
        })
}

fn validate_destination(root: &Path, relative: &str) -> Result<(), CliError> {
    if !safe_relative_path(relative) {
        return Err(CliError::Failure(format!(
            "unsafe emitter output path {relative:?}"
        )));
    }
    let mut destination = root.to_path_buf();
    let mut components = Path::new(relative).components().peekable();
    while let Some(component) = components.next() {
        let Component::Normal(name) = component else {
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
    use crate::file::probe_default_mode;
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

    #[test]
    fn refuses_output_paths_the_plugin_protocol_also_refuses() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("output");

        for path in [
            "NUL",
            "nul.ts",
            "a:b.ts",
            "sub/CON.ts",
            "nested/aux",
            "com1.ts",
            "client.ts\\escape",
            "",
        ] {
            let result = write_files(
                &root,
                vec![OutputFile {
                    path: path.into(),
                    content: "generated".into(),
                }],
            );
            assert!(result.is_err(), "{path:?} must be refused");
            assert!(
                path.is_empty() || !root.join(path).exists(),
                "{path:?} was written"
            );
        }
        assert_eq!(fs::read_dir(&root).unwrap().count(), 0, "nothing written");

        assert!(
            write_files(
                &root,
                vec![OutputFile {
                    path: "nested/client.ts".into(),
                    content: "generated".into(),
                }],
            )
            .is_ok()
        );
    }

    #[test]
    fn new_files_take_the_default_mode_and_existing_files_keep_theirs() {
        use std::os::unix::fs::PermissionsExt;

        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("output");
        write_files(
            &root,
            vec![OutputFile {
                path: "kept.ts".into(),
                content: "old".into(),
            }],
        )
        .unwrap();
        fs::set_permissions(root.join("kept.ts"), fs::Permissions::from_mode(0o640)).unwrap();

        write_files(
            &root,
            vec![
                OutputFile {
                    path: "kept.ts".into(),
                    content: "new".into(),
                },
                OutputFile {
                    path: "fresh.ts".into(),
                    content: "new".into(),
                },
            ],
        )
        .unwrap();

        let mode = |name: &str| fs::metadata(root.join(name)).unwrap().permissions().mode() & 0o777;
        let default = probe_default_mode().unwrap() & 0o777;

        assert_eq!(mode("fresh.ts"), default, "new files skip tempfile's 0600");
        assert_eq!(mode("kept.ts"), 0o640, "existing mode is restored");
    }

    #[test]
    fn emit_leaves_existing_mode_probes_in_the_output_alone() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("output");
        fs::create_dir(&root).unwrap();
        let probe = root.join(format!(".pons-mode-{}", std::process::id()));
        fs::write(&probe, "not a probe").unwrap();

        write_files(
            &root,
            vec![OutputFile {
                path: "client.ts".into(),
                content: "new".into(),
            }],
        )
        .unwrap();

        assert_eq!(fs::read_to_string(&probe).unwrap(), "not a probe");
    }

    #[test]
    fn a_stale_mode_probe_does_not_stop_an_emit() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("output");
        fs::create_dir(&root).unwrap();
        for attempt in 0..8 {
            let probe =
                std::env::temp_dir().join(format!(".pons-mode-{}-{attempt}", std::process::id()));
            fs::write(&probe, "left by a process that died").unwrap();
        }

        write_files(
            &root,
            vec![OutputFile {
                path: "client.ts".into(),
                content: "new".into(),
            }],
        )
        .unwrap();

        assert_eq!(fs::read_to_string(root.join("client.ts")).unwrap(), "new");
        for attempt in 0..8 {
            let probe =
                std::env::temp_dir().join(format!(".pons-mode-{}-{attempt}", std::process::id()));
            let _ = fs::remove_file(probe);
        }
    }
}
