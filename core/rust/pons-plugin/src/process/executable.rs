use std::ffi::{OsStr, OsString};
use std::path::PathBuf;

use crate::pin::PluginPin;

use super::PluginError;

#[cfg_attr(not(unix), allow(unused_variables))]
#[derive(Debug)]
pub(super) struct Executable {
    pub(super) program: OsString,
    prefix: Vec<OsString>,
}

impl Executable {
    pub(super) fn command(&self, name: &str) -> Vec<OsString> {
        let mut arguments = self.prefix.clone();
        arguments.push(name.into());
        arguments
    }
}

const EXECUTABLE_SUFFIXES: &[&str] = if cfg!(windows) {
    &["", ".exe", ".cmd", ".bat"]
} else {
    &[""]
};

pub(super) fn resolve_executable(
    executable: &OsStr,
    search: &[PathBuf],
) -> Result<Executable, PluginError> {
    let name = executable.to_string_lossy();
    let found = if name.contains(std::path::MAIN_SEPARATOR) {
        None
    } else {
        find_in_directories(&name, search).ok().or_else(|| {
            std::env::var_os("PATH")
                .and_then(|path| find_in_path(&name, EXECUTABLE_SUFFIXES, &path).ok())
        })
    };
    match found {
        Some((path, suffix)) => start_resolved(path, suffix),
        None => Ok(Executable {
            program: executable.to_owned(),
            prefix: Vec::new(),
        }),
    }
}

pub(super) fn resolve_pinned(pin: &PluginPin) -> Result<Executable, PluginError> {
    pin.verify().map_err(PluginError)?;
    start_resolved(pin.path().to_path_buf(), pin_suffix(pin.path()))
}

fn pin_suffix(path: &std::path::Path) -> &'static str {
    match path.extension().and_then(OsStr::to_str) {
        Some("cmd") => ".cmd",
        Some("bat") => ".bat",
        _ => "",
    }
}

fn find_in_directories(
    name: &str,
    directories: &[PathBuf],
) -> std::io::Result<(PathBuf, &'static str)> {
    let search = std::env::join_paths(directories).map_err(std::io::Error::other)?;
    find_in_path(name, EXECUTABLE_SUFFIXES, &search)
}

fn start_resolved(
    path: std::path::PathBuf,
    suffix: &'static str,
) -> Result<Executable, PluginError> {
    if suffix.is_empty() {
        return Ok(Executable {
            program: path.into_os_string(),
            prefix: Vec::new(),
        });
    }
    if !cfg!(windows) {
        return Err(PluginError(format!(
            "cannot run {}: a {suffix} emitter needs a windows host",
            path.display()
        )));
    }
    Ok(Executable {
        program: OsString::from("cmd"),
        prefix: vec![OsString::from("/c"), path.into_os_string()],
    })
}

