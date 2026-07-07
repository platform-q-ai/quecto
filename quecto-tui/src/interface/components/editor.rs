//! Multi-line editor component with cursor, history, borders.
//!
//! Modelled on Quecto TUI's Editor: bordered input area with word wrap,
//! cursor movement, line editing, and input history.

use crate::interface::component::Component;
use crate::interface::keys::Key;
use crate::interface::theme;
use crate::interface::utils::{truncate_to_width, visible_width, wrap_text};

/// Multi-line text editor with borders and input history.
pub struct Editor {
    /// The text content (may contain newlines for multi-line).
    lines: Vec<String>,
    /// Cursor row (0-indexed into `lines`).
    cursor_row: usize,
    /// Cursor column (byte offset into the current line).
    cursor_col: usize,
    /// Input history for Up/Down navigation.
    history: Vec<String>,
    /// Current position in history (-1 = editing new text).
    history_index: isize,
    /// Saved current text when navigating history.
    saved_text: String,
    /// Submit callback text (set when user presses Ctrl+Enter).
    submit_text: Option<String>,
    /// Border color function name for bash mode detection.
    bash_mode: bool,
    /// Whether to draw the block cursor (false when focus is elsewhere,
    /// e.g. the sub-agent panel).
    show_cursor: bool,
    /// Render cache.
    cached_width: Option<usize>,
    cached_lines: Option<Vec<String>>,
}

impl Default for Editor {
    fn default() -> Self {
        Self::new()
    }
}

impl Editor {
    pub fn new() -> Self {
        Self {
            lines: vec![String::new()],
            cursor_row: 0,
            cursor_col: 0,
            history: Vec::new(),
            history_index: -1,
            saved_text: String::new(),
            submit_text: None,
            bash_mode: false,
            show_cursor: true,
            cached_width: None,
            cached_lines: None,
        }
    }

    /// Show or hide the block cursor (hidden while focus is elsewhere).
    pub fn set_show_cursor(&mut self, show: bool) {
        if self.show_cursor != show {
            self.show_cursor = show;
            self.invalidate();
        }
    }

    /// Get the full text content (lines joined by newline).
    pub fn text(&self) -> String {
        self.lines.join("\n")
    }

    /// Set the text content, resetting cursor to end.
    pub fn set_text(&mut self, text: &str) {
        self.lines = if text.is_empty() {
            vec![String::new()]
        } else {
            text.split('\n').map(|s| s.to_string()).collect()
        };
        self.cursor_row = self.lines.len() - 1;
        self.cursor_col = self.lines[self.cursor_row].len();
        self.update_bash_mode();
        self.invalidate();
    }

    /// Take the submitted text (if any) and clear it.
    pub fn take_submit(&mut self) -> Option<String> {
        self.submit_text.take()
    }

    /// Byte offset of the cursor within the current line.
    pub fn cursor_col(&self) -> usize {
        self.cursor_col
    }

    /// The line the cursor is currently on.
    pub fn current_line(&self) -> &str {
        &self.lines[self.cursor_row]
    }

    /// Replace `[start_col, cursor_col)` on the current line with `replacement`,
    /// leaving the cursor at the end of the inserted text. Used to swap an
    /// `@token` for a selected file path. No-op on a non-char-boundary range.
    pub fn replace_before_cursor(&mut self, start_col: usize, replacement: &str) {
        let line = &mut self.lines[self.cursor_row];
        let end = self.cursor_col.min(line.len());
        let start = start_col.min(end);
        if !line.is_char_boundary(start) || !line.is_char_boundary(end) {
            return;
        }
        line.replace_range(start..end, replacement);
        self.cursor_col = start + replacement.len();
        self.update_bash_mode();
        self.invalidate();
    }

    /// Maximum history entries to retain.
    const MAX_HISTORY: usize = 500;

    /// Add text to the input history.
    pub fn add_to_history(&mut self, text: &str) {
        if !text.is_empty() {
            // Don't duplicate the last entry.
            if self.history.last().map(|s| s.as_str()) != Some(text) {
                self.history.push(text.to_string());
                // Cap history size.
                if self.history.len() > Self::MAX_HISTORY {
                    self.history.remove(0);
                }
            }
        }
        self.history_index = -1;
    }

