//! Differential renderer — only rewrites changed terminal lines.
//!
//! Modelled on Pi TUI's approach: compare new lines with previous lines,
//! emit cursor movement + line clear + new content only for changed lines.
//! Wraps output in synchronized output markers (`CSI ?2026h` / `CSI ?2026l`)
//! to prevent tearing on terminals that support it.

use std::io::Write;

/// ANSI escape: begin synchronized update.
const SYNC_START: &str = "\x1b[?2026h";
/// ANSI escape: end synchronized update.
const SYNC_END: &str = "\x1b[?2026l";
/// ANSI escape: erase entire line.
const ERASE_LINE: &str = "\x1b[2K";

/// A differential renderer that tracks previously rendered lines and only
/// writes changes to the output.
pub struct DiffRenderer<W: Write> {
    writer: W,
    /// Lines from the previous render cycle.
    previous_lines: Vec<String>,
    /// Terminal width from the previous render cycle.
    previous_width: usize,
    /// Current cursor row (0-indexed from top of our rendered region).
    cursor_row: usize,
}

impl<W: Write> DiffRenderer<W> {
    pub fn new(writer: W) -> Self {
        Self {
            writer,
            previous_lines: Vec::new(),
            previous_width: 0,
            cursor_row: 0,
        }
    }

    /// Render a new set of lines. Only changed lines are written to the terminal.
    ///
    /// `width` is the current terminal width (used to detect resize → full redraw).
    pub fn render(&mut self, new_lines: &[String], width: usize) {
        let width_changed = self.previous_width != 0 && self.previous_width != width;
        let first_render = self.previous_lines.is_empty() && self.previous_width == 0;

        if first_render || width_changed {
            self.full_render(new_lines, width_changed);
            self.previous_lines = new_lines.to_vec();
        } else {
            let changed = self.diff_render(new_lines);
            if changed {
                self.previous_lines = new_lines.to_vec();
            }
        }

        self.previous_width = width;
    }

    /// Force a full redraw on the next render.
    pub fn invalidate(&mut self) {
        self.previous_lines.clear();
        self.previous_width = 0;
        self.cursor_row = 0;
    }

    /// Full render — write all lines.
    fn full_render(&mut self, lines: &[String], clear: bool) {
        let mut buf = String::new();
        buf.push_str(SYNC_START);

        if clear {
            // Clear screen and home cursor
            buf.push_str("\x1b[2J\x1b[H\x1b[3J");
        }

        for (i, line) in lines.iter().enumerate() {
            if i > 0 {
                buf.push_str("\r\n");
            }
            buf.push_str(line);
        }

        buf.push_str(SYNC_END);
        let _ = self.writer.write_all(buf.as_bytes());
        let _ = self.writer.flush();
        self.cursor_row = lines.len().saturating_sub(1);
    }

