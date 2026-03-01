//! REPL progress renderer — displays live agent activity on TTY stderr.
//!
//! When the agent is processing (thinking or executing tools), the terminal
//! would otherwise appear completely hung. This module provides:
//!
//! - [`ProgressRenderer`] — renders spinner frames and tool activity lines to
//!   a `Write` impl (typically `std::io::Stderr`). Gated on `is_tty`: if the
//!   output is not a terminal, all rendering is silently skipped.
//!
//! - [`make_channel_callback`] — creates a [`ProgressCallback`] that sends
//!   events over a `std::sync::mpsc::Sender` for testing or background threads.
//!
//! - [`spawn_spinner_thread`] — spawns a background OS thread that receives
//!   [`AgentProgressEvent`]s from a channel and drives `ProgressRenderer` at
//!   ~12fps, giving the terminal a live animated spinner.
//!
//! ## Design constraints
//!
//! - **No external crates** — pure ANSI escape codes, no `indicatif`/`crossterm`.
//! - **stderr only** — progress never mixes with the captured stdout stream.
//! - **Non-TTY no-op** — `is_tty = false` produces zero output (safe for pipes,
//!   BDD test harness, CI).
//! - **Callback is sync** — the callback just sends on a channel, never blocks.
//! - **Windows fallback** — ANSI escapes work on Windows 10+ (VT mode). On
//!   older terminals the `\r\x1b[K` erase will degrade gracefully.

use std::io::Write;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::domain::agent::{AgentProgressEvent, ProgressCallback};

// ---------------------------------------------------------------------------
// Spinner frames (braille dot pattern, 10 frames)
// ---------------------------------------------------------------------------

/// Spinner animation frames (braille dot pattern).
///
/// These Unicode characters are single-width on all modern terminals and
/// render cleanly at any font size. The sequence produces a smooth rotation.
pub const SPINNER_FRAMES: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// Tick interval for the spinner animation thread (≈12fps).
const TICK_INTERVAL: Duration = Duration::from_millis(80);

// ANSI escape: carriage return + erase to end of line.
// Moves the cursor to column 0 and clears everything after it.
const ERASE_LINE: &str = "\r\x1b[K";

// ---------------------------------------------------------------------------
// ProgressRenderer
// ---------------------------------------------------------------------------

/// Renders live agent progress to a `Write` impl (typically `std::io::Stderr`).
///
/// When `is_tty` is `false`, all methods are no-ops — safe for pipes and tests.
pub struct ProgressRenderer<W: Write + Send> {
    writer: W,
    is_tty: bool,
    frame: usize,
    /// The last status line currently visible on the terminal (for erase).
    current_line: Option<String>,
}

impl<W: Write + Send> ProgressRenderer<W> {
    /// Create a renderer backed by the given writer.
    pub fn new(is_tty: bool, writer: W) -> Self {
        Self {
            writer,
            is_tty,
            frame: 0,
            current_line: None,
        }
    }

    /// Handle an incoming [`AgentProgressEvent`].
    pub fn handle_event(&mut self, event: AgentProgressEvent) {
        if !self.is_tty {
            return;
        }
        match event {
            AgentProgressEvent::Thinking => {
                self.render_status("Thinking...");
            }
            AgentProgressEvent::ToolStarted { name, .. } => {
                self.render_status(&name);
            }
            AgentProgressEvent::ToolFinished {
                name,
                duration_ms,
                is_error,
            } => {
                let icon = if is_error { "✗" } else { "✓" };
                // Print a completed line (newline-terminated so it persists)
                self.clear_current_line();
                let line = format!("  {} {}  {}ms\n", icon, name, duration_ms);
                let _ = self.writer.write_all(line.as_bytes());
                let _ = self.writer.flush();
                self.current_line = None;
            }
            AgentProgressEvent::Done => {
                self.clear_current_line();
            }
        }
    }

    /// Advance the spinner by one frame (called by the tick thread).
    pub fn tick(&mut self) {
        if !self.is_tty {
            return;
        }
        self.frame = (self.frame + 1) % SPINNER_FRAMES.len();
        // Re-render the current status with the new frame
        if let Some(status) = self.current_line.clone() {
            self.render_status_with_frame(&status, self.frame);
        }
    }

    // -----------------------------------------------------------------------
    // Private helpers
    // -----------------------------------------------------------------------

    /// Render a status line using the current spinner frame.
    fn render_status(&mut self, status: &str) {
        let frame = self.frame;
        self.render_status_with_frame(status, frame);
        self.current_line = Some(status.to_string());
    }

    /// Render a status line with the given spinner frame index.
    fn render_status_with_frame(&mut self, status: &str, frame: usize) {
        let spinner = SPINNER_FRAMES[frame % SPINNER_FRAMES.len()];
        // \r moves to column 0; \x1b[K erases to EOL — overwrites in place
        let line = format!("{}{} {}", ERASE_LINE, spinner, status);
        let _ = self.writer.write_all(line.as_bytes());
        let _ = self.writer.flush();
    }

