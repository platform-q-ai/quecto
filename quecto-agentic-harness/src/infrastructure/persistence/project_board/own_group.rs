//! The board's git and gh children each lead a session of their own
//! (`own_session_dying_with_us`). This watches such a child without
//! reaping it, so its group stays its own, and ends that whole group.
use std::thread;
use std::time::{Duration, Instant};

/// How long a group asked to stop with SIGTERM has before SIGKILL.
pub(super) const TERM_GRACE: Duration = Duration::from_millis(500);
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
    // SAFETY: pid is our own unreaped child and info is writable.
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

/// Ends the process group `leader` leads (`setsid` made the child its
/// leader; it is not yet reaped, so the group is still its own): SIGTERM
/// first, so git can remove a ref lock as it goes, then SIGKILL to
/// whatever is left once the leader has exited or [`TERM_GRACE`] passed.
pub(super) fn end_group(leader: u32) {
    match libc::pid_t::try_from(leader) {
        Ok(group) if group > 0 => {
            signal_group(group, libc::SIGTERM);
            let by = Instant::now() + TERM_GRACE;
            while Instant::now() < by && matches!(exited_unreaped(leader), Ok(false)) {
                thread::sleep(GRACE_POLL);
            }
            signal_group(group, libc::SIGKILL);
        }
        // The kernel gives no child a pid that is not a positive pid_t.
        _ => debug_assert!(false, "a child's pid is a positive pid_t: {leader}"),
    }
}

#[cfg(test)]
#[path = "own_group_tests.rs"]
pub(super) mod tests;
