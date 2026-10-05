use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::io::{Read, Write};
#[cfg(unix)]
use std::os::unix::process::CommandExt;
use std::process::{Command, Output, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use pons_ir::Document;
use serde_json::Value;

use crate::protocol::{
    EmitMode, EmitRequest, EmitResponse, EmitterManifest, PROTOCOL_VERSION,
    validate_response_for_document,
};

#[derive(Debug)]
pub struct PluginError(pub String);

const PLUGIN_TIMEOUT: Duration = Duration::from_secs(30);
const PLUGIN_POLL_INTERVAL: Duration = Duration::from_millis(10);
const PLUGIN_DRAIN_GRACE: Duration = Duration::from_secs(1);
const MAX_PLUGIN_OUTPUT_BYTES: usize = 16 * 1024 * 1024;
const MAX_ERROR_LOG_BYTES: usize = 8 * 1024;

impl std::fmt::Display for PluginError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for PluginError {}

#[derive(Debug)]
pub struct PluginResult {
    pub manifest: EmitterManifest,
    pub response: EmitResponse,
    pub capability_warnings: Vec<String>,
    pub logs: Vec<u8>,
}

pub fn invoke(
    executable: &OsStr,
    target: &str,
    ir: &Document,
    options: BTreeMap<String, Value>,
    strict: bool,
) -> Result<PluginResult, PluginError> {
    let mut canonical = ir.clone();
    canonical.canonicalize();
    canonical.validate().map_err(|errors| {
        PluginError(
            errors
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join("\n"),
        )
    })?;
    let ir = &canonical;

    let resolved = resolve_executable(executable)?;
    let manifest_output = run_command(&resolved.program, &resolved.command("describe"), None)?;
    ensure_success(
        executable,
        "describe",
        &manifest_output.status,
        &manifest_output.stderr,
    )?;
    let manifest: EmitterManifest =
        serde_json::from_slice(&manifest_output.stdout).map_err(|error| {
            with_emitter_log(
                &format!(
                    "{} returned an invalid manifest: {error}",
                    executable.to_string_lossy()
                ),
                &manifest_output.stderr,
            )
        })?;
    if manifest.name != target {
        return Err(PluginError(format!(
            "{} identifies itself as {}, expected {target}",
            executable.to_string_lossy(),
            manifest.name
        )));
    }
    manifest.validate(&ir.ir_version).map_err(PluginError)?;
    let capability_warnings: Vec<String> = manifest
        .unsupported_features(ir)
        .into_iter()
        .map(|warning| warning.0)
        .collect();
    if strict && !capability_warnings.is_empty() {
        return Err(with_emitter_log(
            &capability_warnings.join("\n"),
            &manifest_output.stderr,
        ));
    }

    let mode = if strict {
        EmitMode::Strict
    } else {
        EmitMode::Lossy
    };
    let request = EmitRequest {
        protocol: PROTOCOL_VERSION,
        mode,
        ir,
        options,
        target,
    };
    let input = serde_json::to_vec(&request)
        .map_err(|error| PluginError(format!("cannot encode emitter request: {error}")))?;
    let output = run_command(&resolved.program, &resolved.command("emit"), Some(input))?;
    ensure_success(executable, "emit", &output.status, &output.stderr)?;
    let mut logs = manifest_output.stderr;
    logs.extend(&output.stderr);
    let response: EmitResponse = serde_json::from_slice(&output.stdout).map_err(|error| {
        with_emitter_log(
            &format!(
                "{} returned an invalid response: {error}",
                executable.to_string_lossy()
            ),
            &logs,
        )
    })?;
    validate_response_for_document(&response, ir, mode, manifest.capabilities.opaque)
        .map_err(|error| with_emitter_log(&error, &logs))?;
    Ok(PluginResult {
        manifest,
        response,
        capability_warnings,
        logs,
    })
}

fn run_command(
    executable: &OsStr,
    args: &[impl AsRef<OsStr>],
    input: Option<Vec<u8>>,
) -> Result<Output, PluginError> {
    run_command_with_limits(
        executable,
        args,
        input,
        PLUGIN_TIMEOUT,
        MAX_PLUGIN_OUTPUT_BYTES,
    )
}

fn run_command_with_limits(
    executable: &OsStr,
    args: &[impl AsRef<OsStr>],
    input: Option<Vec<u8>>,
    timeout: Duration,
    output_limit: usize,
) -> Result<Output, PluginError> {
    let mut command = Command::new(executable);
    command
        .args(args)
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(unix)]
    command.process_group(0);
    let mut child = command.spawn().map_err(|error| {
        PluginError(format!(
            "cannot start {}: {error}",
            executable.to_string_lossy()
        ))
    })?;
    let writer = match (input, child.stdin.take()) {
        (Some(input), Some(mut stdin)) => Some(thread::spawn(move || stdin.write_all(&input))),
        (Some(_), None) => {
            terminate_before_reap(&mut child);
            return Err(PluginError("emitter stdin is unavailable".into()));
        }
        (None, _) => None,
    };
    let Some(stdout) = child.stdout.take() else {
        terminate_before_reap(&mut child);
        return Err(PluginError("emitter stdout is unavailable".into()));
    };
    let Some(stderr) = child.stderr.take() else {
        terminate_before_reap(&mut child);
        return Err(PluginError("emitter stderr is unavailable".into()));
    };
    let output_error: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
    let stdout_error = Arc::clone(&output_error);
    let stdout_reader = thread::spawn(move || {
        let result = read_output(stdout, output_limit);
        if let Err(error) = &result {
            *stdout_error.lock().unwrap() = Some(error.0.clone());
        }
        result
    });
    let stderr_error = Arc::clone(&output_error);
    let stderr_reader = thread::spawn(move || {
        let result = read_output(stderr, output_limit);
        if let Err(error) = &result {
            *stderr_error.lock().unwrap() = Some(error.0.clone());
        }
        result
    });

    let deadline = Instant::now() + timeout;
    let command_name = args
        .first()
        .map(|argument| argument.as_ref().to_string_lossy().into_owned())
        .unwrap_or_else(|| "plugin".to_owned());
    let reaped = loop {
        if let Some(error) = output_error.lock().ok().and_then(|error| error.clone()) {
            terminate_before_reap(&mut child);
            return Err(PluginError(error));
        }
        match leader_state(&mut child) {
            Ok(Leader::ExitedUnreaped) => break false,
            Ok(Leader::ExitedReaped) => break true,
            Ok(Leader::Running) if Instant::now() >= deadline => {
                terminate_before_reap(&mut child);
                return Err(timeout_error(executable, &command_name));
            }
            Ok(Leader::Running) => thread::sleep(PLUGIN_POLL_INTERVAL),
            Err(error) => {
                terminate_before_reap(&mut child);
                return Err(PluginError(format!(
                    "cannot wait for {}: {error}",
                    executable.to_string_lossy()
                )));
            }
        }
    };

    if !reaped {
        signal_process_group(&child);
    }
    let status = child.wait().map_err(|error| {
        PluginError(format!(
            "cannot wait for {}: {error}",
            executable.to_string_lossy()
        ))
    })?;
    (|| {
        if let Some(writer) = writer
            && let Err(error) =
                join_before(writer, deadline, executable, &command_name, "stdin writer")?
            && error.kind() != std::io::ErrorKind::BrokenPipe
        {
            return Err(PluginError(format!(
                "cannot send request to emitter: {error}"
            )));
        }
        let stdout = join_before(
            stdout_reader,
            deadline,
            executable,
            &command_name,
            "stdout reader",
        )??;
        let stderr = join_before(
            stderr_reader,
            deadline,
            executable,
            &command_name,
            "stderr reader",
        )??;
        Ok(Output {
            status,
            stdout,
            stderr,
        })
    })()
}

