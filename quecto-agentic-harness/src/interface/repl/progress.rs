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
//! - [`spawn_spinner_thread_with_status`] — spawns a background OS thread that
//!   receives [`AgentProgressEvent`]s from a channel and drives `ProgressRenderer`
//!   at ~12fps, giving the terminal a live animated spinner.
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
use std::path::Path;
use std::sync::Arc;
#[cfg(any(test, feature = "test-support"))]
use std::sync::Mutex;
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

/// Max characters to render for status/detail lines.
const MAX_STATUS_LINE_CHARS: usize = 140;
/// Max characters to render for tool status lines (name + args).
const MAX_TOOL_STATUS_CHARS: usize = 160;

// ---------------------------------------------------------------------------
// Terminal safety
// ---------------------------------------------------------------------------

/// Sanitize a string for safe rendering in terminal output.
///
/// Strips all ASCII control characters (0x00–0x1F, 0x7F) including ANSI ESC
/// (`\x1b`), carriage returns, and null bytes. This prevents terminal escape
/// sequence injection via LLM-controlled tool names.
///
/// Only printable ASCII and valid UTF-8 above U+007E are allowed through.
pub(crate) fn sanitize_for_terminal(s: &str) -> String {
    s.chars()
        .filter(|&c| {
            // Keep printable ASCII (0x20–0x7E) and all non-ASCII Unicode
            c >= '\u{0020}' && c != '\u{007F}'
        })
        .collect()
}

use crate::application::context_pruning::truncate_utf8_safe;

fn sanitize_and_truncate(s: &str, max_chars: usize) -> String {
    let sanitized = sanitize_for_terminal(s);
    truncate_utf8_safe(&sanitized, max_chars).into_owned()
}

fn format_compact_tokens(tokens: usize) -> String {
    if tokens >= 1000 {
        let exact = tokens as f64 / 1000.0;
        if (exact.fract() - 0.0).abs() < f64::EPSILON {
            format!("{}k", tokens / 1000)
        } else {
            format!("{:.1}k", exact)
        }
    } else {
        tokens.to_string()
    }
}

fn format_context_usage(context_tokens: usize, max_context_tokens: usize) -> String {
    if max_context_tokens == 0 {
        return "0.0%/0".to_string();
    }
    let pct = (context_tokens as f64 / max_context_tokens as f64) * 100.0;
    format!("{:.1}%/{}", pct, format_compact_tokens(max_context_tokens))
}

fn format_status_detail(
    context_tokens: usize,
    max_context_tokens: usize,
    provider: &str,
    model: &str,
) -> String {
    let usage = format_context_usage(context_tokens, max_context_tokens);
    format!("{} ({}) {}", usage, provider, model)
}

fn format_tool_status(name: &str, arguments: &str) -> String {
    let safe_name = sanitize_for_terminal(name);
    let safe_args = sanitize_for_terminal(arguments);
    let combined = if safe_args.trim().is_empty() {
        safe_name
    } else {
        format!("{} {}", safe_name, safe_args)
    };
    truncate_utf8_safe(&combined, MAX_TOOL_STATUS_CHARS).into_owned()
}

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
    /// Number of status lines rendered beneath the spinner line.
    current_line_count: usize,
    /// Optional static status header (e.g. workspace path).
    status_header: Option<String>,
    /// Optional dynamic status detail line (e.g. model, context usage).
    status_detail: Option<String>,
}

impl<W: Write + Send> ProgressRenderer<W> {
    /// Create a renderer backed by the given writer.
    pub fn new(is_tty: bool, writer: W) -> Self {
        Self {
            writer,
            is_tty,
            frame: 0,
            current_line: None,
            current_line_count: 0,
            status_header: None,
            status_detail: None,
        }
    }

    /// Create a renderer with an optional static status header line.
    pub fn new_with_status(is_tty: bool, writer: W, status_header: Option<String>) -> Self {
        let mut renderer = Self::new(is_tty, writer);
        renderer.status_header = status_header
            .as_deref()
            .map(|line| sanitize_and_truncate(line, MAX_STATUS_LINE_CHARS));
        renderer
    }

