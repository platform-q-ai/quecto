//! Markdown renderer — converts markdown to styled ANSI terminal output.
//!
//! Uses `pulldown-cmark` for parsing and custom ANSI styling for output.
//! Handles: headings, bold, italic, code spans, code blocks, lists,
//! blockquotes, links, horizontal rules, strikethrough.

use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};

use crate::component::Component;
use crate::theme;
use crate::utils::{visible_width, wrap_text};

/// Markdown rendering component.
pub struct Markdown {
    text: String,
    padding_x: usize,
    cached_text: Option<String>,
    cached_width: Option<usize>,
    cached_lines: Option<Vec<String>>,
}

impl Markdown {
    pub fn new(text: &str, padding_x: usize) -> Self {
        Self {
            text: text.to_string(),
            padding_x,
            cached_text: None,
            cached_width: None,
            cached_lines: None,
        }
    }

    pub fn set_text(&mut self, text: &str) {
        self.text = text.to_string();
        self.invalidate();
    }

    /// Render markdown text to styled terminal lines.
    fn render_markdown(&self, width: usize) -> Vec<String> {
        let content_width = width.saturating_sub(self.padding_x * 2).max(1);
        let pad = " ".repeat(self.padding_x);

        if self.text.trim().is_empty() {
            return vec![];
        }

        let opts = Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TABLES;
        let parser = Parser::new_ext(&self.text, opts);

        let mut lines: Vec<String> = Vec::new();
        let mut current_line = String::new();
        let mut in_code_block = false;
        let mut code_block_lang = String::new();
        let mut code_block_content = String::new();
        let mut list_depth: usize = 0;
        let mut ordered_list_index: Vec<u64> = Vec::new();
        let mut in_blockquote = false;
        let mut blockquote_lines: Vec<String> = Vec::new();
        let mut heading_level: u8 = 0;

        // Style stack for nested inline styles.
        let mut style_stack: Vec<InlineStyle> = Vec::new();

        for event in parser {
            match event {
                Event::Start(tag) => match tag {
                    Tag::Heading { level, .. } => {
                        flush_line(&mut current_line, &mut lines);
                        heading_level = level as u8;
                    }
                    Tag::Paragraph => {
                        flush_line(&mut current_line, &mut lines);
                    }
                    Tag::CodeBlock(kind) => {
                        flush_line(&mut current_line, &mut lines);
                        in_code_block = true;
                        code_block_lang = match kind {
                            pulldown_cmark::CodeBlockKind::Fenced(lang) => lang.to_string(),
                            _ => String::new(),
                        };
                        code_block_content.clear();
                    }
                    Tag::List(start) => {
                        flush_line(&mut current_line, &mut lines);
                        list_depth += 1;
                        ordered_list_index.push(start.unwrap_or(1));
                    }
                    Tag::Item => {
                        flush_line(&mut current_line, &mut lines);
                        let indent = "  ".repeat(list_depth.saturating_sub(1));
                        let bullet = if let Some(idx) = ordered_list_index.last_mut() {
                            // Check if parent list is ordered.
                            let num = *idx;
                            *idx += 1;
                            format!("{}{}. ", indent, num)
                        } else {
                            format!("{}{} ", indent, theme::accent("•"))
                        };
                        current_line.push_str(&bullet);
                    }
                    Tag::BlockQuote(_) => {
                        flush_line(&mut current_line, &mut lines);
                        in_blockquote = true;
                        blockquote_lines.clear();
                    }
                    Tag::Strong => {
                        style_stack.push(InlineStyle::Bold);
                    }
                    Tag::Emphasis => {
                        style_stack.push(InlineStyle::Italic);
                    }
                    Tag::Strikethrough => {
                        style_stack.push(InlineStyle::Strikethrough);
                    }
                    Tag::Link { dest_url, .. } => {
                        // Sanitize URL to prevent ANSI escape injection.
                        let safe_url = sanitize_for_display(&dest_url);
                        style_stack.push(InlineStyle::Link(safe_url));
                    }
                    _ => {}
                },

                Event::End(tag_end) => match tag_end {
                    TagEnd::Heading(_) => {
                        let text = std::mem::take(&mut current_line);
                        let styled = match heading_level {
                            1 => theme::bold(&theme::underline(&theme::accent(&text))),
                            2 => theme::bold(&theme::accent(&text)),
                            _ => theme::bold(&theme::accent(&format!(
                                "{} {}",
                                "#".repeat(heading_level as usize),
                                text
                            ))),
                        };
                        lines.push(styled);
                        lines.push(String::new()); // spacing after heading
                        heading_level = 0;
                    }
                    TagEnd::Paragraph => {
                        flush_line(&mut current_line, &mut lines);
                        lines.push(String::new()); // spacing after paragraph
                    }
                    TagEnd::CodeBlock => {
                        // Render the code block with borders.
                        let border_text = format!("```{}", code_block_lang);
                        lines.push(theme::dim(&border_text));
                        for code_line in code_block_content.lines() {
                            lines.push(format!("  {}", theme::dim(code_line)));
                        }
                        lines.push(theme::dim("```"));
                        lines.push(String::new());
                        in_code_block = false;
                        code_block_content.clear();
                    }
                    TagEnd::List(_) => {
                        list_depth = list_depth.saturating_sub(1);
                        ordered_list_index.pop();
                        if list_depth == 0 {
                            flush_line(&mut current_line, &mut lines);
                            lines.push(String::new());
                        }
                    }
                    TagEnd::Item => {
                        flush_line(&mut current_line, &mut lines);
                    }
                    TagEnd::BlockQuote(_) => {
                        flush_line(&mut current_line, &mut lines);
                        // Prefix each blockquote line with a border.
                        for ql in &blockquote_lines {
                            lines.push(format!(
                                "{} {}",
                                theme::dim("│"),
                                theme::italic(&theme::dim(ql))
                            ));
                        }
                        lines.push(String::new());
                        in_blockquote = false;
                        blockquote_lines.clear();
                    }
                    TagEnd::Strong => {
                        style_stack.pop();
                    }
                    TagEnd::Emphasis => {
                        style_stack.pop();
                    }
                    TagEnd::Strikethrough => {
                        style_stack.pop();
                    }
                    TagEnd::Link => {
                        if let Some(InlineStyle::Link(url)) = style_stack.pop() {
                            current_line.push_str(&theme::dim(&format!(" ({})", url)));
                        }
                    }
                    _ => {}
                },

                Event::Text(text) => {
                    if in_code_block {
                        code_block_content.push_str(&text);
                    } else if in_blockquote {
                        blockquote_lines.push(text.to_string());
                    } else {
                        let styled = apply_inline_styles(&text, &style_stack);
                        current_line.push_str(&styled);
                    }
                }

                Event::Code(code) => {
                    let styled = theme::cyan(&format!("`{}`", code));
                    current_line.push_str(&styled);
                }

                Event::SoftBreak => {
                    if in_code_block {
                        code_block_content.push('\n');
                    } else {
                        current_line.push(' ');
                    }
                }

                Event::HardBreak => {
                    if in_blockquote {
                        let text = std::mem::take(&mut current_line);
                        blockquote_lines.push(text);
                    } else {
                        flush_line(&mut current_line, &mut lines);
                    }
                }

                Event::Rule => {
                    flush_line(&mut current_line, &mut lines);
                    lines.push(theme::dim(&"─".repeat(content_width.min(60))));
                    lines.push(String::new());
                }

                _ => {}
            }
        }

        // Flush remaining content.
        flush_line(&mut current_line, &mut lines);

        // Remove trailing empty lines.
        while lines.last().map(|l| l.is_empty()).unwrap_or(false) {
            lines.pop();
        }

        // Wrap and pad each line.
        let mut result = Vec::new();
        for line in &lines {
            if line.is_empty() {
                result.push(String::new());
            } else if visible_width(line) > content_width {
                for wl in wrap_text(line, content_width) {
                    result.push(format!("{}{}", pad, wl));
                }
            } else {
                result.push(format!("{}{}", pad, line));
            }
        }

        result
    }
}

