use std::ffi::OsStr;
use std::io::{Read, Write};
#[cfg(unix)]
use std::os::unix::process::CommandExt;
use std::process::{Command, Output, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use super::PluginError;
use super::termination::{Leader, leader_state, signal_process_group, terminate_before_reap};

const PLUGIN_TIMEOUT: Duration = Duration::from_secs(30);
pub(super) const PLUGIN_POLL_INTERVAL: Duration = Duration::from_millis(10);
const PLUGIN_DRAIN_GRACE: Duration = Duration::from_secs(1);
pub(super) const MAX_PLUGIN_OUTPUT_BYTES: usize = 16 * 1024 * 1024;
const MAX_ERROR_LOG_BYTES: usize = 8 * 1024;

pub(super) fn run_command(
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

pub(super) fn run_command_with_limits(
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

pub(super) fn ensure_success(
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

pub(super) fn with_emitter_log(error: &str, logs: &[u8]) -> PluginError {
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
            &["-c", r#"printf '{"protocol":3}'; exit 0"#],
            Some(vec![b'x'; 1024 * 1024]),
            Duration::from_secs(10),
            MAX_PLUGIN_OUTPUT_BYTES,
        )
        .unwrap();
        assert_eq!(output.stdout, br#"{"protocol":3}"#);
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
}
