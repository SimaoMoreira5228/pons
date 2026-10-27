#[cfg(all(test, unix))]
use std::sync::Mutex;

pub(super) fn terminate_before_reap(child: &mut std::process::Child) {
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
pub(super) enum Leader {
    Running,
    ExitedUnreaped,
    ExitedReaped,
}

pub(super) fn leader_state(child: &mut std::process::Child) -> std::io::Result<Leader> {
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
pub(super) fn signal_process_group(child: &std::process::Child) {
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

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(target_os = "linux")]
    use std::ffi::OsStr;
    #[cfg(target_os = "linux")]
    use std::os::unix::process::CommandExt;
    #[cfg(target_os = "linux")]
    use std::path::Path;
    #[cfg(target_os = "linux")]
    use std::process::{Command, Stdio};
    #[cfg(target_os = "linux")]
    use std::thread;
    #[cfg(target_os = "linux")]
    use std::time::{Duration, Instant};

    #[cfg(target_os = "linux")]
    use crate::process::run::{
        MAX_PLUGIN_OUTPUT_BYTES, PLUGIN_POLL_INTERVAL, run_command_with_limits,
    };

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
}