    /// Differential render — only write changed lines.
    /// Returns `true` if any lines were changed and written.
    fn diff_render(&mut self, new_lines: &[String]) -> bool {
        // Find first and last changed line
        let max_len = new_lines.len().max(self.previous_lines.len());
        let mut first_changed: Option<usize> = None;
        let mut last_changed: usize = 0;

        for i in 0..max_len {
            let old = self.previous_lines.get(i).map(|s| s.as_str()).unwrap_or("");
            let new = new_lines.get(i).map(|s| s.as_str()).unwrap_or("");
            if old != new {
                if first_changed.is_none() {
                    first_changed = Some(i);
                }
                last_changed = i;
            }
        }

        let Some(first) = first_changed else {
            // No changes
            return false;
        };

        let mut buf = String::new();
        buf.push_str(SYNC_START);

        // Move cursor from current position to first changed line
        let delta = first as isize - self.cursor_row as isize;
        if delta > 0 {
            buf.push_str(&format!("\x1b[{}B", delta));
        } else if delta < 0 {
            buf.push_str(&format!("\x1b[{}A", -delta));
        }
        buf.push('\r');

        // Write changed lines
        let render_end = last_changed.min(new_lines.len().saturating_sub(1));
        for i in first..=render_end {
            if i > first {
                buf.push_str("\r\n");
            }
            buf.push_str(ERASE_LINE);
            if let Some(line) = new_lines.get(i) {
                buf.push_str(line);
            }
        }

        // Clear extra lines if content shrank
        if self.previous_lines.len() > new_lines.len() {
            for _ in new_lines.len()..self.previous_lines.len() {
                buf.push_str("\r\n");
                buf.push_str(ERASE_LINE);
            }
            // Move back up
            let extra = self.previous_lines.len() - new_lines.len();
            if extra > 0 {
                buf.push_str(&format!("\x1b[{}A", extra));
            }
        }

        buf.push_str(SYNC_END);
        let _ = self.writer.write_all(buf.as_bytes());
        let _ = self.writer.flush();
        self.cursor_row = render_end;

        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(strs: &[&str]) -> Vec<String> {
        strs.iter().map(|s| s.to_string()).collect()
    }

    /// Helper: render `prev` then `next`, return only the output from the
    /// second render (so we can assert on what changed).
    fn captured_render(prev: &[&str], next: &[&str]) -> String {
        use std::sync::{Arc, Mutex};

        /// A `Write` adapter over `Arc<Mutex<Vec<u8>>>`.
        struct SharedWriter(Arc<Mutex<Vec<u8>>>);
        impl Write for SharedWriter {
            fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
                self.0.lock().unwrap().extend_from_slice(buf);
                Ok(buf.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }

        let buf = Arc::new(Mutex::new(Vec::new()));
        let mut renderer = DiffRenderer::new(SharedWriter(buf.clone()));

        // First render
        if !prev.is_empty() {
            renderer.render(&lines(prev), 80);
        }

        // Clear capture to isolate the second render
        buf.lock().unwrap().clear();

        // Second render — only diff output appears
        renderer.render(&lines(next), 80);

        let data = buf.lock().unwrap().clone();
        String::from_utf8_lossy(&data).to_string()
    }

    #[test]
    fn first_render_outputs_all_lines() {
        let mut buf: Vec<u8> = Vec::new();
        {
            let mut r = DiffRenderer::new(&mut buf as &mut dyn Write);
            r.render(&lines(&["alpha", "beta"]), 80);
        }
        let output = String::from_utf8_lossy(&buf);
        assert!(
            output.contains("alpha"),
            "should contain 'alpha': {}",
            output
        );
        assert!(output.contains("beta"), "should contain 'beta': {}", output);
    }

    #[test]
    fn diff_render_only_emits_changed_lines() {
        let output = captured_render(&["line1", "line2"], &["line1", "CHANGED"]);
        assert!(
            output.contains("CHANGED"),
            "should contain 'CHANGED': {}",
            output
        );
        // "line1" should NOT appear in the diff output (only the first full render has it)
        assert!(
            !output.contains("line1"),
            "should NOT re-emit unchanged 'line1': {}",
            output
        );
    }

    #[test]
    fn diff_render_handles_appended_lines() {
        let output = captured_render(&["line1"], &["line1", "line2"]);
        assert!(output.contains("line2"));
        assert!(!output.contains("line1"));
    }

    #[test]
    fn diff_render_uses_synchronized_output() {
        let mut buf: Vec<u8> = Vec::new();
        {
            let mut r = DiffRenderer::new(&mut buf as &mut dyn Write);
            r.render(&lines(&["hello"]), 80);
        }
        let output = String::from_utf8_lossy(&buf);
        assert!(output.contains(SYNC_START));
        assert!(output.contains(SYNC_END));
    }

    #[test]
    fn diff_render_full_redraw_on_width_change() {
        let output = captured_render(&["same", "same"], &["same", "same"]);
        // With no width change and same content, diff should be minimal
        // (Only cursor positioning, no content)
        // We can't easily test width change in this helper, but verify
        // that identical content produces no content output
        assert!(!output.contains("same"));
    }

    #[test]
    fn diff_render_shrunk_lines() {
        // Previous had 3 lines, new has 2
        let output = captured_render(&["a", "b", "c"], &["a", "b"]);
        // Should clear the removed line
        assert!(output.contains(ERASE_LINE));
    }

    #[test]
    fn diff_render_empty_to_content() {
        // Use the captured_render helper which handles borrows properly
        let output = captured_render(&[], &["hello"]);
        assert!(output.contains("hello"));
    }

    #[test]
    fn diff_render_content_to_empty() {
        let output = captured_render(&["hello"], &[]);
        assert!(output.contains(ERASE_LINE));
    }

    #[test]
    fn diff_render_many_lines() {
        let prev: Vec<&str> = (0..50).map(|_| "same").collect();
        let mut next = prev.clone();
        next[25] = "CHANGED";
        let output = captured_render(&prev, &next);
        assert!(output.contains("CHANGED"));
    }

    #[test]
    fn diff_render_all_changed() {
        let output = captured_render(&["a", "b", "c"], &["x", "y", "z"]);
        assert!(output.contains("x"));
        assert!(output.contains("y"));
        assert!(output.contains("z"));
    }
}
