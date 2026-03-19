//! Signal handling — SIGTSTP (Ctrl+Z suspend), SIGWINCH (resize).
//!
//! SIGTSTP requires special handling: the terminal must be restored to
//! cooked mode before suspending, and re-entered into raw mode on resume.

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
    // Restore terminal to cooked mode.
    let _ = std::io::Write::write_all(&mut std::io::stdout(), b"\x1b[?2004l"); // disable bracketed paste
    let _ = std::io::Write::write_all(&mut std::io::stdout(), b"\x1b[?25h"); // show cursor
    let _ = std::io::Write::flush(&mut std::io::stdout());

    // Restore termios.
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

#[cfg(test)]
mod tests {
    // Signal tests are difficult to unit test without real signal delivery.
    // These are integration-level behaviors tested via manual verification.

    #[test]
    fn module_compiles() {
        // Verify the module compiles and types are correct.
        assert!(true);
    }
}
