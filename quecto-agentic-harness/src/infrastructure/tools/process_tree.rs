//! Local OS process-tree signalling helpers for owned subagent children.

use std::time::Duration;

/// How this harness owns a registered child process for cleanup purposes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ProcessOwner {
    /// Legacy/default: only the immediate pid is known to be owned.
    #[default]
    DirectPid,
    /// Unix local launch with `pgid == pid`; terminate the whole process group.
    LocalProcessGroup,
}

#[cfg(test)]
thread_local! {
    pub(super) static SIGNAL_LOG: std::cell::RefCell<Option<Vec<u32>>> = const { std::cell::RefCell::new(None) };
}

pub(crate) fn terminate_owned_process_tree(pid: u32, owner: ProcessOwner) {
    #[cfg(test)]
    if SIGNAL_LOG.with(|log| {
        let mut log = log.borrow_mut();
        if let Some(log) = log.as_mut() {
            log.push(pid);
            true
        } else {
            false
        }
    }) {
        return;
    }
    // Critical-path invariant (#1925): a lease can only name a child of ours
    // or a same-namespace descendant, never this harness or its parent (the
    // swarm coordinator when running inside a container).
    #[cfg(unix)]
    debug_assert!(
        pid != std::process::id() && i64::from(pid) != i64::from(parent_pid()),
        "signal lease named this harness or its parent (pid {pid})"
    );
    match owner {
        ProcessOwner::DirectPid => sigterm_pid(pid),
        ProcessOwner::LocalProcessGroup => terminate_local_process_group(pid),
    }
}

#[cfg(unix)]
fn parent_pid() -> libc::pid_t {
    // SAFETY: getppid has no preconditions and cannot fail.
    unsafe { libc::getppid() }
}

pub(crate) fn sigterm_pid(pid: u32) {
    #[cfg(unix)]
    if let Ok(pid) = i32::try_from(pid) {
        if pid > 0 {
            // SAFETY: FFI call to `libc::kill` with an owned pid and a constant signal.
            unsafe {
                libc::kill(pid, libc::SIGTERM);
            }
        }
    }
    #[cfg(not(unix))]
    let _ = pid;
}

/// Best-effort TERM/KILL of a local child process group. The child was spawned
/// with `process_group(0)`, so its pid is also the pgid. Reject zero and failed
/// conversions so cleanup can never accidentally signal the caller's group.
pub(crate) fn terminate_local_process_group(pid: u32) {
    #[cfg(unix)]
    if let Ok(pgid) = i32::try_from(pid) {
        if pgid <= 0 {
            return;
        }
        kill_process_group(pgid, libc::SIGTERM);
        std::thread::sleep(Duration::from_millis(50));
        if process_group_alive(pgid) {
            kill_process_group(pgid, libc::SIGKILL);
        }
    }
    #[cfg(not(unix))]
    {
        sigterm_pid(pid);
    }
}

#[cfg(unix)]
fn kill_process_group(pgid: i32, signal: libc::c_int) {
    if pgid <= 0 {
        return;
    }
    // SAFETY: negative pid intentionally targets the owned process group.
    unsafe {
        libc::kill(-pgid, signal);
    }
}

#[cfg(unix)]
fn process_group_alive(pgid: i32) -> bool {
    if pgid <= 0 {
        return false;
    }
    // SAFETY: signal 0 probes existence of the owned process group.
    unsafe { libc::kill(-pgid, 0) == 0 }
}

#[cfg(test)]
#[path = "process_tree_tests.rs"]
mod tests;