    /// Whether we're in bash mode (text starts with !). Tests only — production
    /// reads `bash_mode` directly during render.
    #[cfg(test)]
    pub fn is_bash_mode(&self) -> bool {
        self.bash_mode
    }

    fn update_bash_mode(&mut self) {
        let first = self.lines.first().map(|s| s.as_str()).unwrap_or("");
        self.bash_mode = first.trim_start().starts_with('!');
    }

    // ── Input handling ────────────────────────────────────────────────

    fn insert_char(&mut self, ch: char) {
        let col = self.cursor_col.min(self.lines[self.cursor_row].len());
        // Ensure we're at a char boundary.
        let col = if self.lines[self.cursor_row].is_char_boundary(col) {
            col
        } else {
            prev_char_boundary(&self.lines[self.cursor_row], col)
        };
        self.cursor_col = col;
        self.lines[self.cursor_row].insert(col, ch);
        self.cursor_col += ch.len_utf8();
        self.update_bash_mode();
        self.invalidate();
    }

    fn insert_newline(&mut self) {
        let col = self.cursor_col;
        let rest = self.lines[self.cursor_row][col..].to_string();
        self.lines[self.cursor_row].truncate(col);
        self.cursor_row += 1;
        self.lines.insert(self.cursor_row, rest);
        self.cursor_col = 0;
        self.invalidate();
    }

    fn backspace(&mut self) {
        if self.cursor_col > 0 {
            // Find the previous character boundary.
            let line = &self.lines[self.cursor_row];
            let prev = prev_char_boundary(line, self.cursor_col);
            self.lines[self.cursor_row].drain(prev..self.cursor_col);
            self.cursor_col = prev;
            self.update_bash_mode();
            self.invalidate();
        } else if self.cursor_row > 0 {
            // Join with previous line.
            let current = self.lines.remove(self.cursor_row);
            self.cursor_row -= 1;
            self.cursor_col = self.lines[self.cursor_row].len();
            self.lines[self.cursor_row].push_str(&current);
            self.invalidate();
        }
    }

    fn delete(&mut self) {
        let line = &self.lines[self.cursor_row];
        if self.cursor_col < line.len() {
            let next = next_char_boundary(line, self.cursor_col);
            self.lines[self.cursor_row].drain(self.cursor_col..next);
            self.invalidate();
        } else if self.cursor_row + 1 < self.lines.len() {
            let next_line = self.lines.remove(self.cursor_row + 1);
            self.lines[self.cursor_row].push_str(&next_line);
            self.invalidate();
        }
    }

    fn move_left(&mut self) {
        if self.cursor_col > 0 {
            self.cursor_col = prev_char_boundary(&self.lines[self.cursor_row], self.cursor_col);
            self.invalidate();
        } else if self.cursor_row > 0 {
            self.cursor_row -= 1;
            self.cursor_col = self.lines[self.cursor_row].len();
            self.invalidate();
        }
    }

    fn move_right(&mut self) {
        let line = &self.lines[self.cursor_row];
        if self.cursor_col < line.len() {
            self.cursor_col = next_char_boundary(line, self.cursor_col);
            self.invalidate();
        } else if self.cursor_row + 1 < self.lines.len() {
            self.cursor_row += 1;
            self.cursor_col = 0;
            self.invalidate();
        }
    }

    fn move_up(&mut self) {
        if self.cursor_row > 0 {
            self.cursor_row -= 1;
            self.cursor_col = self.cursor_col.min(self.lines[self.cursor_row].len());
            self.invalidate();
        }
    }

    fn move_down(&mut self) {
        if self.cursor_row + 1 < self.lines.len() {
            self.cursor_row += 1;
            self.cursor_col = self.cursor_col.min(self.lines[self.cursor_row].len());
            self.invalidate();
        }
    }

    fn move_home(&mut self) {
        self.cursor_col = 0;
        self.invalidate();
    }

    fn move_end(&mut self) {
        self.cursor_col = self.lines[self.cursor_row].len();
        self.invalidate();
    }

    fn kill_to_start(&mut self) {
        // Ctrl+U: delete from cursor to start of line.
        self.lines[self.cursor_row].drain(..self.cursor_col);
        self.cursor_col = 0;
        self.update_bash_mode();
        self.invalidate();
    }

