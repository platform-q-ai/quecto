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
/// ANSI escape: hide the real terminal cursor during TUI paints.
const HIDE_CURSOR: &str = "\x1b[?25l";

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
        // Re-assert cursor hiding on EVERY frame (a few bytes) rather than
        // tracking hide state: emulators, suspend/resume, and crash recovery
        // can re-show the cursor behind our back, and per-frame re-assertion
        // heals all of those on the next paint. Intentional belt-and-braces
        // for the #972 cursor-artifact class.
        buf.push_str(HIDE_CURSOR);
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
        // Per-frame cursor re-hide — same intentional belt-and-braces as in
        // `full_render`; see the comment there (#972).
        buf.push_str(HIDE_CURSOR);
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
#[path = "render_tests.rs"]
mod tests;
