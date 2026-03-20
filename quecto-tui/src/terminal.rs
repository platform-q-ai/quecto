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
        let mut termios = unsafe { std::mem::zeroed::<libc::termios>() };
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
}
