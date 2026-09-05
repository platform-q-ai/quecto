//! Ownership-scoped cleanup. A watched leader is not reaped until cleanup, so
//! its PID/PGID cannot be recycled while we signal the group. Linux descendants
//! in other groups are pinned and signalled individually, never by guessed PGID.

/// SIGTERM-to-SIGKILL window. The harness tears down its subagents and
/// container environments on SIGTERM (running each environment's kill
/// script, which under Podman takes on the order of a second), so the
/// window must cover that work; it ends early once the tree has exited.
pub const TERMINATE_GRACE_MS: u64 = 1500;
const TERMINATE_POLL_TICK_MS: u64 = 10;

#[cfg(target_os = "linux")]
#[path = "process_owned.rs"]
mod owned;
#[cfg(target_os = "linux")]
pub(crate) use owned::OwnedProcesses;

#[derive(Debug, PartialEq)]
pub enum QuectodError {
    Overflow(u32),
    Zero,
}
impl std::fmt::Display for QuectodError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Overflow(pid) => write!(f, "PID {pid} exceeds i32::MAX, cannot safely convert"),
            Self::Zero => write!(f, "PID 0 would signal the caller's own process group"),
        }
    }
}
impl std::error::Error for QuectodError {}
pub fn checked_pid(pid: u32) -> Result<i32, QuectodError> {
    if pid == 0 {
        return Err(QuectodError::Zero);
    }
    i32::try_from(pid).map_err(|_| QuectodError::Overflow(pid))
}
pub(crate) fn kill_process_group(pid: i32, signal: libc::c_int) -> libc::c_int {
    if pid <= 0 {
        return -1;
    }
    // SAFETY: a positive identifier is required; callers own the unreaped leader.
    unsafe { libc::kill(-pid, signal) }
}

/// Observe exit without reaping: retain the leader's identity until cleanup.
pub(crate) fn observed_exit(pid: u32) -> std::io::Result<Option<std::process::ExitStatus>> {
    use std::os::unix::process::ExitStatusExt;
    // SAFETY: zero is a valid initial siginfo_t representation for waitid.
    let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
    // SAFETY: waitid writes to a valid siginfo_t; WNOWAIT deliberately retains the child.
    let rc = unsafe {
        libc::waitid(
            libc::P_PID,
            pid,
            &mut info,
            libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
        )
    };
    if rc != 0 {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: successful waitid initializes the child fields of siginfo_t.
    if unsafe { info.si_pid() } == 0 {
        return Ok(None);
    }
    // SAFETY: the child-status field is initialized by successful waitid.
    let status = unsafe { info.si_status() };
    let raw = if info.si_code == libc::CLD_EXITED {
        status << 8
    } else {
        status
    };
    Ok(Some(std::process::ExitStatus::from_raw(raw)))
}

/// Terminate a child whose identity has not been reaped. Success requires
/// bounded verification, not merely observing the leader exit.
pub async fn terminate_child(child: &mut tokio::process::Child, grace_ms: u64) -> bool {
    let Some(pid) = child.id().and_then(|pid| checked_pid(pid).ok()) else {
        return false;
    };
    #[cfg(target_os = "linux")]
    let mut owned = OwnedProcesses::new(pid);
    terminate_owned(
        child,
        grace_ms,
        #[cfg(target_os = "linux")]
        &mut owned,
    )
    .await
}

/// The unreaped leader has exited and (on Linux) so has every observed
/// descendant. Without the descendant verifier, leader exit is all we can see.
fn tree_exited(pid: i32, #[cfg(target_os = "linux")] owned: &OwnedProcesses) -> bool {
    let Ok(pid) = u32::try_from(pid) else {
        return false;
    };
    let leader_exited = matches!(observed_exit(pid), Ok(Some(_)));
    #[cfg(target_os = "linux")]
    {
        leader_exited && owned.all_exited()
    }
    #[cfg(not(target_os = "linux"))]
    {
        leader_exited
    }
}

pub(crate) async fn terminate_owned(
    child: &mut tokio::process::Child,
    grace_ms: u64,
    #[cfg(target_os = "linux")] owned: &mut OwnedProcesses,
) -> bool {
    let Some(pid) = child.id().and_then(|pid| checked_pid(pid).ok()) else {
        return false;
    };
    #[cfg(target_os = "linux")]
    owned.refresh();
    kill_process_group(pid, libc::SIGTERM);
    #[cfg(target_os = "linux")]
    owned.signal(libc::SIGTERM);
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(grace_ms);
    while tokio::time::Instant::now() < deadline {
        #[cfg(target_os = "linux")]
        owned.refresh();
        // Leader exit alone is not proof of cleanup (#1608), but leader exit
        // plus every owned descendant exited is: stop waiting out the grace
        // and go straight to the (now no-op) escalation and verification.
        if tree_exited(
            pid,
            #[cfg(target_os = "linux")]
            owned,
        ) {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(TERMINATE_POLL_TICK_MS)).await;
    }
    // No try_wait before this signal: even an exited leader still pins its PGID.
    kill_process_group(pid, libc::SIGKILL);
    // The leader might not have been a group leader. Kill the owned Child too.
    let _ = child.start_kill();
    #[cfg(target_os = "linux")]
    let complete = {
        let deadline =
            tokio::time::Instant::now() + std::time::Duration::from_millis(grace_ms.max(200));
        loop {
            owned.refresh();
            owned.signal(libc::SIGKILL);
            if owned.all_exited() {
                break true;
            }
            if tokio::time::Instant::now() >= deadline {
                break false;
            }
            tokio::time::sleep(std::time::Duration::from_millis(TERMINATE_POLL_TICK_MS)).await;
        }
    };
    // Other Unix platforms still receive owned-group escalation, but lack this
    // Linux exact-descendant verifier: do not claim a verified tree cleanup.
    #[cfg(not(target_os = "linux"))]
    let complete = false;
    child.wait().await.is_ok() && complete
}

#[cfg(test)]
#[path = "process_tests.rs"]
mod tests;
