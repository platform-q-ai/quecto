//! Simple text component with word wrapping.

use crate::component::Component;
use crate::utils::{self, visible_width};

/// A text component that word-wraps its content to fit the given width.
pub struct Text {
    content: String,
    cached_width: Option<usize>,
    cached_lines: Option<Vec<String>>,
}

impl Text {
    pub fn new(content: &str) -> Self {
        Self {
            content: content.to_string(),
            cached_width: None,
            cached_lines: None,
        }
    }

    pub fn set_content(&mut self, content: &str) {
        self.content = content.to_string();
        self.invalidate();
    }

    pub fn content(&self) -> &str {
        &self.content
    }
}

impl Component for Text {
    fn render(&mut self, width: usize) -> Vec<String> {
        // Check cache
        if let (Some(cached_w), Some(cached)) = (self.cached_width, &self.cached_lines) {
            if cached_w == width {
                return cached.clone();
            }
        }

        let lines = if self.content.is_empty() {
            vec![String::new()]
        } else if visible_width(&self.content) <= width {
            vec![self.content.clone()]
        } else {
            utils::wrap_text(&self.content, width)
        };

        // Update cache
        self.cached_width = Some(width);
        self.cached_lines = Some(lines.clone());

        lines
    }

    fn invalidate(&mut self) {
        self.cached_width = None;
        self.cached_lines = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_text_single_line() {
        let mut t = Text::new("hello");
        let lines = t.render(80);
        assert_eq!(lines, vec!["hello"]);
    }

    #[test]
    fn long_text_wraps() {
        let mut t = Text::new("The quick brown fox jumps over the lazy dog and keeps running");
        let lines = t.render(30);
        assert!(lines.len() > 1);
        for line in &lines {
            assert!(
                visible_width(line) <= 30,
                "line '{}' exceeds width 30 (actual: {})",
                line,
                visible_width(line)
            );
        }
    }

    #[test]
    fn empty_text_renders_empty_line() {
        let mut t = Text::new("");
        let lines = t.render(80);
        assert_eq!(lines, vec![""]);
    }

    #[test]
    fn set_content_updates() {
        let mut t = Text::new("old");
        t.set_content("new");
        let lines = t.render(80);
        assert_eq!(lines, vec!["new"]);
    }

    #[test]
    fn render_respects_width() {
        let mut t = Text::new("Hello, world!");
        let lines = t.render(80);
        for line in &lines {
            assert!(visible_width(line) <= 80);
        }
    }

    #[test]
    fn cache_is_used_on_same_width() {
        let mut t = Text::new("hello");
        let lines1 = t.render(80);
        let lines2 = t.render(80);
        assert_eq!(lines1, lines2);
        // The cache should be populated
        assert!(t.cached_width.is_some());
        assert!(t.cached_lines.is_some());
    }

    #[test]
    fn cache_invalidated_on_width_change() {
        let mut t = Text::new("hello world test");
        let lines1 = t.render(80);
        let lines2 = t.render(10);
        // Different widths should produce different results
        assert_ne!(lines1.len(), lines2.len());
    }
}
