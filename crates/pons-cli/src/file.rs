use std::fs;
use std::io::Write;
use std::path::Path;

use crate::CliError;

pub(crate) fn write_atomic(destination: &Path, content: &str) -> Result<(), CliError> {
    let permissions = match fs::symlink_metadata(destination) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
            return Err(CliError::Failure(format!(
                "{} is not a regular file",
                destination.display()
            )));
        }
        Ok(metadata) => Some(metadata.permissions()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => {
            return Err(CliError::Failure(format!(
                "cannot inspect {}: {error}",
                destination.display()
            )));
        }
    };
    let mut temporary = temporary_beside(destination)?;
    temporary
        .write_all(content.as_bytes())
        .map_err(|error| CliError::Failure(format!("cannot write the file: {error}")))?;
    temporary
        .as_file()
        .sync_all()
        .map_err(|error| CliError::Failure(format!("cannot sync the file to disk: {error}")))?;
    match permissions {
        Some(permissions) => temporary
            .as_file()
            .set_permissions(permissions)
            .map_err(|error| {
                CliError::Failure(format!("cannot preserve file permissions: {error}"))
            })?,
        None => set_default_permissions(temporary.as_file())?,
    }
    replace_file(temporary, destination)
}

pub(crate) fn write_new(destination: &Path, content: &str) -> Result<(), CliError> {
    let mut temporary = temporary_beside(destination)?;
    temporary
        .write_all(content.as_bytes())
        .map_err(|error| CliError::Failure(format!("cannot write the file: {error}")))?;
    temporary
        .as_file()
        .sync_all()
        .map_err(|error| CliError::Failure(format!("cannot sync the file to disk: {error}")))?;
    set_default_permissions(temporary.as_file())?;
    temporary
        .persist_noclobber(destination)
        .map_err(|error| match error.error.kind() {
            std::io::ErrorKind::AlreadyExists => {
                CliError::Failure(format!("{} already exists", destination.display()))
            }
            _ => CliError::Failure(format!(
                "cannot write {}: {}",
                destination.display(),
                error.error
            )),
        })?;
    Ok(())
}

fn temporary_beside(destination: &Path) -> Result<tempfile::NamedTempFile, CliError> {
    let parent = destination
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    tempfile::NamedTempFile::new_in(parent).map_err(|error| {
        CliError::Failure(format!(
            "cannot create a temporary file beside {}: {error}",
            destination.display()
        ))
    })
}

pub(crate) fn replace_file(
    temporary: tempfile::NamedTempFile,
    destination: &Path,
) -> Result<(), CliError> {
    let mut pending = temporary;
    for attempt in 0..5 {
        match pending.persist(destination) {
            Ok(_) => return Ok(()),
            Err(error) => {
                if attempt == 4 || error.error.kind() != std::io::ErrorKind::PermissionDenied {
                    return Err(CliError::Failure(format!(
                        "cannot replace {}: {}",
                        destination.display(),
                        error.error
                    )));
                }

                std::thread::sleep(std::time::Duration::from_millis(50 * (attempt + 1)));
                pending = error.file;
            }
        }
    }
    Ok(())
}

#[cfg(unix)]
pub(crate) fn set_default_permissions(file: &fs::File) -> Result<(), CliError> {
    use std::os::unix::fs::PermissionsExt;

    let mode = probe_default_mode()?;
    file.set_permissions(fs::Permissions::from_mode(mode))
        .map_err(|error| CliError::Failure(format!("cannot set default permissions: {error}")))
}

#[cfg(unix)]
pub(crate) fn probe_default_mode() -> Result<u32, CliError> {
    use std::os::unix::fs::PermissionsExt;
    use std::sync::atomic::{AtomicU32, Ordering};

    static ATTEMPT: AtomicU32 = AtomicU32::new(0);

    for _ in 0..16 {
        let probe = std::env::temp_dir().join(format!(
            ".pons-mode-{}-{}",
            std::process::id(),
            ATTEMPT.fetch_add(1, Ordering::Relaxed)
        ));
        let created = match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&probe)
        {
            Ok(created) => created,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(CliError::Failure(format!(
                    "cannot probe the default file mode at {}: {error}",
                    probe.display()
                )));
            }
        };
        let mode = created.metadata().map_err(|error| {
            CliError::Failure(format!("cannot inspect {}: {error}", probe.display()))
        })?;
        let mode = mode.permissions().mode();
        drop(created);
        let _ = fs::remove_file(&probe);
        return Ok(mode);
    }
    Err(CliError::Failure(
        "cannot probe the default file mode: a stale probe keeps every name".to_owned(),
    ))
}

#[cfg(not(unix))]
pub(crate) fn set_default_permissions(_file: &fs::File) -> Result<(), CliError> {
    Ok(())
}
