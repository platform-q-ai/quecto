//! Signal handling — SIGTSTP (Ctrl+Z suspend), SIGWINCH (resize), and the
//! termination signals (SIGHUP, SIGTERM, SIGINT), which take the ordinary
//! exit path (#2053) instead of their default disposition.
//!
//! SIGTSTP requires special handling: the terminal must be restored to
//! cooked mode before suspending, and re-entered into raw mode on resume.
//!
//! For outgoing signal management (SIGTERM/SIGKILL to the owned harness
//! leader, one pid only), see the [`crate::shell::process`] module.

/// Suspend the process (Ctrl+Z behavior).
///
/// 1. Exit raw mode (restore termios)
/// 2. Show cursor
/// 3. Send SIGTSTP to self
/// 4. On resume: re-enter raw mode, hide cursor
///
/// The caller is responsible for calling `terminal.enter_raw_mode()` and
/// `terminal.hide_cursor()` after this function returns (which happens
/// when the process resumes from suspend).
pub fn suspend() {
    // Leave alternate screen and restore terminal for suspend.
    let _ = std::io::Write::write_all(&mut std::io::stdout(), b"\x1b[?1049l\x1b[?2004l\x1b[?25h");
    let _ = std::io::Write::flush(&mut std::io::stdout());

    // Restore termios.
    // SAFETY: termios calls operate on stdin fd 0; return values are checked before using the struct.
    unsafe {
        let fd = 0; // stdin
        let mut termios: libc::termios = std::mem::zeroed();
        if libc::tcgetattr(fd, &mut termios) == 0 {
            termios.c_lflag |= libc::ICANON | libc::ECHO | libc::ISIG;
            termios.c_iflag |= libc::ICRNL;
            termios.c_oflag |= libc::OPOST;
            libc::tcsetattr(fd, libc::TCSANOW, &termios);
        }
    }

    // Send SIGTSTP to self — process suspends here.
    // SAFETY: raising SIGTSTP for the current process has no memory-safety preconditions.
    unsafe {
        libc::raise(libc::SIGTSTP);
    }

    // Execution resumes here after `fg` or SIGCONT.
    // The caller should re-enter raw mode and hide cursor.
}

/// Register a SIGWINCH handler that sends on a tokio channel.
///
/// Returns a receiver that fires whenever the terminal is resized.
/// The actual dimension query happens in the caller after receiving the signal.
pub async fn sigwinch_stream() -> tokio::sync::mpsc::Receiver<()> {
    let (tx, rx) = tokio::sync::mpsc::channel(1);

    tokio::spawn(async move {
        let mut sig =
            match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::window_change()) {
                Ok(s) => s,
                Err(_) => return,
            };
        loop {
            sig.recv().await;
            if tx.send(()).await.is_err() {
                break;
            }
        }
    });

    rx
}

/// A signal asking the TUI to leave: the terminal went away (SIGHUP), or
/// someone asked from outside (SIGTERM, an external SIGINT — Ctrl+C in raw
/// mode is a key, never a signal). Each takes the same path as Ctrl+D
/// (#2053): persist, then end the owned harness within the leader budget.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerminationSignal {
    Hangup,
    Terminate,
    Interrupt,
}

impl TerminationSignal {
    pub fn name(self) -> &'static str {
        match self {
            Self::Hangup => "SIGHUP",
            Self::Terminate => "SIGTERM",
            Self::Interrupt => "SIGINT",
        }
    }
}

/// Register for SIGHUP, SIGTERM and SIGINT; the receiver fires once per
/// signal (extra ones while one is pending are folded — they ask for the
/// same thing). Registered from the moment this is called, so a signal in
/// the startup window is kept for the caller, never lost to the default
/// disposition. A kind that cannot be registered is left at its default.
pub fn termination_stream() -> tokio::sync::mpsc::Receiver<TerminationSignal> {
    use tokio::signal::unix::{SignalKind, signal};
    let (tx, rx) = tokio::sync::mpsc::channel(1);
    for (kind, which) in [
        (SignalKind::hangup(), TerminationSignal::Hangup),
        (SignalKind::terminate(), TerminationSignal::Terminate),
        (SignalKind::interrupt(), TerminationSignal::Interrupt),
    ] {
        let Ok(mut sig) = signal(kind) else {
            continue;
        };
        let tx = tx.clone();
        tokio::spawn(async move {
            while sig.recv().await.is_some() {
                if tx.try_send(which).is_err() && tx.is_closed() {
                    break;
                }
            }
        });
    }
    rx
}

#[cfg(test)]
#[path = "signals_tests.rs"]
mod tests;
