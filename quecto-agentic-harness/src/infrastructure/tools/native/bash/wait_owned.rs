//! Observe Unix shell exit without reaping its group leader. The unreaped
//! child pins the numeric group identity until output drainage is complete.

#[cfg(unix)]
pub(super) async fn exited(child: &mut tokio::process::Child) -> std::io::Result<()> {
    let pid = child
        .id()
        .ok_or_else(|| std::io::Error::other("shell already reaped"))?;
    // Only a plain `bool` crosses the await: the `siginfo_t` the poll uses
    // lives and dies inside `has_exited`, a synchronous call, so this future
    // stays `Send` where `siginfo_t` is not (macOS, #2237).
    while !has_exited(pid)? {
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    Ok(())
}

/// The `waitid` options of one poll: observe an exit (`WEXITED`) without
/// blocking (`WNOHANG`) and without releasing the PID/PGID ownership
/// (`WNOWAIT`). The flags are disjoint bits, so `|` and `^` combine them
/// alike; one named constant keeps the combination out of the poll itself.
#[cfg(unix)]
const OBSERVE_EXIT_UNREAPED: libc::c_int = libc::WEXITED | libc::WNOHANG | libc::WNOWAIT;

/// Whether our unreaped child `pid` has exited, observed without reaping it
/// (`WNOWAIT`) and without blocking (`WNOHANG`). An interrupted call is
/// "not yet": the caller polls again.
#[cfg(unix)]
fn has_exited(pid: u32) -> std::io::Result<bool> {
    assert!(pid != 0, "a live child never has pid 0");
    // SAFETY: zero is a valid initial siginfo_t; waitid initializes it.
    let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
    // SAFETY: this is our unreaped child and info is writable.
    let result = unsafe { libc::waitid(libc::P_PID, pid, &mut info, OBSERVE_EXIT_UNREAPED) };
    poll_outcome(match result {
        // SAFETY: successful waitid initialized the siginfo union.
        0 => Ok(unsafe { info.si_pid() }),
        _ => Err(std::io::Error::last_os_error()),
    })
}

/// How one poll reads: `Ok` with the `si_pid` a successful `waitid` wrote
/// (zero while the child runs), or the call's error. An interrupted call
/// is "not yet"; any other error is reported.
#[cfg(unix)]
fn poll_outcome(waited: std::io::Result<libc::pid_t>) -> std::io::Result<bool> {
    match waited {
        Ok(exited_pid) => Ok(exited_pid != 0),
        Err(error) => match error.kind() {
            std::io::ErrorKind::Interrupted => Ok(false),
            _ => Err(error),
        },
    }
}

#[cfg(test)]
#[path = "wait_owned_tests.rs"]
mod tests;
