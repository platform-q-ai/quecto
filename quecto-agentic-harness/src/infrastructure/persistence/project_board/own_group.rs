//! The board's git and gh children each lead a session of their own
//! (`own_session_dying_with_us`). This watches such a child without
//! reaping it, so its group stays its own, and ends that whole group.
use std::thread;
use std::time::{Duration, Instant};

/// How long a group asked to stop with SIGTERM has before SIGKILL.
const TERM_GRACE: Duration = Duration::from_millis(500);
/// How often the leader is looked at during [`TERM_GRACE`].
const GRACE_POLL: Duration = Duration::from_millis(5);

/// Whether our unreaped child `pid` has exited, observed without reaping
/// it (`WNOWAIT`) or blocking (`WNOHANG`). An interrupted call is "not
/// yet".
pub(super) fn exited_unreaped(pid: u32) -> std::io::Result<bool> {
    let pid = libc::id_t::from(pid);
    // SAFETY: zero is a valid initial siginfo_t; waitid initializes it.
    let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
    let options = libc::WEXITED | libc::WNOHANG | libc::WNOWAIT;
    // Any pid is safe to ask about: one not our unreaped child answers ECHILD.
    // SAFETY: info is writable.
    match unsafe { libc::waitid(libc::P_PID, pid, &mut info, options) } {
        // SAFETY: a successful waitid initialized the siginfo union.
        0 => Ok(unsafe { info.si_pid() } != 0),
        _ => {
            let error = std::io::Error::last_os_error();
            match error.kind() {
                std::io::ErrorKind::Interrupted => Ok(false),
                _ => Err(error),
            }
        }
    }
}

/// Sends `signal` to every process in `group`.
fn signal_group(group: libc::pid_t, signal: libc::c_int) {
    debug_assert!(group > 0, "a group id is positive");
    // SAFETY: kill(2) takes plain integers and touches no memory.
    unsafe {
        libc::kill(-group, signal);
    }
}

/// What [`end_group`] did. A caller must not go on to wait for a leader
/// that is no longer its own.
#[must_use]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Ended {
    /// The group was sent SIGTERM, then SIGKILL.
    Signalled,
    /// Nothing more was signalled: `leader` was no longer our unreaped
    /// child (ECHILD: something else reaped it), so its pid, and so its
    /// group's, may already be another's.
    NotOurs,
}

/// Ends the process group `leader` leads (`setsid` made the child its
/// leader; while it is our unreaped child the group is still its own):
/// SIGTERM first, so git can remove a ref lock as it goes, then SIGKILL to
/// whatever is left, at once when the leader has already exited, else once
/// it exits or [`TERM_GRACE`] passed. Each signal is sent only while the
/// leader is seen to be still ours; a reused group is never signalled.
pub(super) fn end_group(leader: u32) -> Ended {
    let group = match libc::pid_t::try_from(leader) {
        Ok(group) if group > 0 => group,
        // The kernel gives no child a pid that is not a positive pid_t.
        _ => {
            debug_assert!(false, "a child's pid is a positive pid_t: {leader}");
            return Ended::NotOurs;
        }
    };
    let Ok(exited) = exited_unreaped(leader) else {
        return Ended::NotOurs;
    };
    signal_group(group, libc::SIGTERM);
    match exited {
        true => {}
        false => match grace(leader) {
            Ok(()) => {}
            Err(_) => return Ended::NotOurs,
        },
    }
    signal_group(group, libc::SIGKILL);
    Ended::Signalled
}

/// Waits until our unreaped child `leader` exits or [`TERM_GRACE`] passes.
///
/// # Errors
/// `leader` is no longer our unreaped child.
fn grace(leader: u32) -> std::io::Result<()> {
    let by = Instant::now() + TERM_GRACE;
    while !exited_unreaped(leader)? && Instant::now() < by {
        thread::sleep(GRACE_POLL);
    }
    Ok(())
}

#[cfg(test)]
#[path = "own_group_tests.rs"]
pub(super) mod tests;
