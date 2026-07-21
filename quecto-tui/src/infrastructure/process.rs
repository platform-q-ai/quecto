//! Safe process-group management for child agent cleanup.
//!
//! Provides checked PID conversion (u32 → i32) and process-group signal
//! helpers so that wrapping casts cannot accidentally target PID 1 (init).
//!
//! For incoming signal handling (SIGTSTP, SIGWINCH), see the [`crate::infrastructure::signals`] module.

/// Grace period between SIGTERM and SIGKILL (milliseconds).
pub const TERMINATE_GRACE_MS: u64 = 200;

/// Interval between `try_wait()` polls while waiting for graceful exit.
const TERMINATE_POLL_TICK_MS: u64 = 10;

/// Error returned when a PID cannot be safely used for signalling.
#[derive(Debug, PartialEq)]
pub enum QuectodError {
    /// PID exceeds `i32::MAX` — wrapping `as i32` would produce a wrong value.
    Overflow(u32),
    /// PID 0 would signal the caller's own process group.
    Zero,
}

impl std::fmt::Display for QuectodError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            QuectodError::Overflow(pid) => {
                write!(f, "PID {pid} exceeds i32::MAX, cannot safely convert")
            }
            QuectodError::Zero => write!(f, "PID 0 would signal the caller's own process group"),
        }
    }
}

impl std::error::Error for QuectodError {}

/// Convert a `u32` PID (from `child.id()`) to a safe `i32` for use with `libc::kill`.
///
/// Returns `Err` if the PID is 0 or exceeds `i32::MAX`.
pub fn checked_pid(pid: u32) -> Result<i32, QuectodError> {
    if pid == 0 {
        return Err(QuectodError::Zero);
    }
    i32::try_from(pid).map_err(|_| QuectodError::Overflow(pid))
}

/// Send a signal to an entire process group.
///
/// Uses `libc::kill(-pid, sig)` — the negative PID targets the group.
/// Returns the raw libc result (0 on success, -1 on error).
///
/// Requires `quectod > 0`. Returns `-1` without signalling if `quectod <= 0`
/// (pid 0 would signal the caller's group, negative would flip sign).
pub(crate) fn kill_process_group(pid: i32, signal: libc::c_int) -> libc::c_int {
    if pid <= 0 {
        return -1;
    }
    // SAFETY: `quectod > 0` is checked above, so `-pid` targets that process group; libc handles invalid signals/pids by returning -1.
    unsafe { libc::kill(-pid, signal) }
}

/// Terminate a child agent and its entire process group.
///
/// 1. SIGTERM the process group (graceful shutdown).
/// 2. Poll `child.try_wait()` in `TERMINATE_POLL_TICK_MS` ticks up to `grace_ms`.
/// 3. If still alive after the grace period, SIGKILL the process group.
///
/// If the PID cannot be safely converted, falls back to killing only the
/// direct child via `child.kill()`.
pub async fn terminate_child(child: &mut tokio::process::Child, grace_ms: u64) {
    if let Some(raw_pid) = child.id() {
        match checked_pid(raw_pid) {
            Ok(pid) => {
                let rc = kill_process_group(pid, libc::SIGTERM);
                if rc == -1 && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH) {
                    // Process group already gone, skip grace period.
                    let _ = child.wait().await;
                    return;
                }

                // Poll for exit with short ticks to avoid unnecessary delay.
                let deadline =
                    tokio::time::Instant::now() + std::time::Duration::from_millis(grace_ms);
                loop {
                    match child.try_wait() {
                        Ok(Some(_)) => return, // Already exited after SIGTERM.
                        Ok(None) => {}
                        Err(_) => break,
                    }
                    if tokio::time::Instant::now() >= deadline {
                        break;
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(TERMINATE_POLL_TICK_MS))
                        .await;
                }

                // Still alive — force kill the group.
                kill_process_group(pid, libc::SIGKILL);
            }
            Err(e) => {
                eprintln!("Warning: unsafe PID {raw_pid}, falling back to child.kill(): {e}");
                let _ = child.kill().await;
            }
        }
    } else {
        let _ = child.kill().await;
    }
    let _ = child.wait().await;
}

/// Terminate a process group that may have outlived its (already-reaped)
/// leader — the sub-agents-share-the-agent's-group case on TUI exit (#1047).
///
/// Signals nothing when the group has no members left (`kill(-pgid, 0)`
/// fails): an empty group's PGID may have been recycled by an unrelated
/// process, so a liveness probe gates every signal. While any member is
/// alive POSIX forbids reassigning the PGID, so a successful probe means the
/// group is still ours and safe to signal.
///
/// 1. Probe the group; return if empty.
/// 2. SIGTERM the group (graceful shutdown).
/// 3. Poll the probe in `TERMINATE_POLL_TICK_MS` ticks up to `grace_ms`.
/// 4. If members remain after the grace period, SIGKILL the group.
pub async fn terminate_group_if_alive(pgid: i32, grace_ms: u64) {
    if kill_process_group(pgid, 0) == -1 {
        return; // No members left — never signal a possibly-recycled PGID.
    }
    if kill_process_group(pgid, libc::SIGTERM) == -1 {
        return; // Group vanished between probe and signal.
    }
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(grace_ms);
    loop {
        if kill_process_group(pgid, 0) == -1 {
            return; // All members exited after SIGTERM.
        }
        if tokio::time::Instant::now() >= deadline {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(TERMINATE_POLL_TICK_MS)).await;
    }
    kill_process_group(pgid, libc::SIGKILL);
}

#[cfg(test)]
#[path = "process_tests.rs"]
mod tests;
