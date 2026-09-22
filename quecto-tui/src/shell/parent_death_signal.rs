//! Linux `PR_SET_PDEATHSIG` on the owned harness (#2053), the same defence
//! in depth the harness arms on its own children: a TUI that dies outright
//! (SIGKILL, a crash) cannot run its ordinary exit, so the kernel sends
//! SIGTERM to the harness the TUI launched — the harness's own shutdown
//! path, the one pid this TUI owns, nothing else. Armed only when the exit
//! policy is kill-on-exit: a detached harness must outlive the TUI.
//!
//! Made race-safe by re-checking `getppid()` after arming: a parent that
//! died between `fork` and `prctl` would never trigger it, so the child
//! exits at once instead. Caveat: the kernel fires on the death of the
//! *thread* that forked, so callers spawn from a thread that lives as long
//! as the TUI does.

/// Arm the parent-death signal on `command`; `parent_pid` is the launcher's
/// pid captured before spawning, so the post-arm check compares against the
/// process that actually forked.
#[cfg(target_os = "linux")]
pub fn arm(command: &mut std::process::Command, parent_pid: u32) {
    use std::os::unix::process::CommandExt;
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
                // Arming failed: not fatal, the ordinary exit still ends it.
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
pub fn arm(_command: &mut std::process::Command, _parent_pid: u32) {}

#[cfg(test)]
#[path = "parent_death_signal_tests.rs"]
mod tests;
