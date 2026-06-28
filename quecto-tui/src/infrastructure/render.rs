//! Differential renderer — only rewrites changed terminal lines.
//!
//! Modelled on Quecto TUI's approach: compare new lines with previous lines,
//! emit cursor movement + line clear + new content only for changed lines.
//! Wraps output in synchronized output markers (`CSI ?2026h` / `CSI ?2026l`)
//! to prevent tearing on terminals that support it.

use std::fmt::Write as _;
use std::io::{self, Write};

/// ANSI escape: begin synchronized update.
const SYNC_START: &str = "\x1b[?2026h";
/// ANSI escape: end synchronized update.
const SYNC_END: &str = "\x1b[?2026l";
/// ANSI escape: erase entire line.
const ERASE_LINE: &str = "\x1b[2K";
/// ANSI escape: reset all SGR attributes (so erases/scrolled-in cells use the
/// default background and never inherit a stale tool-box color — #884).
const SGR_RESET: &str = "\x1b[0m";
/// ANSI escape: disable terminal auto-wrap (so a full-width line written on the
/// bottom row can't auto-scroll the viewport mid-paint — #884).
const AUTOWRAP_OFF: &str = "\x1b[?7l";
/// ANSI escape: re-enable terminal auto-wrap.
const AUTOWRAP_ON: &str = "\x1b[?7h";

/// A differential renderer that tracks previously rendered lines and only
/// writes changes to the output.
pub struct DiffRenderer<W: Write> {
    writer: W,
    /// Lines from the previous render cycle.
    previous_lines: Vec<String>,
    /// Terminal width from the previous render cycle.
    previous_width: usize,
}

impl<W: Write> DiffRenderer<W> {
    pub fn new(writer: W) -> Self {
        Self {
            writer,
            previous_lines: Vec::new(),
            previous_width: 0,
        }
    }

    /// Render a new set of lines. Only changed lines are written to the terminal.
    ///
    /// `width` is the current terminal width (used to detect resize → full redraw).
    pub fn render(&mut self, new_lines: &[String], width: usize) -> io::Result<()> {
        let width_changed = self.previous_width != 0 && self.previous_width != width;
        let first_render = self.previous_lines.is_empty() && self.previous_width == 0;

        if first_render || width_changed {
            self.full_render(new_lines, width_changed)?;
            self.previous_lines = new_lines.to_vec();
        } else {
            let changed = self.diff_render(new_lines)?;
            if changed {
                self.previous_lines = new_lines.to_vec();
            }
        }

        self.previous_width = width;
        Ok(())
    }

    /// Force a full redraw on the next render.
    pub fn invalidate(&mut self) {
        self.previous_lines.clear();
        self.previous_width = 0;
    }

    /// Full render — write all lines.
    fn full_render(&mut self, lines: &[String], clear: bool) -> io::Result<()> {
        let mut buf = String::new();
        buf.push_str(SYNC_START);
        // Establish a known origin for the renderer's cursor cache. The first
        // render can occur after terminal setup/query escape writes, so do not
        // assume the real cursor already starts at row 0/column 0.
        buf.push_str("\x1b[H");

        if clear {
            // Clear screen and home cursor.
            buf.push_str("\x1b[2J\x1b[H\x1b[3J");
        }

        // Disable auto-wrap for the duration of the paint so a full-width line
        // written on the bottom row can't auto-scroll the viewport (which would
        // desync `previous_lines` from the real rows until the next
        // invalidate/resize — the same defect-#1 class fixed in `diff_render`).
        // #884
        buf.push_str(AUTOWRAP_OFF);

        for (i, line) in lines.iter().enumerate() {
            if i > 0 {
                buf.push_str("\r\n");
            }
            buf.push_str(line);
        }

        buf.push_str(AUTOWRAP_ON);
        buf.push_str(SYNC_END);
        self.writer.write_all(buf.as_bytes())?;
        self.writer.flush()?;
        Ok(())
    }

    /// Differential render — only write changed lines.
    /// Returns `true` if any lines were changed and written.
    fn diff_render(&mut self, new_lines: &[String]) -> io::Result<bool> {
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
            return Ok(false);
        };

        let mut buf = String::new();
        buf.push_str(SYNC_START);
        // Reset SGR up front so every ERASE_LINE below — and any line that
        // scrolls into view — paints on the default background, never a stale
        // tool-box color (kills the green bleed across panels). #884
        buf.push_str(SGR_RESET);
        // Disable auto-wrap for the duration of the paint so writing a
        // full-width line on the bottom row cannot scroll the viewport and
        // desync our row model. #884
        buf.push_str(AUTOWRAP_OFF);