    /// Handle an incoming [`AgentProgressEvent`].
    pub fn handle_event(&mut self, event: AgentProgressEvent) {
        if !self.is_tty {
            return;
        }
        match event {
            AgentProgressEvent::Thinking {
                context_tokens,
                max_context_tokens,
                provider,
                model,
            } => {
                let detail =
                    format_status_detail(context_tokens, max_context_tokens, &provider, &model);
                self.status_detail = Some(sanitize_and_truncate(&detail, MAX_STATUS_LINE_CHARS));
                self.render_status("Thinking...");
            }
            AgentProgressEvent::ToolStarted {
                name, arguments, ..
            } => {
                let status = format_tool_status(&name, &arguments);
                self.render_status(&status);
            }
            AgentProgressEvent::ToolFinished {
                name,
                arguments,
                duration_ms,
                is_error,
                ..
            } => {
                let icon = if is_error { "✗" } else { "✓" };
                let safe_tool = format_tool_status(&name, &arguments);
                // Print a completed line (newline-terminated so it persists)
                self.clear_current_line();
                let line = format!("  {} {}  {}ms\n", icon, safe_tool, duration_ms);
                let _ = self.writer.write_all(line.as_bytes());
                let _ = self.writer.flush();
                self.current_line = None;
            }
            AgentProgressEvent::Token(_) | AgentProgressEvent::TurnCompleted { .. } => {
                // Tokens and per-turn message snapshots are forwarded via the
                // UDS protocol layer, not the REPL spinner. The REPL uses
                // non-streaming progress events.
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
        // Re-render only if there is an active status line.
        // Borrow as &str to avoid allocating a clone of `current_line` every 80ms.
        if self.current_line.is_some() {
            let spinner = SPINNER_FRAMES[self.frame % SPINNER_FRAMES.len()];
            // Build the line from the stored status without cloning the String.
            let status = self.current_line.as_deref().unwrap_or("");
            let line = format!("{}{} {}", ERASE_LINE, spinner, status);
            let _ = self.writer.write_all(line.as_bytes());
            let _ = self.writer.flush();
        }
    }

    // -----------------------------------------------------------------------
    // Private helpers
    // -----------------------------------------------------------------------

    /// Render a status line using the current spinner frame and store it
    /// as the active line (so ticks know what to redraw).
    fn render_status(&mut self, status: &str) {
        self.clear_current_line();
        let spinner = SPINNER_FRAMES[self.frame % SPINNER_FRAMES.len()];
        let status_lines = self.status_lines();
        // \r moves to column 0; \x1b[K erases to EOL — overwrites in place
        let mut line = format!("{}{} {}", ERASE_LINE, spinner, status);
        for status_line in &status_lines {
            line.push('\n');
            line.push_str(status_line);
        }
        if !status_lines.is_empty() {
            line.push_str(&format!("\x1b[{}A\r", status_lines.len()));
        }
        let _ = self.writer.write_all(line.as_bytes());
        let _ = self.writer.flush();
        self.current_line = Some(status.to_string());
        self.current_line_count = status_lines.len();
    }

    /// Return the status lines rendered beneath the spinner (if any).
    fn status_lines(&self) -> Vec<String> {
        let mut lines = Vec::new();
        if let Some(header) = self.status_header.as_deref() {
            if !header.is_empty() {
                lines.push(header.to_string());
            }
        }
        if let Some(detail) = self.status_detail.as_deref() {
            if !detail.is_empty() {
                lines.push(detail.to_string());
            }
        }
        lines
    }

    /// Erase the current spinner line from the terminal.
    fn clear_current_line(&mut self) {
        if self.current_line.is_some() {
            let mut line = String::new();
            line.push_str(ERASE_LINE);
            if self.current_line_count > 0 {
                for _ in 0..self.current_line_count {
                    line.push('\n');
                    line.push_str(ERASE_LINE);
                }
                line.push_str(&format!("\x1b[{}A\r", self.current_line_count));
            }
            let _ = self.writer.write_all(line.as_bytes());
            let _ = self.writer.flush();
            self.current_line = None;
            self.current_line_count = 0;
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
    /// Send a Done event and wait for the spinner thread to exit cleanly.
    ///
    /// This ensures the spinner line is cleared before the final response
    /// is printed to stdout. Prefer calling `stop()` explicitly; the `Drop`
    /// impl provides a best-effort fallback for panic paths.
    pub fn stop(mut self) {
        // Sending Done causes the thread to clear the line and exit its loop.
        let _ = self._tx.send(AgentProgressEvent::Done);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl Drop for SpinnerHandle {
    /// Best-effort cleanup on drop (e.g. panic unwind).
    ///
    /// Sends `Done` so the spinner thread exits and clears the terminal line.
    /// Does not join the thread (joining in `Drop` can deadlock). The thread
    /// will exit naturally once the channel closes.
    fn drop(&mut self) {
        // Ignore send errors — receiver may already be gone.
        let _ = self._tx.send(AgentProgressEvent::Done);
        // Do NOT join here — joining in Drop risks deadlock if the thread
        // is waiting on something the dropping context holds.
    }
}

/// Build a status header line showing the current working directory and git branch.
pub fn build_status_header_line() -> Option<String> {
    let cwd = std::env::current_dir().ok()?;
    let path = cwd.display().to_string();
    let branch = read_git_branch(&cwd);
    Some(match branch {
        Some(b) if !b.is_empty() => format!("{} ({})", path, b),
        _ => path,
    })
}

fn read_git_branch(dir: &Path) -> Option<String> {
    let head_path = dir.join(".git").join("HEAD");
    let content = std::fs::read_to_string(head_path).ok()?;
    let trimmed = content.trim();
    if let Some(reference) = trimmed.strip_prefix("ref: ") {
        return reference.rsplit('/').next().map(|s| s.to_string());
    }
    None
}

/// Spawn a background OS thread that drives a `ProgressRenderer<Stderr>` at
/// ~12fps, receiving [`AgentProgressEvent`]s from the returned channel.
///
/// The thread exits when it receives [`AgentProgressEvent::Done`] or when the
/// sender side of the returned channel is dropped.
///
/// **Only call on TTY sessions** — the renderer is a no-op for non-TTY, but
/// spawning an unnecessary thread wastes resources.
pub fn spawn_spinner_thread_with_status(
    status_header: Option<String>,
) -> (ProgressCallback, SpinnerHandle) {
    let (tx, rx) = std::sync::mpsc::channel::<AgentProgressEvent>();
    let tx_clone = tx.clone();

    let thread = std::thread::spawn(move || {
        let stderr = std::io::stderr();
        let mut renderer = ProgressRenderer::new_with_status(true, stderr, status_header);

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
// Arc<Mutex<Vec<u8>>> writer adapter (for test-support and unit tests)
// ---------------------------------------------------------------------------

/// A `Write` adapter over `Arc<Mutex<Vec<u8>>>` for test capture.
///
/// Allows `ProgressRenderer` to be created with an in-memory buffer so tests
/// can assert on the rendered output without touching real stderr.
///
/// Gated on `test-support` feature and `test` cfg — not present in release builds.
#[cfg(any(test, feature = "test-support"))]
pub struct MutexVecWriter(pub Arc<Mutex<Vec<u8>>>);

#[cfg(any(test, feature = "test-support"))]
impl Write for MutexVecWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        // Recover from mutex poison: if a prior panic poisoned the lock,
        // extract the guard anyway rather than panicking again (double-panic = abort).
        let mut guard = self.0.lock().unwrap_or_else(|e| e.into_inner());
        guard.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[cfg(any(test, feature = "test-support"))]
impl ProgressRenderer<MutexVecWriter> {
    /// Create a TTY-mode renderer that writes to a captured buffer.
    ///
    /// Used by [`crate::interface::cli::run_repl_with_tty_captured`] so BDD
    /// tests can assert on spinner output without touching the real process stderr.
    /// Create a TTY-mode renderer with a status header line.
    pub fn new_tty_capture_with_status(
        buf: Arc<Mutex<Vec<u8>>>,
        status_header: Option<String>,
    ) -> Self {
        ProgressRenderer::new_with_status(true, MutexVecWriter(buf), status_header)
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
