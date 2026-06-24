//! Raw terminal control — enter/exit raw mode, cursor, screen, resize.
//!
//! Pure ANSI escape codes. No crossterm, no termion, no external crate.
//! Uses libc directly for termios manipulation on Unix.

use std::io::Write;
use std::os::unix::io::AsRawFd;

/// Saved terminal state for restoration on exit.
struct SavedTermios {
    original: libc::termios,
}

/// Terminal handle — manages raw mode and provides ANSI helpers.
pub struct Terminal {
    saved: Option<SavedTermios>,
    /// Current terminal width.
    pub width: usize,
    /// Current terminal height.
    pub height: usize,
}

impl Default for Terminal {
    fn default() -> Self {
        Self::new()
    }
}

impl Terminal {
    /// Create a new terminal handle and query dimensions.
    pub fn new() -> Self {
        let (w, h) = get_terminal_size();
        Self {
            saved: None,
            width: w,
            height: h,
        }
    }

    /// Enter raw mode: disable echo, line buffering, and signal processing.
    pub fn enter_raw_mode(&mut self) {
        if self.saved.is_some() {
            return; // already in raw mode
        }

        let fd = std::io::stdin().as_raw_fd();
        // SAFETY: `libc::termios` is a plain C struct; zeroed memory is an acceptable buffer for `tcgetattr` to fill.
        let mut termios = unsafe { std::mem::zeroed::<libc::termios>() };
        // SAFETY: `fd` is stdin and `termios` points to valid writable memory; return code is checked.
        let rc = unsafe { libc::tcgetattr(fd, &mut termios) };
        if rc != 0 {
            // Not a TTY or error — don't enter raw mode.
            return;
        }

        let saved = SavedTermios { original: termios };

        // Disable canonical mode, echo, and signal chars
        termios.c_lflag &= !(libc::ICANON | libc::ECHO | libc::ISIG | libc::IEXTEN);
        // Disable input processing (CR → NL, flow control)
        termios.c_iflag &= !(libc::IXON | libc::ICRNL | libc::BRKINT | libc::INPCK | libc::ISTRIP);
        // Disable output processing
        termios.c_oflag &= !libc::OPOST;
        // Read byte-at-a-time with no timeout
        termios.c_cc[libc::VMIN] = 1;
        termios.c_cc[libc::VTIME] = 0;

        // SAFETY: `fd` is stdin and `termios` was initialized by `tcgetattr` then modified with valid flags.
        unsafe {
            libc::tcsetattr(fd, libc::TCSAFLUSH, &termios);
        }

        self.saved = Some(saved);

        // Enter alternate screen buffer, enable bracketed paste, and enable
        // SGR mouse reporting (scroll wheel + click/drag events).
        // The alternate buffer prevents scrollback interference which
        // causes border duplication during streaming (#479).
        // Mouse: 1000 = basic events, 1002 = button event tracking (drag),
        //        1006 = SGR extended format.
        // 1002 supersedes 1000 (adds drag events for text selection #528).
        // Both are enabled for terminal compatibility.
        let _ =
            std::io::stdout().write_all(b"\x1b[?1049h\x1b[?2004h\x1b[?1000h\x1b[?1002h\x1b[?1006h");
        let _ = std::io::stdout().flush();
    }

    /// Exit raw mode and restore the original terminal state.
    ///
    /// Sends terminal reset escape sequences BEFORE restoring termios
    /// so they're written while we still have raw output mode.
    pub fn exit_raw_mode(&mut self) {
        if let Some(saved) = self.saved.take() {
            // Leave alt screen FIRST — it restores the main buffer's saved
            // cursor/attributes. Then reset everything on the main buffer.
            let _ = std::io::stdout().write_all(
                concat!(
                    "\x1b[?1006l", // Disable SGR mouse reporting
                    "\x1b[?1002l", // Disable button event tracking (drag)
                    "\x1b[?1000l", // Disable basic mouse reporting
                    "\x1b[?1049l", // Leave alternate screen buffer (restores main)
                    "\x1b[?2004l", // Disable bracketed paste
                    "\x1b[?25h",   // Show cursor
                    "\x1b[0m",     // Reset all SGR attributes
                    "\x1b[>4;0m",  // Reset modifyOtherKeys (xterm/tmux)
                    "\x1b[<u",     // Pop Kitty keyboard protocol flags
                )
                .as_bytes(),
            );
            let _ = std::io::stdout().flush();

            // Restore original termios settings (cooked mode, echo, etc.).
            let fd = std::io::stdin().as_raw_fd();
            // SAFETY: `saved.original` is a termios value previously returned by `tcgetattr` for stdin.
            unsafe {
                libc::tcsetattr(fd, libc::TCSANOW, &saved.original);
            }
        }
    }

    /// Re-query terminal dimensions (call after SIGWINCH).
    pub fn refresh_size(&mut self) {
        let (w, h) = get_terminal_size();
        self.width = w;
        self.height = h;
    }

