//! Linux `PR_SET_PDEATHSIG`: a child this harness spawns is signalled when
//! its launcher dies outright (#1935, #2501). For tool children it backs
//! the portable parent-loss contract, the launch-bound control connection;
//! for the board's git and gh children it is what ends them with quecto.
//! Arming is fail-closed: a child it cannot be armed for is never run.
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

/// `pid` as the parent id [`arm_in_child`] takes: itself when it is a
/// positive `pid_t`, else 0, which [`arm_in_child`] refuses, so the spawn
/// fails rather than run a child unarmed.
fn parent_id(pid: u32) -> libc::pid_t {
    match libc::pid_t::try_from(pid) {
        Ok(parent) if parent > 0 => parent,
        _ => 0,
    }
}

/// Arm the parent-death signal on `command`. `parent_pid` is the launcher's
/// pid captured before spawning, so the post-arm check compares against the
/// process that actually forked.
#[cfg(target_os = "linux")]
pub fn arm(command: &mut tokio::process::Command, parent_pid: u32) {
    let parent = parent_id(parent_pid);
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
        match libc::prctl(libc::PR_SET_PDEATHSIG, signal.number(), 0, 0, 0) {
            0 => {}
            _ => return Err(std::io::Error::last_os_error()),
        }
        match libc::getppid() == parent {
            true => Ok(()),
            false => libc::_exit(1),
        }
    }
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

/// What [`own_session_dying_with_us`] does before the child execs, for a
/// failed spawn to name: a session, and on Linux the death signal (a
/// refusal to arm fails the spawn).
#[cfg(target_os = "linux")]
pub const SPAWN_ARMS: &str = "making its session, arming the death signal";
/// What [`own_session_dying_with_us`] does before the child execs, for a
/// failed spawn to name: off Linux, a session alone; nothing is armed.
#[cfg(not(target_os = "linux"))]
pub const SPAWN_ARMS: &str = "making its session";

/// Makes `command`'s child lead a session of its own (`setsid`: nothing it
/// starts reaches this process's terminal, and its group can be ended as
/// one) that is sent SIGTERM when this process dies (on Linux), so a
/// quecto killed outright leaves no child behind. The calling thread must
/// wait for the child (see the caveat above). A process id that is no
/// process's is refused by [`arm_in_child`], so the spawn fails.
pub fn own_session_dying_with_us(command: &mut std::process::Command) {
    let parent = parent_id(std::process::id());
    // SAFETY: the closure only calls setsid and arm_in_child (async-signal-safe).
    unsafe {
        std::os::unix::process::CommandExt::pre_exec(command, move || match libc::setsid() {
            session if session > 0 => arm_in_child(parent, DeathSignal::Terminate),
            _ => Err(std::io::Error::last_os_error()),
        });
    }
}

#[cfg(test)]
#[path = "parent_death_signal_tests.rs"]
pub(crate) mod tests;