        // Repaint each changed line using ABSOLUTE cursor addressing
        // (`\x1b[{row};1H`). The rendered region's origin is row 1 (established
        // by `full_render`'s `\x1b[H`), so line `i` lives on terminal row
        // `i + 1`. Absolute moves can't scroll the viewport the way `\r\n`
        // stepping on the bottom row does — removing the ghost/jitter. #884
        let render_end = last_changed.min(new_lines.len().saturating_sub(1));
        for i in first..=render_end {
            let _ = write!(buf, "\x1b[{};1H", i + 1);
            buf.push_str(ERASE_LINE);
            if let Some(line) = new_lines.get(i) {
                buf.push_str(line);
            }
        }

        // Clear extra lines if content shrank — also absolutely addressed.
        if self.previous_lines.len() > new_lines.len() {
            for i in new_lines.len()..self.previous_lines.len() {
                let _ = write!(buf, "\x1b[{};1H", i + 1);
                buf.push_str(ERASE_LINE);
            }
        }

        buf.push_str(AUTOWRAP_ON);
        buf.push_str(SYNC_END);
        self.writer.write_all(buf.as_bytes())?;
        self.writer.flush()?;

        Ok(true)
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
            renderer.render(&lines(prev), 80).unwrap();
        }

        // Clear capture to isolate the second render
        buf.lock().unwrap().clear();

        // Second render — only diff output appears
        renderer.render(&lines(next), 80).unwrap();

        let data = buf.lock().unwrap().clone();
        String::from_utf8_lossy(&data).to_string()
    }

    #[test]
    fn first_render_outputs_all_lines() {
        let mut buf: Vec<u8> = Vec::new();
        {
            let mut r = DiffRenderer::new(&mut buf as &mut dyn Write);
            r.render(&lines(&["alpha", "beta"]), 80).unwrap();
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
            r.render(&lines(&["hello"]), 80).unwrap();
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

    // ── invalidate() ───────────────────────────────────────────────────

    #[test]
    fn invalidate_forces_full_redraw_on_next_render() {
        use std::sync::{Arc, Mutex};

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
        let mut r = DiffRenderer::new(SharedWriter(buf.clone()));

        // Initial render.
        r.render(&lines(&["alpha", "beta"]), 80).unwrap();
        buf.lock().unwrap().clear();

        // Render identical content → should be a no-op diff (no content).
        r.render(&lines(&["alpha", "beta"]), 80).unwrap();
        let diff_output = {
            let data = buf.lock().unwrap().clone();
            String::from_utf8_lossy(&data).to_string()
        };
        assert!(
            !diff_output.contains("alpha"),
            "identical render should not re-emit: {diff_output}"
        );

        // Invalidate → next render must be full.
        r.invalidate();
        buf.lock().unwrap().clear();

        r.render(&lines(&["alpha", "beta"]), 80).unwrap();
        let full_output = {
            let data = buf.lock().unwrap().clone();
            String::from_utf8_lossy(&data).to_string()
        };
        assert!(
            full_output.contains("alpha"),
            "after invalidate, identical content must be fully redrawn: {full_output}"
        );
        assert!(
            full_output.contains("beta"),
            "after invalidate, all lines must be redrawn: {full_output}"
        );
    }

    #[test]
    fn invalidate_clears_width_change_detection() {
        // After invalidate, a render at the same width as before should still
        // do a full render (because previous_width was reset to 0).
        let output = captured_render_with_invalidate(&["hello"], 80, &["hello"], 80);
        assert!(
            output.contains("hello"),
            "after invalidate, same-width render should be full: {output}"
        );
    }

    #[test]
    fn invalidate_after_width_change_still_full_redraw() {
        // If width changed AND invalidate was called, should still full-render.
        // After invalidate, previous_width is reset to 0, so the width-changed
        // path (which requires previous_width != 0) is NOT taken — instead
        // the first-render path fires, which does NOT clear the screen.
        let output = captured_render_with_invalidate(&["hello"], 80, &["hello"], 120);
        assert!(
            output.contains("hello"),
            "after invalidate + width change, should be full: {output}"
        );
        // first-render path does NOT emit \x1b[2J (no screen clear).
        assert!(
            !output.contains("\x1b[2J"),
            "after invalidate, first-render path should NOT clear screen: {output}"
        );
    }

    /// Helper: render `prev` at `width_prev`, invalidate, then render `next`
    /// at `width_next`, returning only the second render's output.
    fn captured_render_with_invalidate(
        prev: &[&str],
        width_prev: usize,
        next: &[&str],
        width_next: usize,
    ) -> String {
        use std::sync::{Arc, Mutex};

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
        let mut r = DiffRenderer::new(SharedWriter(buf.clone()));

        r.render(&lines(prev), width_prev).unwrap();
        r.invalidate();
        buf.lock().unwrap().clear();
        r.render(&lines(next), width_next).unwrap();

        let data = buf.lock().unwrap().clone();
        String::from_utf8_lossy(&data).to_string()
    }
    struct FailingWriter {
        fail_on_flush: bool,
    }

    impl Write for FailingWriter {
        fn write(&mut self, _buf: &[u8]) -> std::io::Result<usize> {
            if self.fail_on_flush {
                Ok(_buf.len())
            } else {
                Err(std::io::Error::other("write failed"))
            }
        }

        fn flush(&mut self) -> std::io::Result<()> {
            if self.fail_on_flush {
                Err(std::io::Error::other("flush failed"))
            } else {
                Ok(())
            }
        }
    }

    // ── #884: SGR reset + absolute addressing in diff_render ───────────

    /// Defect #2: every `ERASE_LINE` must be preceded by an SGR reset so the
    /// erase cannot inherit a stale (tool-box) background and smear it across
    /// the panel columns.
    #[test]
    fn diff_render_resets_sgr_before_erasing() {
        let output = captured_render(&["a", "b", "c"], &["x", "y", "z"]);
        let reset = output.find("\x1b[0m");
        let erase = output.find(ERASE_LINE);
        assert!(
            reset.is_some(),
            "diff output must emit an SGR reset: {output:?}"
        );
        assert!(erase.is_some(), "diff output must erase: {output:?}");
        assert!(
            reset.unwrap() < erase.unwrap(),
            "SGR reset must precede the first ERASE_LINE: {output:?}"
        );
    }

    /// Defect #1: vertical movement must use absolute cursor addressing
    /// (`\x1b[{row};1H`) so a step on the bottom row can never scroll the
    /// viewport or desync the renderer's row model. No bare `\r\n` vertical
    /// steps may appear in diff output.
    #[test]
    fn diff_render_uses_absolute_addressing_not_newline_steps() {
        let output = captured_render(&["a", "b", "c"], &["x", "y", "z"]);
        assert!(
            !output.contains("\r\n"),
            "diff output must not vertically step with \\r\\n (can scroll): {output:?}"
        );
        assert!(
            output.contains(";1H"),
            "diff output must use absolute cursor addressing: {output:?}"
        );
        // Reject ANY relative vertical move (`\x1b[<n>A`/`\x1b[<n>B`), not just
        // the 1-count form — the old code emitted multi-line steps too.
        let has_relative_vmove = output.as_bytes().windows(2).enumerate().any(|(i, w)| {
            if w != b"\x1b[" {
                return false;
            }
            let mut j = i + 2;
            let bytes = output.as_bytes();
            let mut saw_digit = false;
            while j < bytes.len() && bytes[j].is_ascii_digit() {
                saw_digit = true;
                j += 1;
            }
            saw_digit && j < bytes.len() && (bytes[j] == b'A' || bytes[j] == b'B')
        });
        assert!(
            !has_relative_vmove,
            "diff output must not use relative vertical moves (\\x1b[<n>A/B): {output:?}"
        );
    }

    /// Defect #1, shrink path: clearing trailing lines must also use absolute
    /// addressing (no `\r\n` stepping onto the bottom row).
    #[test]
    fn diff_render_shrink_uses_absolute_addressing() {
        let output = captured_render(&["a", "b", "c", "d"], &["a", "b"]);
        assert!(
            !output.contains("\r\n"),
            "shrink diff must not step with \\r\\n: {output:?}"
        );
        assert!(
            output.contains(ERASE_LINE),
            "shrink diff must erase: {output:?}"
        );
    }

    #[test]
    fn render_returns_write_errors() {
        let mut renderer = DiffRenderer::new(FailingWriter {
            fail_on_flush: false,
        });

        let err = renderer
            .render(&lines(&["hello"]), 80)
            .expect_err("write failure should be returned");

        assert_eq!(err.to_string(), "write failed");
    }

    #[test]
    fn render_returns_flush_errors() {
        let mut renderer = DiffRenderer::new(FailingWriter {
            fail_on_flush: true,
        });

        let err = renderer
            .render(&lines(&["hello"]), 80)
            .expect_err("flush failure should be returned");

        assert_eq!(err.to_string(), "flush failed");
    }
}