impl Component for Markdown {
    fn render(&mut self, width: usize) -> Vec<String> {
        if let (Some(ct), Some(cw), Some(cl)) =
            (&self.cached_text, self.cached_width, &self.cached_lines)
        {
            if ct == &self.text && cw == width {
                return cl.clone();
            }
        }

        let lines = self.render_markdown(width);

        self.cached_text = Some(self.text.clone());
        self.cached_width = Some(width);
        self.cached_lines = Some(lines.clone());

        lines
    }

    fn invalidate(&mut self) {
        self.cached_text = None;
        self.cached_width = None;
        self.cached_lines = None;
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
enum InlineStyle {
    Bold,
    Italic,
    Strikethrough,
    Link(String),
}

fn apply_inline_styles(text: &str, stack: &[InlineStyle]) -> String {
    let mut result = text.to_string();
    for style in stack.iter().rev() {
        result = match style {
            InlineStyle::Bold => theme::bold(&result),
            InlineStyle::Italic => theme::italic(&result),
            InlineStyle::Strikethrough => format!("\x1b[9m{}\x1b[29m", result),
            InlineStyle::Link(_) => theme::underline(&theme::blue(&result)),
        };
    }
    result
}

/// Strip ANSI escape sequences and control characters from text for safe display.
fn sanitize_for_display(s: &str) -> String {
    s.chars()
        .filter(|&c| c >= '\u{0020}' && c != '\u{007F}')
        .collect()
}

fn flush_line(current: &mut String, lines: &mut Vec<String>) {
    if !current.is_empty() {
        lines.push(std::mem::take(current));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strip_ansi(s: &str) -> String {
        let mut result = String::new();
        let mut in_escape = false;
        for ch in s.chars() {
            if in_escape {
                if ch.is_ascii_alphabetic() || ch == '~' {
                    in_escape = false;
                }
            } else if ch == '\x1b' {
                in_escape = true;
            } else {
                result.push(ch);
            }
        }
        result
    }

    fn render_md(text: &str, width: usize) -> Vec<String> {
        let mut md = Markdown::new(text, 0);
        md.render(width)
    }

    fn render_plain(text: &str, width: usize) -> String {
        let lines = render_md(text, width);
        lines
            .iter()
            .map(|l| strip_ansi(l))
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn heading_level_1() {
        let lines = render_md("# Hello", 80);
        assert!(!lines.is_empty());
        let plain = strip_ansi(&lines[0]);
        assert!(
            plain.contains("Hello"),
            "heading should contain 'Hello': {}",
            plain
        );
    }

    #[test]
    fn heading_level_2() {
        let plain = render_plain("## World", 80);
        assert!(plain.contains("World"));
    }

    #[test]
    fn bold_text() {
        let lines = render_md("This is **bold** text", 80);
        let joined = lines.join("");
        // Bold uses \x1b[1m...\x1b[0m
        assert!(
            joined.contains("\x1b[1m"),
            "should contain bold escape: {}",
            joined
        );
    }

    #[test]
    fn italic_text() {
        let lines = render_md("This is *italic* text", 80);
        let joined = lines.join("");
        // Italic uses \x1b[3m...\x1b[0m
        assert!(
            joined.contains("\x1b[3m"),
            "should contain italic escape: {}",
            joined
        );
    }

    #[test]
    fn code_span() {
        let plain = render_plain("Use `cargo build` to compile", 80);
        assert!(
            plain.contains("`cargo build`"),
            "should contain code span: {}",
            plain
        );
    }

    #[test]
    fn code_block() {
        let md = "```rust\nfn main() {}\n```";
        let plain = render_plain(md, 80);
        assert!(
            plain.contains("fn main()"),
            "should contain code: {}",
            plain
        );
        assert!(
            plain.contains("```"),
            "should contain code block borders: {}",
            plain
        );
    }

    #[test]
    fn unordered_list() {
        let plain = render_plain("- item one\n- item two\n- item three", 80);
        assert!(plain.contains("item one"));
        assert!(plain.contains("item two"));
        assert!(plain.contains("item three"));
    }

    #[test]
    fn ordered_list() {
        let plain = render_plain("1. first\n2. second\n3. third", 80);
        assert!(plain.contains("1."));
        assert!(plain.contains("first"));
        assert!(plain.contains("second"));
    }

    #[test]
    fn blockquote() {
        let lines = render_md("> This is a quote", 80);
        let joined = lines.join("\n");
        let plain = strip_ansi(&joined);
        assert!(
            plain.contains("This is a quote"),
            "should contain quote: {}",
            plain
        );
        assert!(
            plain.contains("│"),
            "should contain quote border: {}",
            plain
        );
    }

    #[test]
    fn horizontal_rule() {
        let plain = render_plain("---", 80);
        assert!(plain.contains("─"), "should contain rule: {}", plain);
    }

    #[test]
    fn link() {
        let plain = render_plain("[Example](https://example.com)", 80);
        assert!(
            plain.contains("Example"),
            "should contain link text: {}",
            plain
        );
        assert!(
            plain.contains("example.com"),
            "should contain URL: {}",
            plain
        );
    }

    #[test]
    fn empty_text_renders_empty() {
        let mut md = Markdown::new("", 0);
        let lines = md.render(80);
        assert!(lines.is_empty());
    }

    #[test]
    fn cache_works() {
        let mut md = Markdown::new("# Test", 0);
        let lines1 = md.render(80);
        let lines2 = md.render(80);
        assert_eq!(lines1, lines2);
        assert!(md.cached_lines.is_some());
    }

    #[test]
    fn respects_width() {
        let long_text = "This is a very long paragraph that should be wrapped to fit within the specified terminal width without overflowing.";
        let mut md = Markdown::new(long_text, 1);
        let lines = md.render(40);
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
    fn padding_applied() {
        let mut md = Markdown::new("hello", 2);
        let lines = md.render(80);
        assert!(!lines.is_empty());
        // First non-empty line should start with padding spaces.
        let first = &lines[0];
        assert!(first.starts_with("  "), "should have padding: '{}'", first);
    }
}