    /// Hide the cursor.
    pub fn hide_cursor(&self) {
        let _ = std::io::stdout().write_all(b"\x1b[?25l");
        let _ = std::io::stdout().flush();
    }

    /// Show the cursor.
    pub fn show_cursor(&self) {
        let _ = std::io::stdout().write_all(b"\x1b[?25h");
        let _ = std::io::stdout().flush();
    }

    /// Write raw bytes to stdout.
    pub fn write(&self, data: &[u8]) {
        let _ = std::io::stdout().write_all(data);
        let _ = std::io::stdout().flush();
    }

    /// Write a string to stdout.
    pub fn write_str(&self, s: &str) {
        self.write(s.as_bytes());
    }

    /// Clear the entire screen and move cursor to (0,0).
    pub fn clear_screen(&self) {
        self.write_str("\x1b[2J\x1b[H");
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        self.exit_raw_mode();
        self.show_cursor();
    }
}

/// Query the terminal size using ioctl.
fn get_terminal_size() -> (usize, usize) {
    let mut ws = libc::winsize {
        ws_row: 0,
        ws_col: 0,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };

    // SAFETY: `ws` points to valid writable memory for the TIOCGWINSZ ioctl; return code is checked.
    let result = unsafe { libc::ioctl(libc::STDOUT_FILENO, libc::TIOCGWINSZ, &mut ws) };

    if result == 0 && ws.ws_col > 0 && ws.ws_row > 0 {
        (ws.ws_col as usize, ws.ws_row as usize)
    } else {
        // Fallback
        (80, 24)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_size_is_reasonable() {
        let (w, h) = get_terminal_size();
        assert!(w > 0);
        assert!(h > 0);
    }

    #[test]
    fn terminal_new_has_dimensions() {
        let term = Terminal::new();
        assert!(term.width > 0);
        assert!(term.height > 0);
    }

    #[test]
    fn terminal_default_matches_new() {
        let term = Terminal::default();
        assert!(term.width > 0);
        assert!(term.height > 0);
    }

    #[test]
    fn refresh_size_keeps_positive_dimensions() {
        let mut term = Terminal::new();
        term.width = 1;
        term.height = 1;
        term.refresh_size();
        assert!(term.width > 0);
        assert!(term.height > 0);
    }

    #[test]
    fn write_helpers_do_not_panic() {
        let term = Terminal::new();
        // Benign byte writes — no screen-altering escapes.
        term.write(b"");
        term.write_str("");
        // hide+show cursor is a visual no-op pair.
        term.hide_cursor();
        term.show_cursor();
    }

    #[test]
    fn exit_raw_mode_without_enter_is_noop() {
        let mut term = Terminal::new();
        // No saved termios → exit is a no-op and must not panic.
        term.exit_raw_mode();
        assert!(term.saved.is_none());
    }

    // ── enter_raw_mode behavior ─────────────────────────────────────

    #[test]
    fn enter_raw_mode_is_idempotent() {
        // Calling enter_raw_mode twice should not panic and should not
        // overwrite the saved state (the second call is a no-op).
        let mut term = Terminal::new();
        term.enter_raw_mode();
        let first_saved = term.saved.is_some();
        term.enter_raw_mode(); // second call — should be a no-op
        assert_eq!(
            term.saved.is_some(),
            first_saved,
            "second enter_raw_mode should not change saved state"
        );
        term.exit_raw_mode();
    }

    #[test]
    fn enter_then_exit_restores_no_saved_state() {
        let mut term = Terminal::new();
        term.enter_raw_mode();
        // In a test env (non-TTY) enter_raw_mode may not save anything,
        // but exit must still be safe.
        term.exit_raw_mode();
        assert!(term.saved.is_none());
    }

    #[test]
    fn drop_after_enter_raw_mode_does_not_panic() {
        // Drop calls exit_raw_mode + show_cursor; must not panic.
        let mut term = Terminal::new();
        term.enter_raw_mode();
        drop(term);
    }

    // ── write helpers ───────────────────────────────────────────────

    #[test]
    fn write_str_emits_bytes() {
        let term = Terminal::new();
        // Just verify no panic on a benign string.
        term.write_str("test");
    }

    #[test]
    fn clear_screen_does_not_panic() {
        let term = Terminal::new();
        term.clear_screen();
    }

    // ── refresh_size ────────────────────────────────────────────────

    #[test]
    fn refresh_size_updates_both_dimensions() {
        let mut term = Terminal::new();
        term.width = 1;
        term.height = 1;
        term.refresh_size();
        // After refresh, dimensions should match the real terminal
        // (or the 80×24 fallback in a non-TTY environment).
        assert!(term.width >= 80, "width should be at least the fallback");
        assert!(term.height >= 24, "height should be at least the fallback");
    }

    #[test]
    fn manual_dimension_override_persists_until_refresh() {
        let mut term = Terminal::new();
        term.width = 200;
        term.height = 50;
        assert_eq!(term.width, 200);
        assert_eq!(term.height, 50);
        term.refresh_size();
        // After refresh the real/fallback size takes over.
        assert!(term.width != 200 || term.height != 50);
    }
}