fn read_output(mut pipe: impl Read, limit: usize) -> Result<Vec<u8>, PluginError> {
    let mut bytes = Vec::new();
    pipe.by_ref()
        .take(limit.saturating_add(1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|error| PluginError(format!("cannot read emitter output: {error}")))?;
    if bytes.len() > limit {
        return Err(PluginError(format!(
            "emitter output exceeded {limit} bytes"
        )));
    }
    Ok(bytes)
}

fn join_before<T>(
    handle: thread::JoinHandle<T>,
    deadline: Instant,
    executable: &OsStr,
    command: &str,
    name: &str,
) -> Result<T, PluginError> {
    loop {
        if handle.is_finished() {
            return handle
                .join()
                .map_err(|_| PluginError(format!("emitter {name} panicked")));
        }
        if Instant::now() >= deadline + PLUGIN_DRAIN_GRACE {
            return Err(timeout_error(executable, command));
        }
        thread::sleep(PLUGIN_POLL_INTERVAL);
    }
}

fn timeout_error(executable: &OsStr, command: &str) -> PluginError {
    PluginError(format!(
        "{} {command} timed out",
        executable.to_string_lossy()
    ))
}

fn terminate_before_reap(child: &mut std::process::Child) {
    terminate_process_tree(child);
    let _ = child.kill();
    let _ = child.wait();
}

fn terminate_process_tree(child: &std::process::Child) {
    signal_process_group(child);
    #[cfg(windows)]
    #[cfg(windows)]
    let _ = std::process::Command::new("taskkill")
        .args(["/PID", &child.id().to_string(), "/T", "/F"])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
}

#[cfg_attr(not(unix), allow(dead_code))]
#[derive(PartialEq, Eq)]
enum Leader {
    Running,
    ExitedUnreaped,
    ExitedReaped,
}

fn leader_state(child: &mut std::process::Child) -> std::io::Result<Leader> {
    #[cfg(unix)]
    {
        match waitid_exited_unreaped(child)? {
            Some(true) => return Ok(Leader::ExitedUnreaped),
            Some(false) => return Ok(Leader::Running),
            None => {}
        }
    }

    Ok(match child.try_wait()? {
        Some(_) => Leader::ExitedReaped,
        None => Leader::Running,
    })
}

#[cfg(unix)]
fn waitid_exited_unreaped(child: &std::process::Child) -> std::io::Result<Option<bool>> {
    #[cfg(test)]
    if WAITID_UNSUPPORTED.with(|unsupported| unsupported.get()) {
        return Ok(None);
    }
    let mut info = unsafe { std::mem::zeroed::<libc::siginfo_t>() };
    loop {
        let result = unsafe {
            libc::waitid(
                libc::P_PID,
                child.id(),
                &mut info,
                libc::WEXITED | libc::WNOWAIT | libc::WNOHANG,
            )
        };
        if result == 0 {
            return Ok(Some(unsafe { info.si_pid() } != 0));
        }
        let error = std::io::Error::last_os_error();
        if waitid_unsupported(&error) {
            return Ok(None);
        }
        if error.raw_os_error() != Some(libc::EINTR) {
            return Err(error);
        }
    }
}

#[cfg(unix)]
fn waitid_unsupported(error: &std::io::Error) -> bool {
    matches!(
        error.raw_os_error(),
        Some(libc::EINVAL) | Some(libc::ENOSYS)
    )
}

#[cfg_attr(not(unix), allow(unused_variables))]
#[derive(Debug)]
struct Executable {
    program: OsString,
    prefix: Vec<OsString>,
}

impl Executable {
    fn command(&self, name: &str) -> Vec<OsString> {
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

fn resolve_executable(executable: &OsStr) -> Result<Executable, PluginError> {
    let name = executable.to_string_lossy();
    let found = if name.contains(std::path::MAIN_SEPARATOR) {
        None
    } else {
        std::env::var_os("PATH")
            .and_then(|search| find_in_path(&name, EXECUTABLE_SUFFIXES, &search).ok())
    };
    match found {
        Some((path, suffix)) => start_resolved(path, suffix),
        None => Ok(Executable {
            program: executable.to_owned(),
            prefix: Vec::new(),
        }),
    }
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

#[cfg_attr(not(unix), allow(unused_variables))]
fn signal_process_group(child: &std::process::Child) {
    #[cfg(unix)]
    {
        #[cfg(test)]
        GROUP_SIGNALS.lock().unwrap().push(child.id());
        unsafe {
            let _ = libc::kill(-(child.id() as i32), libc::SIGKILL);
        }
    }
}

#[cfg(all(test, unix))]
static GROUP_SIGNALS: Mutex<Vec<u32>> = Mutex::new(Vec::new());

#[cfg(all(test, unix))]
thread_local! {
    static WAITID_UNSUPPORTED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

#[cfg(all(test, unix))]
fn without_waitid<T>(body: impl FnOnce() -> T) -> T {
    WAITID_UNSUPPORTED.with(|unsupported| unsupported.set(true));
    let result = body();
    WAITID_UNSUPPORTED.with(|unsupported| unsupported.set(false));
    result
}

#[cfg(all(test, unix))]
fn group_signals(pid: u32) -> usize {
    GROUP_SIGNALS
        .lock()
        .unwrap()
        .iter()
        .filter(|signalled| **signalled == pid)
        .count()
}

fn ensure_success(
    executable: &OsStr,
    command: &str,
    status: &std::process::ExitStatus,
    stderr: &[u8],
) -> Result<(), PluginError> {
    if status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(stderr);
    let detail = log_tail(stderr.trim());
    Err(PluginError(if detail.is_empty() {
        format!(
            "{} {command} exited with {status}",
            executable.to_string_lossy()
        )
    } else {
        format!(
            "{} {command} exited with {status}: {detail}",
            executable.to_string_lossy()
        )
    }))
}

fn with_emitter_log(error: &str, logs: &[u8]) -> PluginError {
    let log = log_tail(String::from_utf8_lossy(logs).trim());
    if log.is_empty() {
        PluginError(error.to_owned())
    } else {
        PluginError(format!("{error}\nemitter log: {log}"))
    }
}

pub fn log_tail(text: &str) -> String {
    if text.len() <= MAX_ERROR_LOG_BYTES {
        return text.to_owned();
    }
    let start = (text.len() - MAX_ERROR_LOG_BYTES..text.len())
        .find(|index| text.is_char_boundary(*index))
        .unwrap_or(0);
    format!("...{}", &text[start..])
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(target_os = "linux")]
    use std::path::Path;

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
    fn a_name_given_as_a_path_is_run_where_it_sits() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("pons-emit-py");
        std::fs::write(&path, b"#!/bin/sh\n").unwrap();
        let resolved = resolve_executable(OsStr::new(&path)).unwrap();
        assert_eq!(resolved.program, path);
        assert!(resolved.prefix.is_empty());
        assert_eq!(resolved.command("emit"), vec![OsString::from("emit")]);
    }

    #[test]
    fn rejects_invalid_ir_before_starting_plugin() {
        let ir: Document = serde_json::from_str(
            r#"{"ir_version":"9.0","info":{"title":"Example","version":"1"}}"#,
        )
        .unwrap();
        let error = invoke(
            OsStr::new("plugin-does-not-exist"),
            "fixture",
            &ir,
            BTreeMap::new(),
            true,
        )
        .unwrap_err();

        assert!(error.0.contains("unsupported IR version 9.0"));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn times_out_plugin_commands() {
        let error = run_command_with_limits(
            OsStr::new("sh"),
            &["-c", "sleep 1"],
            None,
            Duration::from_millis(20),
            MAX_PLUGIN_OUTPUT_BYTES,
        )
        .unwrap_err();
        assert!(error.0.contains("timed out"));

        let error = run_command_with_limits(
            OsStr::new("sh"),
            &["-c", "sleep 1"],
            Some(b"{}".to_vec()),
            Duration::from_millis(20),
            MAX_PLUGIN_OUTPUT_BYTES,
        )
        .unwrap_err();
        assert!(error.0.contains("timed out"));

        let marker =
            std::env::temp_dir().join(format!("pons-plugin-descendant-{}", std::process::id()));
        let _ = std::fs::remove_file(&marker);
        let command = format!("(sleep 0.2; touch {}) & exit 0", marker.display());
        let output = run_command_with_limits(
            OsStr::new("sh"),
            &["-c", &command],
            None,
            Duration::from_millis(100),
            MAX_PLUGIN_OUTPUT_BYTES,
        )
        .unwrap();
        assert!(output.status.success());
        std::thread::sleep(Duration::from_millis(300));
        assert!(!marker.exists());
        let _ = std::fs::remove_file(marker);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn exited_children_stay_unreaped_until_the_group_is_signalled() {
        let mut command = Command::new("sh");
        command
            .args(["-c", "exit 0"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        command.process_group(0);
        let mut child = command.spawn().unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while leader_state(&mut child).unwrap() == Leader::Running {
            assert!(Instant::now() < deadline, "plugin never exited");
            thread::sleep(PLUGIN_POLL_INTERVAL);
        }

        assert!(Path::new(&format!("/proc/{}", child.id())).exists());
        signal_process_group(&child);
        assert_eq!(child.wait().unwrap().code(), Some(0));
        assert!(!Path::new(&format!("/proc/{}", child.id())).exists());
    }

    #[cfg(unix)]
    #[test]
    fn falls_back_to_try_wait_only_where_waitid_cannot_report() {
        for errno in [libc::EINVAL, libc::ENOSYS] {
            assert!(
                waitid_unsupported(&std::io::Error::from_raw_os_error(errno)),
                "errno {errno} means this unix has no usable WNOWAIT"
            );
        }
        for errno in [libc::EINTR, libc::ECHILD, libc::EPERM] {
            assert!(
                !waitid_unsupported(&std::io::Error::from_raw_os_error(errno)),
                "errno {errno} is a real wait failure, not a missing capability"
            );
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn limits_plugin_output() {
        let error = run_command_with_limits(
            OsStr::new("sh"),
            &["-c", "printf 12345678901234567"],
            None,
            Duration::from_millis(100),
            16,
        )
        .unwrap_err();
        assert!(error.0.contains("output exceeded 16 bytes"));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn keeps_the_answer_of_a_target_that_stops_reading_its_stdin() {
        let output = run_command_with_limits(
            OsStr::new("sh"),
            &["-c", r#"printf '{"protocol":2}'; exit 0"#],
            Some(vec![b'x'; 1024 * 1024]),
            Duration::from_secs(10),
            MAX_PLUGIN_OUTPUT_BYTES,
        )
        .unwrap();
        assert_eq!(output.stdout, br#"{"protocol":2}"#);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_reader_that_finishes_after_the_deadline_still_wins() {
        let output = run_command_with_limits(
            OsStr::new("sh"),
            &[
                "-c",
                r#"setsid sh -c "sleep 0.3" & printf response; exit 0"#,
            ],
            None,
            Duration::from_millis(50),
            MAX_PLUGIN_OUTPUT_BYTES,
        )
        .unwrap();
        assert_eq!(output.stdout, b"response");
    }

    #[cfg(target_os = "linux")]
    fn stub_emitter(name: &str, script: &str) -> std::path::PathBuf {
        use std::process;

        let path = std::env::temp_dir().join(format!("pons-plugin-{name}-{}", process::id()));

        let mut writer = Command::new("sh")
            .args(["-c", "cat > \"$1\" && chmod 755 \"$1\"", "sh"])
            .arg(&path)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        writer
            .stdin
            .take()
            .unwrap()
            .write_all(script.as_bytes())
            .unwrap();
        let status = writer.wait().unwrap();
        assert!(status.success(), "cannot write the {name} stub: {status}");
        path
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn only_an_unreaped_leader_has_its_group_signalled() {
        let directory =
            std::env::temp_dir().join(format!("pons-plugin-group-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let pid_file = directory.join("pid");
        let marker = directory.join("marker");
        let _ = std::fs::remove_file(&marker);

        let ready = directory.join("ready");
        let _ = std::fs::remove_file(&ready);

        let running = format!(
            "echo $$ > {}; (sleep 0.5; touch {}) & sleep 5",
            pid_file.display(),
            marker.display()
        );
        let error = run_command_with_limits(
            OsStr::new("sh"),
            &["-c", &running],
            None,
            Duration::from_millis(200),
            MAX_PLUGIN_OUTPUT_BYTES,
        )
        .unwrap_err();
        assert!(error.0.contains("timed out"), "{}", error.0);
        let pid = read_pid(&pid_file);
        thread::sleep(Duration::from_millis(500));
        assert!(!marker.exists(), "a still-running target loses its group");
        assert_eq!(group_signals(pid), 1, "its group is signalled once");

        let escaped = format!(
            "echo $$ > {}; setsid sh -c 'touch {}; sleep 5' & while [ ! -f {} ]; do :; done; \
             exit 0",
            pid_file.display(),
            ready.display(),
            ready.display()
        );
        let error = run_command_with_limits(
            OsStr::new("sh"),
            &["-c", &escaped],
            None,
            Duration::from_millis(50),
            MAX_PLUGIN_OUTPUT_BYTES,
        )
        .unwrap_err();
        assert!(error.0.contains("timed out"), "{}", error.0);
        let pid = read_pid(&pid_file);
        assert_eq!(
            group_signals(pid),
            1,
            "a descendant holding the pipe outlives the leader, and the group was already \
             signalled while that leader was unreaped"
        );
        std::fs::remove_dir_all(&directory).unwrap();
    }

    #[cfg(all(test, target_os = "linux"))]
    fn read_pid(path: &std::path::Path) -> u32 {
        std::fs::read_to_string(path)
            .unwrap()
            .trim()
            .parse()
            .unwrap()
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_platform_without_wnowait_leaves_the_reaped_leaders_group_alone() {
        let directory =
            std::env::temp_dir().join(format!("pons-plugin-reaped-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let pid_file = directory.join("pid");
        let marker = directory.join("marker");
        let _ = std::fs::remove_file(&marker);

        let command = format!(
            "echo $$ > {}; (sleep 0.5; touch {}) & exit 0",
            pid_file.display(),
            marker.display()
        );
        let output = without_waitid(|| {
            run_command_with_limits(
                OsStr::new("sh"),
                &["-c", &command],
                None,
                Duration::from_millis(100),
                MAX_PLUGIN_OUTPUT_BYTES,
            )
        })
        .unwrap();
        assert!(output.status.success(), "{}", output.status);

        let pid = read_pid(&pid_file);
        assert_eq!(
            group_signals(pid),
            0,
            "try_wait reaped the leader, so its group must be left alone"
        );
        assert!(
            marker.exists(),
            "the descendant a reaped leader left behind survives, because its group was never \
             signalled"
        );
        std::fs::remove_dir_all(&directory).unwrap();
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn bounds_the_emitter_log_it_reports_in_an_error() {
        let output = run_command_with_limits(
            OsStr::new("sh"),
            &[
                "-c",
                r#"dd if=/dev/zero bs=1048576 count=9 2>/dev/null | tr '\000' 'x' 1>&2; exit 3"#,
            ],
            None,
            Duration::from_secs(10),
            MAX_PLUGIN_OUTPUT_BYTES,
        )
        .unwrap();
        assert!(!output.status.success());
        let error =
            ensure_success(OsStr::new("stub"), "emit", &output.status, &output.stderr).unwrap_err();
        assert!(
            error.0.contains("exited with exit status: 3"),
            "{}",
            &error.0[..80]
        );
        assert!(
            error.0.len() < 4 * MAX_ERROR_LOG_BYTES,
            "a 9 MiB log cannot become a 9 MiB error line: {} bytes",
            error.0.len()
        );
        assert!(error.0.contains(": ..."), "{}", &error.0[..80]);
        assert!(error.0.ends_with('x'), "the tail of the log is kept");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn an_unparsable_answer_keeps_the_emitters_explanation() {
        let document: Document =
            serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();

        let unparsable_manifest = stub_emitter(
            "unparsable-manifest",
            r#"#!/bin/sh
printf '%s\n' 'the range key is protokl' >&2
printf '%s' '{"name":"fixture","protokl":{"min":"2.0","max":"2.0"}}'
"#,
        );
        let error = invoke(
            unparsable_manifest.as_os_str(),
            "fixture",
            &document,
            BTreeMap::new(),
            false,
        )
        .unwrap_err();
        assert!(error.0.contains("invalid manifest"), "{}", error.0);
        assert!(error.0.contains("unknown field `protokl`"), "{}", error.0);
        assert!(error.0.contains("the range key is protokl"), "{}", error.0);
        let _ = std::fs::remove_file(&unparsable_manifest);

        let unparsable_response = stub_emitter(
            "unparsable-response",
            r#"#!/bin/sh
if [ "$1" = describe ]; then
  printf '%s' '{"name":"fixture","version":"1.0.0","protocol":{"min":"2.0","max":"2.0"},"ir_version":{"min":"0.2","max":"0.2"},"capabilities":{"streaming":["sse"],"enum_reprs":["string","external","internal","adjacent","untagged"],"direction_split":true,"provides":true,"flattened":true,"opaque":true,"object_openness":["reject","preserve_typed","preserve_opaque"]}}'
  exit 0
fi
cat > /dev/null
printf '%s\n' 'the coverage key is coverag' >&2
printf '%s' '{"protocol":2,"coverag":{}}'
"#,
        );
        let error = invoke(
            unparsable_response.as_os_str(),
            "fixture",
            &document,
            BTreeMap::new(),
            false,
        )
        .unwrap_err();
        assert!(error.0.contains("invalid response"), "{}", error.0);
        assert!(error.0.contains("unknown field `coverag`"), "{}", error.0);
        assert!(
            error.0.contains("the coverage key is coverag"),
            "{}",
            error.0
        );
        let _ = std::fs::remove_file(&unparsable_response);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_refusing_emitter_gets_its_own_log_reported() {
        let document: Document =
            serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();

        let refusing = stub_emitter(
            "refusing",
            r#"#!/bin/sh
printf '%s\n' 'cannot model preserved objects' >&2
printf '%s' '{"name":"fixture","version":"1.0.0","protocol":{"min":"2.0","max":"2.0"},"ir_version":{"min":"0.2","max":"0.2"},"capabilities":{"streaming":["sse"],"enum_reprs":["string","external","internal","adjacent","untagged"],"direction_split":true,"provides":true,"flattened":true,"object_openness":["reject","preserve_typed","preserve_opaque"]}}'
"#,
        );
        let error = invoke(
            refusing.as_os_str(),
            "fixture",
            &document,
            BTreeMap::new(),
            true,
        )
        .unwrap_err();
        assert!(error.0.contains("does not declare opaque value support"));
        assert!(
            error.0.contains("cannot model preserved objects"),
            "{}",
            error.0
        );
        let _ = std::fs::remove_file(&refusing);

        let unclassified = stub_emitter(
            "unclassified",
            r#"#!/bin/sh
if [ "$1" = describe ]; then
  printf '%s' '{"name":"fixture","version":"1.0.0","protocol":{"min":"2.0","max":"2.0"},"ir_version":{"min":"0.2","max":"0.2"},"capabilities":{"streaming":["sse"],"enum_reprs":["string","external","internal","adjacent","untagged"],"direction_split":true,"provides":true,"flattened":true,"opaque":true,"object_openness":["reject","preserve_typed","preserve_opaque"]}}'
  exit 0
fi
cat > /dev/null
printf '%s\n' 'gave up on types.User' >&2
printf '%s' '{"protocol":2,"files":[{"path":"client.ts","content":"generated"}],"coverage":{"exact":["types.EventQuery","types.LoginError"],"opaque":["operations.auth.login","operations.events.watch","types.LoginRequest"],"lossy":[],"unsupported":[]}}'
"#,
        );
        let error = invoke(
            unclassified.as_os_str(),
            "fixture",
            &document,
            BTreeMap::new(),
            true,
        )
        .unwrap_err();
        assert!(error.0.contains("missing subjects"), "{}", error.0);
        assert!(error.0.contains("gave up on types.User"), "{}", error.0);
        let _ = std::fs::remove_file(&unclassified);
    }
}
