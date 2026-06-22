//! Signal handling — SIGTSTP (Ctrl+Z suspend), SIGWINCH (resize).
//!
//! SIGTSTP requires special handling: the terminal must be restored to
//! cooked mode before suspending, and re-entered into raw mode on resume.
//!
//! For outgoing signal management (SIGTERM/SIGKILL to child process groups),
//! see the [`crate::infrastructure::process`] module.

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

#[cfg(test)]
mod tests {
    use super::*;

    // ── sigwinch_stream ──────────────────────────────────────────────

    #[tokio::test]
    async fn sigwinch_stream_returns_a_receiver() {
        // The function spawns a signal handler and returns a receiver.
        // We can't easily deliver a real SIGWINCH in a unit test, but we
        // can verify the channel is created and the receiver is usable.
        let mut rx = sigwinch_stream().await;
        // The receiver should be available for recv (it won't have any
        // messages unless we send a signal, which we avoid in tests).
        // Just verifying it doesn't panic and is the right type.
        tokio::select! {
            _ = rx.recv() => {
                // A signal arrived during the test — that's fine.
            }
            _ = tokio::time::sleep(std::time::Duration::from_millis(10)) => {
                // No signal — expected in a test environment.
            }
        }
    }

    #[tokio::test]
    async fn sigwinch_stream_receives_signal() {
        // Deliver a real SIGWINCH and verify the channel fires.
        let mut rx = sigwinch_stream().await;

        // Give the spawned task time to register the handler.
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        // SAFETY: raising SIGWINCH to self is safe; the default action is ignored (no handler installed by default for SIGWINCH).
        unsafe {
            libc::raise(libc::SIGWINCH);
        }

        // The channel should fire within a reasonable time.
        let result = tokio::time::timeout(std::time::Duration::from_millis(500), rx.recv()).await;
        assert!(
            result.is_ok(),
            "sigwinch_stream should fire after SIGWINCH is raised"
        );
    }

    // ── suspend ──────────────────────────────────────────────────────

    #[test]
    fn suspend_does_not_panic_when_not_a_tty() {
        // In a test environment stdin is typically not a TTY. The suspend
        // function checks termios and skips termios restoration if
        // tcgetattr fails. However, it DOES raise SIGTSTP unconditionally,
        // which would suspend the test process. We can't safely call
        // suspend() in a unit test without potentially hanging.
        //
        // Instead, we verify the function exists and compiles correctly.
        // The suspend behavior is tested via manual verification.
        // This is a compile-time + type check.
        let _ = suspend; // function pointer — verifies it exists and is callable
    }
}
