//! Linux `PR_SET_PDEATHSIG` as defence in depth (#1935).
//!
//! The portable parent-loss contract is the launch-bound control connection;
//! this only shortens the window on Linux when the parent is killed outright.
//! It is armed in `pre_exec` and made race-safe by re-checking `getppid()`
//! after arming: if the parent already died between `fork` and `prctl`, the
//! signal would never arrive, so the child exits immediately instead.
//!
//! Caveat (documented, not a contract): the kernel delivers the signal when
//! the *thread* that forked exits, not only the process. Launches happen on
//! long-lived tokio worker threads, so this is acceptable as belt-and-braces
//! only.

/// Arm the parent-death signal on `command`. `parent_pid` is the launcher's
/// pid captured before spawning, so the post-arm check compares against the
/// process that actually forked.
#[cfg(target_os = "linux")]
pub fn arm(command: &mut tokio::process::Command, parent_pid: u32) {
    let expected = libc::pid_t::try_from(parent_pid).unwrap_or(0);
    // The closure runs in the forked child before exec and only calls
    // async-signal-safe libc functions (prctl, getppid, _exit).
    // SAFETY: `pre_exec` requires exactly that discipline, which holds.
    unsafe {
        command.pre_exec(move || {
            if libc::prctl(
                libc::PR_SET_PDEATHSIG,
                libc::SIGTERM as libc::c_ulong,
                0,
                0,
                0,
            ) != 0
            {
                // Arming failed: not fatal, the protocol contract still holds.
                return Ok(());
            }
            if expected > 0 && libc::getppid() != expected {
                // The parent died before the signal was armed.
                libc::_exit(1);
            }
            Ok(())
        });
    }
}

#[cfg(not(target_os = "linux"))]
pub fn arm(_command: &mut tokio::process::Command, _parent_pid: u32) {}

#[cfg(test)]
#[path = "parent_death_signal_tests.rs"]
mod tests;
