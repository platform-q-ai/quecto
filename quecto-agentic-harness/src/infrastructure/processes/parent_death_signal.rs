//! Linux `PR_SET_PDEATHSIG` as defence in depth (#1935).
//!
//! The portable parent-loss contract is the launch-bound control connection;
//! this only shortens the window on Linux when the parent is killed outright.
//! It is armed in `pre_exec` and made race-safe by re-checking `getppid()`
//! after arming: if the parent already died between `fork` and `prctl`, the
//! signal would never arrive, so the child exits immediately instead.
//!
//! The signal is always SIGTERM ([`DeathSignal`]), so a child can clean up
//! as it goes: git removes its ref locks and ends the children it waits
//! on. It reaches the child alone: a grandchild the child leaves running
//! (an ssh, a credential helper it does not wait on) is not signalled.
//!
//! When the kernel refuses to arm it, the spawn fails: no child runs
//! unarmed, for [`arm`] and [`arm_in_child`] alike.
//!
//! Caveat (documented, not a contract): the kernel delivers the signal when
//! the *thread* that forked exits, not only the process. Launches happen on
//! long-lived tokio worker threads, or on a thread that waits for the child
//! ([`own_session_dying_with_us`]), so this is acceptable.

/// The one signal a child is sent when its launcher dies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeathSignal {
    /// SIGTERM: the child may clean up as it goes.
    Terminate,
}

impl DeathSignal {
    #[cfg(target_os = "linux")]
    fn number(self) -> libc::c_ulong {
        match self {
            Self::Terminate => libc::SIGTERM as libc::c_ulong,
        }
    }
}

/// Arm the parent-death signal on `command`. `parent_pid` is the launcher's
/// pid captured before spawning, so the post-arm check compares against the
/// process that actually forked.
#[cfg(target_os = "linux")]
pub fn arm(command: &mut tokio::process::Command, parent_pid: u32) {
    let parent = libc::pid_t::try_from(parent_pid).unwrap_or(0);
    // The closure runs in the forked child before exec.
    // SAFETY: it only calls arm_in_child, whose own contract that is.
    unsafe {
        command.pre_exec(move || arm_in_child(parent, DeathSignal::Terminate));
    }
}

#[cfg(not(target_os = "linux"))]
pub fn arm(_command: &mut tokio::process::Command, _parent_pid: u32) {}

/// Arms `signal` from inside a forked child before it execs. If `parent`
/// already died, the signal would never come, so the child exits at once.
/// Only on Linux; elsewhere nothing is armed, and a child killed with its
/// launcher outlives it until its own deadline or exit.
///
/// # Errors
/// A parent id that is not a process's, or the kernel refused to arm it.
///
/// # Safety
/// Call it only in a forked child before exec (a `pre_exec` closure): it
/// may `_exit` the calling process, and it calls only async-signal-safe
/// functions (`prctl`, `getppid`, `_exit`) that touch no memory of the
/// parent and allocate nothing.
#[cfg(target_os = "linux")]
pub unsafe fn arm_in_child(parent: libc::pid_t, signal: DeathSignal) -> std::io::Result<()> {
    match parent > 0 {
        true => {}
        false => return Err(std::io::ErrorKind::InvalidInput.into()),
    }
    // SAFETY: in a forked child before exec, per this function's contract.
    unsafe {
        if libc::prctl(libc::PR_SET_PDEATHSIG, signal.number(), 0, 0, 0) != 0 {
            return Err(std::io::Error::last_os_error());
        }
        if libc::getppid() != parent {
            libc::_exit(1);
        }
    }
    Ok(())
}

/// Off Linux nothing is armed.
///
/// # Errors
/// Never.
///
/// # Safety
/// As on Linux: only in a forked child before exec.
#[cfg(not(target_os = "linux"))]
pub unsafe fn arm_in_child(_parent: libc::pid_t, _signal: DeathSignal) -> std::io::Result<()> {
    Ok(())
}

/// Makes `command`'s child lead a session of its own (`setsid`: nothing it
/// starts reaches this process's terminal, and its group can be ended as
/// one) that is sent SIGTERM when this process dies (on Linux), so a
/// quecto killed outright leaves no child behind. The calling thread must
/// wait for the child (see the caveat above).
///
/// # Panics
/// A process id that is not positive, which the kernel never gives.
pub fn own_session_dying_with_us(command: &mut std::process::Command) {
    let parent = libc::pid_t::try_from(std::process::id())
        .ok()
        .filter(|pid| *pid > 0)
        .expect("a positive process id");
    // SAFETY: the closure only calls setsid and arm_in_child (async-signal-safe).
    unsafe {
        std::os::unix::process::CommandExt::pre_exec(command, move || match libc::setsid() {
            -1 => Err(std::io::Error::last_os_error()),
            _ => arm_in_child(parent, DeathSignal::Terminate),
        });
    }
}

#[cfg(test)]
#[path = "parent_death_signal_tests.rs"]
pub(crate) mod tests;