fn find_in_path(
    name: &str,
    suffixes: &[&'static str],
    search: &OsStr,
) -> std::io::Result<(std::path::PathBuf, &'static str)> {
    for directory in std::env::split_paths(search) {
        let trimmed = directory.to_string_lossy();
        let trimmed = trimmed
            .strip_prefix('"')
            .and_then(|trimmed| trimmed.strip_suffix('"'))
            .unwrap_or(&trimmed);
        for suffix in suffixes {
            let candidate = std::path::Path::new(trimmed).join(format!("{name}{suffix}"));
            if candidate.is_file() {
                return Ok((candidate, suffix));
            }
        }
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::NotFound,
        format!("{name} is not on the search path"),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_emitter_is_resolved_by_the_names_a_host_can_start() {
        let directory = tempfile::tempdir().unwrap();
        let suffixes = ["", ".exe", ".cmd", ".bat"];
        for suffix in suffixes {
            std::fs::write(
                directory.path().join(format!("pons-emit-py{suffix}")),
                b"#!/bin/sh\n",
            )
            .unwrap();
        }
        let search = std::env::join_paths([directory.path()]).unwrap();
        let (path, suffix) = find_in_path("pons-emit-py", &suffixes, &search).unwrap();
        assert_eq!(path, directory.path().join("pons-emit-py"));
        assert_eq!(suffix, "");

        let (path, suffix) = find_in_path("pons-emit-py", &suffixes[1..], &search).unwrap();
        assert_eq!(path, directory.path().join("pons-emit-py.exe"));
        assert_eq!(suffix, ".exe");

        let (path, suffix) = find_in_path("pons-emit-py", &suffixes[2..], &search).unwrap();
        assert_eq!(path, directory.path().join("pons-emit-py.cmd"));
        assert_eq!(suffix, ".cmd");

        let (path, suffix) = find_in_path("pons-emit-py", &suffixes[3..], &search).unwrap();
        assert_eq!(path, directory.path().join("pons-emit-py.bat"));
        assert_eq!(suffix, ".bat");

        assert!(
            find_in_path("pons-emit-absent", &suffixes, &search)
                .unwrap_err()
                .to_string()
                .contains("pons-emit-absent")
        );
    }

    #[test]
    fn a_quoted_search_entry_is_still_searched() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(directory.path().join("pons-emit-py"), b"#!/bin/sh\n").unwrap();
        let search = std::env::join_paths([std::path::PathBuf::from(format!(
            "\"{}\"",
            directory.path().display()
        ))])
        .unwrap();
        let (path, _) = find_in_path("pons-emit-py", &[""], &search).unwrap();
        assert_eq!(path, directory.path().join("pons-emit-py"));
    }

    #[test]
    fn a_windows_script_is_only_started_on_a_windows_host() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("pons-emit-py.cmd");
        std::fs::write(&path, b"@echo off\n").unwrap();
        if cfg!(windows) {
            let resolved = start_resolved(path.clone(), ".cmd").unwrap();
            assert_eq!(resolved.program, OsString::from("cmd"));
            assert_eq!(resolved.command("emit")[0], OsString::from("/c"));
            assert_eq!(resolved.command("emit")[1], path.into_os_string());
            assert_eq!(resolved.command("emit")[2], OsString::from("emit"));
        } else {
            let error = start_resolved(path, ".cmd").unwrap_err();
            assert!(error.0.contains("needs a windows host"), "{}", error.0);
        }
    }

    #[test]
    fn a_configured_search_directory_is_consulted_before_the_search_path() {
        let directory = tempfile::tempdir().unwrap();
        let configured = directory.path().join("tools");
        let empty = directory.path().join("empty");
        std::fs::create_dir_all(&configured).unwrap();
        std::fs::create_dir_all(&empty).unwrap();
        std::fs::write(configured.join("pons-emit-absent"), b"#!/bin/sh\n").unwrap();

        let name = OsStr::new("pons-emit-absent");
        assert!(
            find_in_directories("pons-emit-absent", std::slice::from_ref(&empty)).is_err(),
            "a directory with no emitter in it does not answer"
        );
        assert_eq!(
            find_in_directories("pons-emit-absent", &[empty.clone(), configured.clone()])
                .unwrap()
                .0,
            configured.join("pons-emit-absent")
        );
        assert_eq!(
            resolve_executable(name, std::slice::from_ref(&configured))
                .unwrap()
                .program,
            configured.join("pons-emit-absent").into_os_string(),
            "the configured directory answers, because no such emitter is on the search path"
        );
        assert_eq!(
            resolve_executable(name, &[]).unwrap().program,
            name.to_owned(),
            "with nothing configured the name is passed to the operating system as it is"
        );
    }

    #[test]
    fn a_name_given_as_a_path_is_run_where_it_sits() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("pons-emit-py");
        std::fs::write(&path, b"#!/bin/sh\n").unwrap();
        let resolved = resolve_executable(OsStr::new(&path), &[]).unwrap();
        assert_eq!(resolved.program, path);
        assert!(resolved.prefix.is_empty());
        assert_eq!(resolved.command("emit"), vec![OsString::from("emit")]);
    }
}