    /// Erase the current spinner line from the terminal.
    fn clear_current_line(&mut self) {
        if self.current_line.is_some() {
            let _ = self.writer.write_all(ERASE_LINE.as_bytes());
            let _ = self.writer.flush();
            self.current_line = None;
        }
    }
}

// ---------------------------------------------------------------------------
// make_channel_callback
// ---------------------------------------------------------------------------

/// Create a [`ProgressCallback`] that sends events over a
/// `std::sync::mpsc::Sender`.
///
/// If the receiver has been dropped, sends silently fail (no panic).
/// This is the bridge between the async agent loop and the background spinner
/// thread (or a BDD test recorder).
pub fn make_channel_callback(tx: std::sync::mpsc::Sender<AgentProgressEvent>) -> ProgressCallback {
    Arc::new(move |event: AgentProgressEvent| {
        // Intentionally ignore send errors: if the receiver is gone (e.g. test
        // dropped the channel early), we just skip the event.
        let _ = tx.send(event);
    })
}

// ---------------------------------------------------------------------------
// spawn_spinner_thread
// ---------------------------------------------------------------------------

/// A handle to the background spinner thread.
///
/// Dropping this handle signals the spinner thread to stop, but does not
/// wait for it. Call [`SpinnerHandle::stop`] to wait for a clean shutdown.
pub struct SpinnerHandle {
    thread: Option<std::thread::JoinHandle<()>>,
    /// Send side kept alive to prevent the channel from closing prematurely.
    _tx: std::sync::mpsc::Sender<AgentProgressEvent>,
}

impl SpinnerHandle {
    /// Send a Done event and wait for the spinner thread to exit.
    ///
    /// This ensures the spinner line is cleared before the final response
    /// is printed to stdout.
    pub fn stop(mut self) {
        // Sending Done causes the thread to clear the line and exit its loop.
        let _ = self._tx.send(AgentProgressEvent::Done);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// Spawn a background OS thread that drives a `ProgressRenderer<Stderr>` at
/// ~12fps, receiving [`AgentProgressEvent`]s from the returned channel.
///
/// The thread exits when it receives [`AgentProgressEvent::Done`] or when the
/// sender side of the returned channel is dropped.
///
/// **Only call on TTY sessions** — the renderer is a no-op for non-TTY, but
/// spawning an unnecessary thread wastes resources.
pub fn spawn_spinner_thread() -> (ProgressCallback, SpinnerHandle) {
    let (tx, rx) = std::sync::mpsc::channel::<AgentProgressEvent>();
    let tx_clone = tx.clone();

    let thread = std::thread::spawn(move || {
        let stderr = std::io::stderr();
        let mut renderer = ProgressRenderer::new(true, stderr);

        loop {
            match rx.recv_timeout(TICK_INTERVAL) {
                Ok(AgentProgressEvent::Done) => {
                    renderer.handle_event(AgentProgressEvent::Done);
                    break;
                }
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                    // Sender dropped — clean up and exit
                    renderer.handle_event(AgentProgressEvent::Done);
                    break;
                }
                Ok(event) => {
                    renderer.handle_event(event);
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                    renderer.tick();
                }
            }
        }
    });

    let callback = make_channel_callback(tx_clone);
    let handle = SpinnerHandle {
        thread: Some(thread),
        _tx: tx,
    };
    (callback, handle)
}

// ---------------------------------------------------------------------------
// Arc<Mutex<Vec<u8>>> writer adapter (for unit tests)
// ---------------------------------------------------------------------------

/// A `Write` adapter over `Arc<Mutex<Vec<u8>>>` for unit testing.
///
/// Allows `ProgressRenderer` to be created with an in-memory buffer so tests
/// can assert on the rendered output without touching real stderr.
pub struct MutexVecWriter(pub Arc<Mutex<Vec<u8>>>);

impl Write for MutexVecWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl ProgressRenderer<MutexVecWriter> {
    /// Create a TTY-mode renderer that writes to a captured buffer.
    ///
    /// Used by [`crate::interface::cli::run_repl_with_tty_captured`] so BDD
    /// tests can assert on spinner output without touching the real process stderr.
    pub fn new_tty_capture(buf: Arc<Mutex<Vec<u8>>>) -> Self {
        ProgressRenderer::new(true, MutexVecWriter(buf))
    }

    /// Test-only constructor allowing explicit TTY control over the capture buffer.
    ///
    /// Use `new_tty_capture` for production capture scenarios. This variant is
    /// for unit tests that need to verify non-TTY silence (`is_tty = false`).
    #[cfg(test)]
    pub fn new_with_writer(is_tty: bool, buf: Arc<Mutex<Vec<u8>>>) -> Self {
        ProgressRenderer::new(is_tty, MutexVecWriter(buf))
    }
}

#[cfg(test)]
#[path = "progress_tests.rs"]
mod tests;