    fn kill_to_end(&mut self) {
        // Ctrl+K: delete from cursor to end of line.
        self.lines[self.cursor_row].truncate(self.cursor_col);
        self.invalidate();
    }

    fn word_left(&mut self) {
        // Ctrl+Left: move to start of previous word.
        let line = &self.lines[self.cursor_row];
        if self.cursor_col == 0 {
            if self.cursor_row > 0 {
                self.cursor_row -= 1;
                self.cursor_col = self.lines[self.cursor_row].len();
            }
            self.invalidate();
            return;
        }
        let bytes = line.as_bytes();
        let mut pos = self.cursor_col;
        // Skip whitespace backward.
        while pos > 0 && bytes[pos - 1].is_ascii_whitespace() {
            pos -= 1;
        }
        // Skip word chars backward.
        while pos > 0 && !bytes[pos - 1].is_ascii_whitespace() {
            pos -= 1;
        }
        self.cursor_col = pos;
        self.invalidate();
    }

    fn word_right(&mut self) {
        // Ctrl+Right: move to end of next word.
        let line = &self.lines[self.cursor_row];
        if self.cursor_col >= line.len() {
            if self.cursor_row + 1 < self.lines.len() {
                self.cursor_row += 1;
                self.cursor_col = 0;
            }
            self.invalidate();
            return;
        }
        let bytes = line.as_bytes();
        let mut pos = self.cursor_col;
        // Skip word chars forward.
        while pos < bytes.len() && !bytes[pos].is_ascii_whitespace() {
            pos += 1;
        }
        // Skip whitespace forward.
        while pos < bytes.len() && bytes[pos].is_ascii_whitespace() {
            pos += 1;
        }
        self.cursor_col = pos;
        self.invalidate();
    }

    fn navigate_history_up(&mut self) {
        if self.history.is_empty() {
            return;
        }
        if self.history_index < 0 {
            self.saved_text = self.text();
            self.history_index = self.history.len() as isize - 1;
        } else if self.history_index > 0 {
            self.history_index -= 1;
        } else {
            return;
        }
        let text = self.history[self.history_index as usize].clone();
        self.set_text(&text);
    }

    fn navigate_history_down(&mut self) {
        if self.history_index < 0 {
            return;
        }
        if (self.history_index as usize) < self.history.len() - 1 {
            self.history_index += 1;
            let text = self.history[self.history_index as usize].clone();
            self.set_text(&text);
        } else {
            self.history_index = -1;
            let text = self.saved_text.clone();
            self.set_text(&text);
        }
    }

    fn submit(&mut self) {
        let text = self.text();
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return;
        }
        self.add_to_history(trimmed);
        self.submit_text = Some(text);
        self.lines = vec![String::new()];
        self.cursor_row = 0;
        self.cursor_col = 0;
        self.update_bash_mode();
        self.invalidate();
    }
}

impl Component for Editor {
    fn render(&mut self, width: usize) -> Vec<String> {
        if let Some(cached) = &self.cached_lines {
            if self.cached_width == Some(width) {
                return cached.clone();
            }
        }

        let inner_width = if width > 4 { width - 2 } else { width }; // 1 char padding each side
        let border_char = "─";
        let border_color: fn(&str) -> String = if self.bash_mode {
            theme::yellow
        } else {
            theme::accent
        };

        let mut output = Vec::new();

        // Top border with prompt indicator.
        let indicator = if self.bash_mode { " ! " } else { " > " };
        let indicator_styled = border_color(indicator);
        let indicator_width = visible_width(indicator);
        let remaining = width.saturating_sub(indicator_width);
        let top_border = border_color(&border_char.repeat(remaining.min(indicator_width)))
            + &indicator_styled
            + &border_color(&border_char.repeat(remaining.saturating_sub(indicator_width)));
        output.push(truncate_to_width(&top_border, width, None));

        // Content lines with cursor.
        for (row_idx, line) in self.lines.iter().enumerate() {
            let display = if row_idx == self.cursor_row && self.show_cursor {
                render_line_with_cursor(line, self.cursor_col, inner_width)
            } else if visible_width(line) > inner_width {
                wrap_text(line, inner_width)
            } else {
                vec![line.clone()]
            };
            for dl in display {
                let padded = format!(" {} ", dl);
                output.push(truncate_to_width(&padded, width, None));
            }
        }

        // Bottom border.
        let bottom = border_color(&border_char.repeat(width));
        output.push(truncate_to_width(&bottom, width, None));

        self.cached_width = Some(width);
        self.cached_lines = Some(output.clone());
        output
    }

