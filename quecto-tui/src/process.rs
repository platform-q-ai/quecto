//! Safe process-group management for child agent cleanup.
//!
//! Provides checked PID conversion (u32 → i32) and process-group signal
//! helpers so that wrapping casts cannot accidentally target PID 1 (init).
//!
//! For incoming signal handling (SIGTSTP, SIGWINCH), see the [`crate::signals`] module.

/// Grace period between SIGTERM and SIGKILL (milliseconds).
pub const TERMINATE_GRACE_MS: u64 = 200;

/// Error returned when a PID cannot be safely used for signalling.
#[derive(Debug, PartialEq)]
pub enum PidError {
    /// PID exceeds `i32::MAX` — wrapping `as i32` would produce a wrong value.
    Overflow(u32),
    /// PID 0 would signal the caller's own process group.
    Zero,
}

impl std::fmt::Display for PidError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PidError::Overflow(pid) => {
                write!(f, "PID {pid} exceeds i32::MAX, cannot safely convert")
            }
            PidError::Zero => write!(f, "PID 0 would signal the caller's own process group"),
        }
    }
}

impl std::error::Error for PidError {}

/// Convert a `u32` PID (from `child.id()`) to a safe `i32` for use with `libc::kill`.
///
/// Returns `Err` if the PID is 0 or exceeds `i32::MAX`.
pub fn checked_pid(pid: u32) -> Result<i32, PidError> {
    if pid == 0 {
        return Err(PidError::Zero);
    }
    i32::try_from(pid).map_err(|_| PidError::Overflow(pid))
}

/// Send a signal to an entire process group.
///
/// Uses `libc::kill(-pid, sig)` — the negative PID targets the group.
/// Returns the raw libc result (0 on success, -1 on error).
///
/// Requires `pid > 0`. Returns `-1` without signalling if `pid <= 0`
/// (pid 0 would signal the caller's group, negative would flip sign).
pub(crate) fn kill_process_group(pid: i32, signal: libc::c_int) -> libc::c_int {
    if pid <= 0 {
        return -1;
    }
    unsafe { libc::kill(-pid, signal) }
}

/// Terminate a child agent and its entire process group.
///
/// 1. SIGTERM the process group (graceful shutdown).
/// 2. Poll `child.try_wait()` in 10ms ticks up to `grace_ms`.
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
                    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
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

#[cfg(test)]
mod tests {
    use super::*;

    // --- checked_pid tests ---

    #[test]
    fn checked_pid_normal_value() {
        assert_eq!(checked_pid(1234), Ok(1234));
    }

    #[test]
    fn checked_pid_one() {
        assert_eq!(checked_pid(1), Ok(1));
    }

    #[test]
    fn checked_pid_i32_max() {
        let max = i32::MAX as u32; // 2_147_483_647
        assert_eq!(checked_pid(max), Ok(i32::MAX));
    }

    #[test]
    fn checked_pid_i32_max_plus_one_is_err() {
        let overflow = (i32::MAX as u32) + 1; // 2_147_483_648
        assert_eq!(checked_pid(overflow), Err(PidError::Overflow(overflow)));
    }

    #[test]
    fn checked_pid_u32_max_is_err() {
        assert_eq!(checked_pid(u32::MAX), Err(PidError::Overflow(u32::MAX)));
    }

    #[test]
    fn checked_pid_u32_max_does_not_produce_negative_one() {
        // This is the critical safety check: u32::MAX as i32 == -1,
        // and kill(-(-1), sig) == kill(1, sig) == kill init.
        let result = checked_pid(u32::MAX);
        assert!(result.is_err());
        // Verify the old unchecked cast WOULD have produced -1:
        assert_eq!(u32::MAX as i32, -1, "confirms the wrapping cast danger");
    }

    #[test]
    fn checked_pid_zero_is_err() {
        assert_eq!(checked_pid(0), Err(PidError::Zero));
    }

    #[test]
    fn pid_error_display_overflow() {
        let e = PidError::Overflow(2_147_483_648);
        let msg = e.to_string();
        assert!(msg.contains("2147483648"), "should mention the PID value");
        assert!(msg.contains("i32::MAX"), "should mention the limit");
    }

    #[test]
    fn pid_error_display_zero() {
        let e = PidError::Zero;
        let msg = e.to_string();
        assert!(msg.contains("PID 0"), "should mention PID 0");
    }

    #[test]
    fn pid_error_implements_std_error() {
        let e: Box<dyn std::error::Error> = Box::new(PidError::Zero);
        assert!(e.to_string().contains("PID 0"));
    }

    // --- kill_process_group tests ---

    #[test]
    fn kill_process_group_with_invalid_pid_returns_error() {
        // PID 999_999_999 almost certainly doesn't exist.
        let result = kill_process_group(999_999_999, libc::SIGTERM);
        // libc::kill returns -1 on error (ESRCH — no such process).
        assert_eq!(result, -1);
    }

    #[test]
    fn kill_process_group_rejects_zero_pid() {
        // PID 0 would signal the caller's own process group.
        let result = kill_process_group(0, libc::SIGTERM);
        assert_eq!(result, -1);
    }

    #[test]
    fn kill_process_group_rejects_negative_pid() {
        // Negative PID would flip sign and target an unrelated process.
        let result = kill_process_group(-1, libc::SIGTERM);
        assert_eq!(result, -1);
    }

    #[test]
    fn kill_process_group_with_nonexistent_pid_returns_error() {
        // Use a PID that almost certainly doesn't exist as a process group.
        // Signal 0 is a null signal — only checks permissions.
        let result = kill_process_group(999_999_998, 0);
        // Should fail with ESRCH (no such process group).
        assert_eq!(result, -1);
    }

    #[test]
    fn terminate_grace_ms_constant_is_reasonable() {
        assert!(TERMINATE_GRACE_MS >= 100, "grace period too short");
        assert!(TERMINATE_GRACE_MS <= 5000, "grace period too long");
    }
}
