//! Observe Linux shell exit without reaping its group leader. The unreaped
//! child pins the numeric group identity until output drainage is complete.

#[cfg(target_os = "linux")]
pub(super) async fn exited(child: &mut tokio::process::Child) -> std::io::Result<()> {
    let pid = child
        .id()
        .ok_or_else(|| std::io::Error::other("shell already reaped"))?;
    loop {
        // SAFETY: zero is a valid initial siginfo_t; waitid initializes it.
        let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
        // WNOWAIT observes exit without releasing PID/PGID ownership.
        // SAFETY: this is our unreaped child and info is writable.
        let result = unsafe {
            libc::waitid(
                libc::P_PID,
                pid,
                &mut info,
                libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
            )
        };
        if result != 0 {
            let error = std::io::Error::last_os_error();
            if error.kind() != std::io::ErrorKind::Interrupted {
                return Err(error);
            }
        } else {
            // SAFETY: successful waitid initialized the siginfo union.
            if unsafe { info.si_pid() } != 0 {
                return Ok(());
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
}
