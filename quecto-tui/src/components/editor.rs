//! Multi-line editor component with cursor, history, borders.
//!
//! Modelled on Pi TUI's Editor: bordered input area with word wrap,
//! cursor movement, line editing, and input history.

use crate::component::Component;
use crate::keys::Key;
use crate::theme;
use crate::utils::{truncate_to_width, visible_width, wrap_text};

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
    /// Render cache.
    cached_width: Option<usize>,
    cached_lines: Option<Vec<String>>,
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
            cached_width: None,
            cached_lines: None,
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

    /// Whether we're in bash mode (text starts with !).
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
        let top_border = border_color(&border_char.repeat(remaining.min(3)))
            + &indicator_styled
            + &border_color(&border_char.repeat(remaining.saturating_sub(3)));
        output.push(truncate_to_width(&top_border, width, None));

        // Content lines with cursor.
        for (row_idx, line) in self.lines.iter().enumerate() {
            let display = if row_idx == self.cursor_row {
                render_line_with_cursor(line, self.cursor_col, inner_width)
            } else {
                let wrapped = if visible_width(line) > inner_width {
                    wrap_text(line, inner_width)
                } else {
                    vec![line.clone()]
                };
                wrapped
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
                self.insert_newline();
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
            // Ctrl+Enter to submit (raw mode sends this as Ctrl+J sometimes,
            // but we use Alt+Enter as the submit key to avoid ambiguity).
            Key::Alt('\r') | Key::Alt('\n') => {
                self.submit();
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
    let col = cursor_col.min(line.len());
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
mod tests {
    use super::*;

    #[test]
    fn insert_characters() {
        let mut e = Editor::new();
        e.handle_input(&Key::Char('h'));
        e.handle_input(&Key::Char('i'));
        assert_eq!(e.text(), "hi");
    }

    #[test]
    fn backspace_deletes_char() {
        let mut e = Editor::new();
        e.set_text("hello");
        e.handle_input(&Key::Backspace);
        assert_eq!(e.text(), "hell");
    }

    #[test]
    fn cursor_left_right() {
        let mut e = Editor::new();
        e.set_text("abcd");
        e.handle_input(&Key::Left);
        e.handle_input(&Key::Left);
        e.handle_input(&Key::Char('X'));
        assert_eq!(e.text(), "abXcd");
    }

    #[test]
    fn home_moves_to_start() {
        let mut e = Editor::new();
        e.set_text("hello");
        e.handle_input(&Key::Home);
        e.handle_input(&Key::Char('X'));
        assert_eq!(e.text(), "Xhello");
    }

    #[test]
    fn end_moves_to_end() {
        let mut e = Editor::new();
        e.set_text("hello");
        e.handle_input(&Key::Home);
        e.handle_input(&Key::End);
        e.handle_input(&Key::Char('!'));
        assert_eq!(e.text(), "hello!");
    }

    #[test]
    fn ctrl_u_kills_to_start() {
        let mut e = Editor::new();
        e.set_text("hello world");
        // Move cursor to position 5
        e.handle_input(&Key::Home);
        for _ in 0..5 {
            e.handle_input(&Key::Right);
        }
        e.handle_input(&Key::Ctrl('u'));
        assert_eq!(e.text(), " world");
    }

    #[test]
    fn ctrl_k_kills_to_end() {
        let mut e = Editor::new();
        e.set_text("hello world");
        e.handle_input(&Key::Home);
        for _ in 0..5 {
            e.handle_input(&Key::Right);
        }
        e.handle_input(&Key::Ctrl('k'));
        assert_eq!(e.text(), "hello");
    }

    #[test]
    fn multiline_input() {
        let mut e = Editor::new();
        e.handle_input(&Key::Char('a'));
        e.handle_input(&Key::Enter);
        e.handle_input(&Key::Char('b'));
        assert_eq!(e.text(), "a\nb");
        assert_eq!(e.lines.len(), 2);
    }

    #[test]
    fn submit_clears_and_returns() {
        let mut e = Editor::new();
        e.handle_input(&Key::Char('h'));
        e.handle_input(&Key::Char('i'));
        e.submit();
        assert_eq!(e.take_submit(), Some("hi".to_string()));
        assert_eq!(e.text(), "");
    }

    #[test]
    fn history_navigation() {
        let mut e = Editor::new();
        e.set_text("first");
        e.submit();
        e.set_text("second");
        e.submit();
        // Up goes to most recent
        e.navigate_history_up();
        assert_eq!(e.text(), "second");
        e.navigate_history_up();
        assert_eq!(e.text(), "first");
        // Down goes back
        e.navigate_history_down();
        assert_eq!(e.text(), "second");
        e.navigate_history_down();
        assert_eq!(e.text(), ""); // back to empty (saved text)
    }

    #[test]
    fn render_has_borders() {
        let mut e = Editor::new();
        e.set_text("hello");
        let lines = e.render(40);
        assert!(
            lines.len() >= 3,
            "should have top border, content, bottom border"
        );
        // First and last lines should contain border characters
        assert!(
            lines[0].contains('─'),
            "top border should contain ─: {}",
            lines[0]
        );
        assert!(
            lines.last().unwrap().contains('─'),
            "bottom border should contain ─"
        );
    }

    #[test]
    fn render_respects_width() {
        let mut e = Editor::new();
        e.set_text("hello world");
        let lines = e.render(40);
        for line in &lines {
            assert!(
                visible_width(line) <= 40,
                "line exceeds width 40: {} (width={})",
                line,
                visible_width(line)
            );
        }
    }

    #[test]
    fn bash_mode_detected() {
        let mut e = Editor::new();
        e.set_text("!ls -la");
        assert!(e.is_bash_mode());
        e.set_text("hello");
        assert!(!e.is_bash_mode());
    }

    #[test]
    fn delete_at_cursor() {
        let mut e = Editor::new();
        e.set_text("abcd");
        e.handle_input(&Key::Home);
        e.handle_input(&Key::Delete);
        assert_eq!(e.text(), "bcd");
    }

    #[test]
    fn paste_inserts_text() {
        let mut e = Editor::new();
        e.handle_input(&Key::Paste("hello\nworld".to_string()));
        assert_eq!(e.text(), "hello\nworld");
    }

    #[test]
    fn word_left_movement() {
        let mut e = Editor::new();
        e.set_text("hello world test");
        // Cursor is at end
        e.word_left();
        e.handle_input(&Key::Char('X'));
        assert_eq!(e.text(), "hello world Xtest");
    }

    #[test]
    fn word_right_movement() {
        let mut e = Editor::new();
        e.set_text("hello world test");
        e.handle_input(&Key::Home);
        e.word_right();
        e.handle_input(&Key::Char('X'));
        assert_eq!(e.text(), "hello Xworld test");
    }
}
