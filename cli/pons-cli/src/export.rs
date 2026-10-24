use std::fs;
use std::path::Path;
use std::process::{Command, Stdio};

use crate::CliError;
use crate::config::{self, Frontend};

pub(crate) fn run() -> Result<(), CliError> {
    let config = config::discover(None)?.ok_or_else(|| {
        CliError::Usage("export needs a pons.toml; run \"pons init\" to write one".into())
    })?;
    let Some(ir) = config.document() else {
        return Err(CliError::Usage(format!(
            "{}: [project] has no ir; it is the path to write the IR to",
            config.file().display()
        )));
    };
    let Some(frontend) = config.frontend() else {
        return Err(CliError::Usage(format!(
            "{}: export needs a [frontend] table",
            config.file().display()
        )));
    };
    let output = frontend_output(frontend, config.directory())?;
    let mut document = pons_ir::decode(&output).map_err(|error| {
        CliError::Failure(format!(
            "the {} frontend produced invalid IR: {error}",
            frontend.name()
        ))
    })?;
    crate::validate(&document)?;
    document.canonicalize();
    let mut canonical = document
        .to_canonical_json()
        .map_err(|error| CliError::Failure(format!("cannot serialize IR: {error}")))?;
    canonical.push('\n');
    if let Some(parent) = ir.parent().filter(|parent| !parent.as_os_str().is_empty()) {
        fs::create_dir_all(parent).map_err(|error| {
            CliError::Failure(format!("cannot create {}: {error}", parent.display()))
        })?;
    }
    crate::file::write_atomic(ir, &canonical)?;
    println!("{}", ir.display());
    Ok(())
}

fn frontend_output(frontend: &Frontend, directory: &Path) -> Result<Vec<u8>, CliError> {
    let output = shell(frontend.command())
        .current_dir(directory)
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .output()
        .map_err(|error| {
            CliError::Failure(format!(
                "cannot run the {} frontend {:?}: {error}",
                frontend.name(),
                frontend.command()
            ))
        })?;
    if !output.status.success() {
        return Err(CliError::Failure(format!(
            "the {} frontend {:?} failed with {}",
            frontend.name(),
            frontend.command(),
            output.status
        )));
    }
    Ok(output.stdout)
}

#[cfg(unix)]
fn shell(command: &str) -> Command {
    let mut process = Command::new("sh");
    process.arg("-c").arg(command);
    process
}

#[cfg(windows)]
fn shell(command: &str) -> Command {
    let mut process = Command::new("cmd");
    process.arg("/C").arg(command);
    process
}
