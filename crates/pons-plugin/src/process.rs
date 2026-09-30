use std::collections::BTreeMap;
use std::ffi::OsStr;
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
const MAX_PLUGIN_OUTPUT_BYTES: usize = 16 * 1024 * 1024;

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

    let manifest_output = run_command(executable, &["describe"], None)?;
    ensure_success(
        executable,
        "describe",
        &manifest_output.status,
        &manifest_output.stderr,
    )?;
    let manifest: EmitterManifest =
        serde_json::from_slice(&manifest_output.stdout).map_err(|error| {
            PluginError(format!(
                "{} returned an invalid manifest: {error}",
                executable.to_string_lossy()
            ))
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
        return Err(PluginError(capability_warnings.join("\n")));
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
    let output = run_command(executable, &["emit"], Some(input))?;
    ensure_success(executable, "emit", &output.status, &output.stderr)?;
    let response: EmitResponse = serde_json::from_slice(&output.stdout).map_err(|error| {
        PluginError(format!(
            "{} returned an invalid response: {error}",
            executable.to_string_lossy()
        ))
    })?;
    validate_response_for_document(&response, ir, mode, manifest.capabilities.opaque)
        .map_err(PluginError)?;
    let mut logs = manifest_output.stderr;
    logs.extend(output.stderr);
    Ok(PluginResult {
        manifest,
        response,
        capability_warnings,
        logs,
    })
}

fn run_command(
    executable: &OsStr,
    args: &[&str],
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
    args: &[&str],
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
            terminate(&mut child);
            return Err(PluginError("emitter stdin is unavailable".into()));
        }
        (None, _) => None,
    };
    let Some(stdout) = child.stdout.take() else {
        terminate(&mut child);
        return Err(PluginError("emitter stdout is unavailable".into()));
    };
    let Some(stderr) = child.stderr.take() else {
        terminate(&mut child);
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
    let command_name = args.first().copied().unwrap_or("plugin");
    loop {
        if let Some(error) = output_error.lock().ok().and_then(|error| error.clone()) {
            terminate(&mut child);
            return Err(PluginError(error));
        }
        match exited_unreaped(&mut child) {
            Ok(true) => break,
            Ok(false) if Instant::now() >= deadline => {
                terminate(&mut child);
                return Err(timeout_error(executable, command_name));
            }
            Ok(false) => thread::sleep(PLUGIN_POLL_INTERVAL),
            Err(error) => {
                terminate(&mut child);
                return Err(PluginError(format!(
                    "cannot wait for {}: {error}",
                    executable.to_string_lossy()
                )));
            }
        }
    }
    signal_process_group(&child);
    let status = child.wait().map_err(|error| {
        PluginError(format!(
            "cannot wait for {}: {error}",
            executable.to_string_lossy()
        ))
    })?;
    if let Some(writer) = writer {
        join_before(writer, deadline, executable, command_name, "stdin writer")?
            .map_err(|error| PluginError(format!("cannot send request to emitter: {error}")))?;
    }
    let stdout = join_before(
        stdout_reader,
        deadline,
        executable,
        command_name,
        "stdout reader",
    )??;
    let stderr = join_before(
        stderr_reader,
        deadline,
        executable,
        command_name,
        "stderr reader",
    )??;
    Ok(Output {
        status,
        stdout,
        stderr,
    })
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
        if Instant::now() >= deadline {
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

fn terminate(child: &mut std::process::Child) {
    signal_process_group(child);
    let _ = child.kill();
    let _ = child.wait();
}

fn exited_unreaped(child: &mut std::process::Child) -> std::io::Result<bool> {
    #[cfg(unix)]
    {
        if let Some(exited) = waitid_exited_unreaped(child)? {
            return Ok(exited);
        }
    }
    child.try_wait().map(|status| status.is_some())
}

#[cfg(unix)]
fn waitid_exited_unreaped(child: &std::process::Child) -> std::io::Result<Option<bool>> {
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
fn signal_process_group(child: &std::process::Child) {
    #[cfg(unix)]
    unsafe {
        let _ = libc::kill(-(child.id() as i32), libc::SIGKILL);
    }
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
    let detail = String::from_utf8_lossy(stderr);
    let detail = detail.trim();
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

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(target_os = "linux")]
    use std::path::Path;

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
        while !exited_unreaped(&mut child).unwrap() {
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
}