    fn handle_input(&mut self, key: &Key) -> bool {
        match key {
            Key::Char(ch) => {
                self.insert_char(*ch);
                true
            }
            Key::Enter => {
                self.submit();
                true
            }
            Key::Backspace => {
                self.backspace();
                true
            }
            Key::Delete => {
                self.delete();
                true
            }
            Key::Left => {
                self.move_left();
                true
            }
            Key::Right => {
                self.move_right();
                true
            }
            Key::Up => {
                if self.lines.len() == 1 {
                    self.navigate_history_up();
                } else {
                    self.move_up();
                }
                true
            }
            Key::Down => {
                if self.lines.len() == 1 {
                    self.navigate_history_down();
                } else {
                    self.move_down();
                }
                true
            }
            Key::Home => {
                self.move_home();
                true
            }
            Key::End => {
                self.move_end();
                true
            }
            Key::Ctrl('u') => {
                self.kill_to_start();
                true
            }
            Key::Ctrl('k') => {
                self.kill_to_end();
                true
            }
            Key::Ctrl('a') => {
                self.move_home();
                true
            }
            Key::Ctrl('e') => {
                self.move_end();
                true
            }
            // Shift+Enter or Alt+Enter inserts a newline (multi-line editing).
            Key::ShiftEnter | Key::Alt('\r') | Key::Alt('\n') => {
                self.insert_newline();
                true
            }
            // Ctrl+Left / Ctrl+Right for word movement.
            Key::Alt('b') => {
                self.word_left();
                true
            }
            Key::Alt('f') => {
                self.word_right();
                true
            }
            Key::Paste(text) => {
                for ch in text.chars() {
                    if ch == '\n' {
                        self.insert_newline();
                    } else if ch != '\r' {
                        self.insert_char(ch);
                    }
                }
                true
            }
            _ => false,
        }
    }

    fn invalidate(&mut self) {
        self.cached_width = None;
        self.cached_lines = None;
    }
}

/// Render a single line with a block cursor at the given column.
fn render_line_with_cursor(line: &str, cursor_col: usize, max_width: usize) -> Vec<String> {
    // `cursor_col` is normally kept on a char boundary by the editor's
    // mutators, but slicing on a mid-char column would panic. Snap to the
    // previous boundary defensively so any future caller stays safe.
    let mut col = cursor_col.min(line.len());
    if !line.is_char_boundary(col) {
        col = prev_char_boundary(line, col);
    }
    let before = &line[..col];
    let at_cursor = if col < line.len() {
        let next = next_char_boundary(line, col);
        &line[col..next]
    } else {
        " "
    };
    let after = if col < line.len() {
        let next = next_char_boundary(line, col);
        &line[next..]
    } else {
        ""
    };

    // Reverse video for cursor character.
    let cursor_display = format!("{}\x1b[7m{}\x1b[27m{}", before, at_cursor, after);

    if visible_width(&cursor_display) <= max_width {
        vec![cursor_display]
    } else {
        // If line is too long, wrap it. Cursor styling will be approximate.
        wrap_text(&cursor_display, max_width)
    }
}

/// Find the previous character boundary before `pos` in a UTF-8 string.
fn prev_char_boundary(s: &str, pos: usize) -> usize {
    let mut p = pos;
    while p > 0 {
        p -= 1;
        if s.is_char_boundary(p) {
            return p;
        }
    }
    0
}

/// Find the next character boundary after `pos` in a UTF-8 string.
fn next_char_boundary(s: &str, pos: usize) -> usize {
    let mut p = pos + 1;
    while p < s.len() {
        if s.is_char_boundary(p) {
            return p;
        }
        p += 1;
    }
    s.len()
}

#[cfg(test)]
#[path = "editor_tests.rs"]
mod tests;
