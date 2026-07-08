//! Watches the TUI-owned agent child process and records its exit diagnosis
//! (#1047).
//!
//! When the agent child dies mid-session (e.g. a panic-abort near a full
//! context window), the UDS connection simply closes — without this watcher
//! the TUI could only say "Agent disconnected" with no way to diagnose WHY.
//! The watcher reaps the child and stores a human-readable exit description
//! that the disconnect notification surfaces.

use std::sync::{Arc, Mutex};

/// Shared slot the watcher fills with the child's exit description once the
/// child has been reaped. `None` until the child exits.
pub type ExitDetailSlot = Arc<Mutex<Option<String>>>;

/// Take ownership of the spawned agent child, reap it in the background, and
/// return the slot its exit description will be written to.
///
/// The watcher owns the `Child` from here on (tokio's `Child::wait` needs
/// `&mut`); callers that need to terminate the agent later should keep its
/// PID and signal the process group (see
/// [`crate::infrastructure::process::terminate_process_group`]).
pub fn watch_child(mut child: tokio::process::Child) -> ExitDetailSlot {
    let slot: ExitDetailSlot = Arc::new(Mutex::new(None));
    let writer = Arc::clone(&slot);
    tokio::spawn(async move {
        if let Ok(status) = child.wait().await {
            let detail = describe_exit(status);
            *writer.lock().unwrap_or_else(|e| e.into_inner()) = Some(detail);
        }
    });
    slot
}

/// Read the recorded exit detail, if the child has exited and been reaped.
pub fn read_exit_detail(slot: &ExitDetailSlot) -> Option<String> {
    slot.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

/// Human-readable description of how the agent child exited.
pub fn describe_exit(status: std::process::ExitStatus) -> String {
    use std::os::unix::process::ExitStatusExt;
    if let Some(sig) = status.signal() {
        match signal_name(sig) {
            Some(name) => format!("agent process aborted: signal {sig} ({name})"),
            None => format!("agent process aborted: signal {sig}"),
        }
    } else if let Some(code) = status.code() {
        format!("agent process exited with code {code}")
    } else {
        "agent process exited with unknown status".to_string()
    }
}

/// Names for the signals an agent child plausibly dies from.
fn signal_name(sig: i32) -> Option<&'static str> {
    Some(match sig {
        libc::SIGHUP => "SIGHUP",
        libc::SIGINT => "SIGINT",
        libc::SIGQUIT => "SIGQUIT",
        libc::SIGILL => "SIGILL",
        libc::SIGABRT => "SIGABRT",
        libc::SIGBUS => "SIGBUS",
        libc::SIGFPE => "SIGFPE",
        libc::SIGKILL => "SIGKILL",
        libc::SIGSEGV => "SIGSEGV",
        libc::SIGPIPE => "SIGPIPE",
        libc::SIGTERM => "SIGTERM",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn exit_detail_of(mut cmd: tokio::process::Command) -> String {
        let child = cmd
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("spawn test child");
        let slot = watch_child(child);
        for _ in 0..200 {
            if let Some(detail) = read_exit_detail(&slot) {
                return detail;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        panic!("watcher did not record the child's exit in time");
    }

    #[tokio::test]
    async fn records_signal_exit_with_name() {
        let mut cmd = tokio::process::Command::new("sh");
        cmd.args(["-c", "kill -ABRT $$"]);
        let detail = exit_detail_of(cmd).await;
        assert_eq!(detail, "agent process aborted: signal 6 (SIGABRT)");
    }

    #[tokio::test]
    async fn records_nonzero_exit_code() {
        let mut cmd = tokio::process::Command::new("sh");
        cmd.args(["-c", "exit 3"]);
        let detail = exit_detail_of(cmd).await;
        assert_eq!(detail, "agent process exited with code 3");
    }

    #[test]
    fn unknown_signal_has_no_name_suffix() {
        use std::os::unix::process::ExitStatusExt;
        let status = std::process::ExitStatus::from_raw(34); // signal 34 (real-time)
        assert_eq!(describe_exit(status), "agent process aborted: signal 34");
    }
}
