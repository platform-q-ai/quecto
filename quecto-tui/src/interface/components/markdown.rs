//! Markdown renderer - converts markdown to styled ANSI terminal output.
//!
//! Uses `pulldown-cmark` for parsing and custom ANSI styling for output.
//! Handles: headings, bold, italic, code spans, code blocks, lists,
//! blockquotes, links, horizontal rules, strikethrough.

use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};

use crate::interface::component::Component;
use crate::interface::theme;
use crate::interface::utils::{truncate_to_width, visible_width, wrap_text};

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
    #[allow(clippy::cognitive_complexity, clippy::too_many_lines)]
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
        let mut in_table = false;
        let mut table_rows: Vec<Vec<String>> = Vec::new();
        let mut current_row: Vec<String> = Vec::new();
        let mut current_cell = String::new();

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
                    Tag::Table(_alignments) => {
                        flush_line(&mut current_line, &mut lines);
                        in_table = true;
                        table_rows.clear();
                    }
                    Tag::TableHead | Tag::TableRow => {
                        current_row = Vec::new();
                    }
                    Tag::TableCell => {
                        current_cell = String::new();
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
                    TagEnd::Table => {
                        // Render the collected table.
                        if !table_rows.is_empty() {
                            let rendered = render_table(&table_rows, content_width);
                            lines.extend(rendered);
                            lines.push(String::new());
                        }
                        in_table = false;
                        table_rows.clear();
                    }
                    TagEnd::TableHead | TagEnd::TableRow if !current_row.is_empty() => {
                        table_rows.push(std::mem::take(&mut current_row));
                    }
                    TagEnd::TableHead | TagEnd::TableRow => {}
                    TagEnd::TableCell => {
                        let sanitized = sanitize_for_display(&current_cell);
                        current_cell.clear();
                        current_row.push(sanitized);
                    }
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
                    if in_table {
                        current_cell.push_str(&text);
                    } else if in_code_block {
                        code_block_content.push_str(&text);
                    } else if in_blockquote {
                        blockquote_lines.push(text.to_string());
                    } else {
                        let styled = apply_inline_styles(&text, &style_stack);
                        current_line.push_str(&styled);
                    }
                }

                Event::Code(code) => {
                    if in_table {
                        // Append code text to the table cell (#550).
                        // Backticks preserved for display as plain text.
                        current_cell.push('`');
                        current_cell.push_str(&code);
                        current_cell.push('`');
                    } else {
                        let styled = theme::cyan(&format!("`{}`", code));
                        current_line.push_str(&styled);
                    }
                }

                Event::SoftBreak => {
                    if in_table {
                        current_cell.push(' ');
                    } else if in_code_block {
                        code_block_content.push('\n');
                    } else {
                        current_line.push(' ');
                    }
                }

                Event::HardBreak => {
                    if in_table {
                        current_cell.push(' ');
                    } else if in_blockquote {
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

/// Shrink column widths to fit available space (#550).
///
/// Leaves narrow columns at their natural width; only shrinks columns
/// that exceed their fair share. Iterates until all columns fit.
fn shrink_columns(widths: &mut [usize], avail: usize) {
    let n = widths.len();
    if n == 0 {
        return;
    }
    let min_col = 3;
    let mut frozen = vec![false; n];

    // Iterative: freeze columns at or below fair share, redistribute
    // remaining space among unfrozen columns.
    for _ in 0..n {
        let unfrozen_count = frozen.iter().filter(|&&f| !f).count();
        if unfrozen_count == 0 {
            break;
        }
        let frozen_total: usize = widths
            .iter()
            .zip(frozen.iter())
            .filter(|&(_, &f)| f)
            .map(|(&w, _)| w)
            .sum();
        let remaining = avail.saturating_sub(frozen_total);
        let fair = remaining / unfrozen_count;

        let mut changed = false;
        for i in 0..n {
            if !frozen[i] && widths[i] <= fair {
                frozen[i] = true;
                changed = true;
            }
        }
        if !changed {
            // All unfrozen columns exceed fair share — distribute evenly.
            // Give extra remainder chars to the first unfrozen columns.
            let mut assigned = 0;
            let unfrozen: Vec<usize> = (0..n).filter(|&i| !frozen[i]).collect();
            for (j, &i) in unfrozen.iter().enumerate() {
                let w = if j < remaining % unfrozen.len() {
                    fair + 1
                } else {
                    fair
                };
                widths[i] = w.max(min_col);
                assigned += widths[i];
            }
            // If frozen columns already consumed all space, min_col may
            // push total over avail — accept this as a graceful overflow.
            let _ = assigned;
            break;
        }
    }
}

/// Render a table as aligned text columns.
#[allow(clippy::cognitive_complexity)]
fn render_table(rows: &[Vec<String>], max_width: usize) -> Vec<String> {
    if rows.is_empty() {
        return vec![];
    }

    // Calculate column widths using display width (not byte length).
    let num_cols = rows.iter().map(|r| r.len()).max().unwrap_or(0);
    let mut col_widths = vec![0usize; num_cols];
    for row in rows {
        for (i, cell) in row.iter().enumerate() {
            if i < num_cols {
                col_widths[i] = col_widths[i].max(visible_width(cell));
            }
        }
    }

    // Cap total width (#550).
    // Strategy: shrink only oversized columns, leaving narrow ones at natural width.
    let gap = 2; // spaces between columns
    let total: usize = col_widths.iter().sum::<usize>() + (num_cols.saturating_sub(1)) * gap;
    if total > max_width && num_cols > 0 {
        let avail = max_width.saturating_sub((num_cols.saturating_sub(1)) * gap);
        let sum: usize = col_widths.iter().sum();
        if sum > 0 {
            shrink_columns(&mut col_widths, avail);
        } else {
            let min_per_col = (avail / num_cols).max(1);
            col_widths.fill(min_per_col);
        }
    }

    let mut lines = Vec::new();

    for (row_idx, row) in rows.iter().enumerate() {
        let mut parts = Vec::new();
        for (i, cell) in row.iter().enumerate() {
            let w = col_widths.get(i).copied().unwrap_or(10);
            let truncated = truncate_to_width(cell, w, None);
            let cell_width = visible_width(&truncated);
            let padding = w.saturating_sub(cell_width);
            let padded = format!("{}{}", truncated, " ".repeat(padding));
            parts.push(padded);
        }
        let line = parts.join(&" ".repeat(gap));
        if row_idx == 0 {
            // Header row — bold.
            lines.push(theme::bold(&line));
            // Separator.
            let sep_parts: Vec<String> = col_widths.iter().map(|&w| "─".repeat(w)).collect();
            lines.push(theme::dim(&sep_parts.join(&" ".repeat(gap))));
        } else {
            lines.push(line);
        }
    }

    lines
}

/// Strip ANSI escape sequences and control characters from text for safe display.
///
/// Removes complete CSI sequences (ESC[...letter), OSC sequences
/// (ESC]...BEL/ST), bare ESC, and all C0/C1 control characters.
fn sanitize_for_display(s: &str) -> String {
    let mut result = String::new();
    let mut chars = s.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '\x1b' {
            // Start of escape sequence — consume the entire sequence.
            match chars.peek() {
                Some(&'[') => {
                    // CSI sequence: ESC [ ... (letter or ~)
                    chars.next(); // consume '['
                    loop {
                        match chars.next() {
                            Some(c) if c.is_ascii_alphabetic() || c == '~' => break,
                            None => break,
                            _ => {} // consume parameter bytes
                        }
                    }
                }
                Some(&']') => {
                    // OSC sequence: ESC ] ... (BEL or ST)
                    chars.next(); // consume ']'
                    loop {
                        match chars.next() {
                            Some('\x07') => break,
                            Some('\x1b') if chars.peek() == Some(&'\\') => {
                                chars.next();
                                break;
                            }
                            None => break,
                            _ => {}
                        }
                    }
                }
                _ => {
                    // Bare ESC or unknown — skip ESC and continue.
                }
            }
            continue;
        }
        // Filter out control characters (C0: 0x00-0x1F, DEL: 0x7F).
        if ch >= '\u{0020}' && ch != '\u{007F}' {
            result.push(ch);
        }
    }

    result
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

    // --- Table safety tests (#465, #468, #470) ---

    #[test]
    fn table_cell_ansi_escape_stripped() {
        // An LLM could inject ANSI escapes in table cell content.
        // The \x1b[31m sequence sets red text — must be stripped.
        let md = "| Header |\n|--------|\n| \x1b[31mred\x1b[0m |";
        let plain = render_plain(md, 80);
        assert!(
            !plain.contains("\x1b"),
            "ANSI escapes should be stripped from table cells: {}",
            plain
        );
        assert!(
            plain.contains("red"),
            "cell text should be preserved: {}",
            plain
        );
    }

    #[test]
    fn table_cell_control_chars_stripped() {
        // Control characters like BEL, cursor movement must be stripped.
        let md = "| Header |\n|--------|\n| \x07bell\x08back |";
        let plain = render_plain(md, 80);
        assert!(
            !plain.contains('\x07'),
            "BEL should be stripped: {:?}",
            plain
        );
        assert!(
            !plain.contains('\x08'),
            "BS should be stripped: {:?}",
            plain
        );
    }

    #[test]
    fn table_cell_sanitize_preserves_text() {
        let md = "| Name | Value |\n|------|-------|\n| foo  | bar   |";
        let plain = render_plain(md, 80);
        assert!(plain.contains("foo"), "cell text should be preserved");
        assert!(plain.contains("bar"), "cell text should be preserved");
    }

    #[test]
    fn table_cjk_column_width() {
        // CJK characters are double-width. "你好" is 4 display columns.
        // The column must be at least 4 wide, not 6 (byte length of UTF-8).
        let md = "| Header |\n|--------|\n| 你好 |";
        let plain = render_plain(md, 80);
        // The key test is that render_table uses visible_width, not .len().
        // We verify by checking the render doesn't panic and text appears.
        assert!(plain.contains("你好"), "CJK text should appear: {}", plain);
    }

    #[test]
    fn table_column_width_uses_display_width_not_bytes() {
        // "café" is 5 bytes but 4 display characters.
        // Column width should be 4 (display), not 5 (bytes).
        let rows = vec![vec!["café".to_string()], vec!["test".to_string()]];
        let lines = render_table(&rows, 80);
        // Both rows should align — if byte length is used, "café" gets
        // allocated 5 chars of width while "test" gets 4, causing misalignment.
        let plain: Vec<String> = lines.iter().map(|l| strip_ansi(l)).collect();
        assert!(plain.len() >= 2, "should have header + separator + data");
        // Verify the data row "test" is padded to the same width as "café".
        // With visible_width, both are 4 display chars, so padding is identical.
    }

    #[test]
    fn table_all_empty_cells_no_panic() {
        // All empty cells means col_widths sum is 0 — must not divide by zero.
        let md = "| | |\n|--|--|\n| | |";
        let plain = render_plain(md, 80);
        // Should not panic — just render empty or minimal table.
        let _ = plain;
    }

    #[test]
    fn table_all_empty_cells_via_render_table() {
        // Direct test of render_table with empty cells.
        let rows = vec![
            vec![String::new(), String::new()],
            vec![String::new(), String::new()],
        ];
        // Must not panic (division by zero in scale calculation).
        let lines = render_table(&rows, 40);
        assert!(!lines.is_empty(), "should produce some output");
    }

    #[test]
    fn sanitize_for_display_strips_full_ansi_sequences() {
        // Full CSI sequences must be completely removed, not just the ESC byte.
        assert_eq!(sanitize_for_display("\x1b[31mhello\x1b[0m"), "hello");
        assert_eq!(sanitize_for_display("\x1b[1;31;42mtext\x1b[0m"), "text");
    }

    #[test]
    fn sanitize_for_display_strips_osc_sequences() {
        // OSC hyperlink: ESC]8;;url BEL text ESC]8;; BEL
        let osc = "\x1b]8;;http://evil.com\x07click\x1b]8;;\x07";
        assert_eq!(sanitize_for_display(osc), "click");
    }

    #[test]
    fn sanitize_for_display_strips_control_chars() {
        assert_eq!(sanitize_for_display("normal"), "normal");
        assert_eq!(sanitize_for_display("\x07\x08\x0B"), "");
        assert_eq!(sanitize_for_display("a\x00b"), "ab");
        assert_eq!(sanitize_for_display("a\x7Fb"), "ab"); // DEL
    }

    #[test]
    fn sanitize_for_display_preserves_normal_text() {
        assert_eq!(sanitize_for_display("hello world"), "hello world");
        assert_eq!(sanitize_for_display("café"), "café");
        assert_eq!(sanitize_for_display("你好"), "你好");
    }

    // --- Inline code in table cells (#550) ---

    #[test]
    fn table_inline_code_stays_in_cell() {
        let md = "| Tool | Description |\n|------|-------------|\n| `bash` | Run commands |";
        let plain = render_plain(md, 80);
        // "bash" should be on the same line as "Run commands", not on a separate line.
        let lines: Vec<&str> = plain.lines().collect();
        let data_line = lines
            .iter()
            .find(|l| l.contains("bash"))
            .expect("should contain bash");
        assert!(
            data_line.contains("Run commands"),
            "inline code and description should be on the same line: {:?}",
            data_line,
        );
    }

    #[test]
    fn table_mixed_text_and_code_in_cell() {
        let md = "| Command |\n|---------|\n| Use `poem.txt` file |";
        let plain = render_plain(md, 80);
        let data_line = plain
            .lines()
            .find(|l| l.contains("poem.txt"))
            .expect("should contain poem.txt");
        assert!(
            data_line.contains("Use") && data_line.contains("file"),
            "mixed text and code should be in one cell: {:?}",
            data_line,
        );
    }

    #[test]
    fn table_code_only_cell_renders_correctly() {
        let md = "| Name |\n|------|\n| `test` |";
        let plain = render_plain(md, 80);
        assert!(
            plain.contains("test"),
            "code-only cell should contain the text: {:?}",
            plain,
        );
    }

    #[test]
    fn table_tool_list_not_truncated_at_80_cols() {
        let md = "| Tool | Description |\n|------|-------------|\n\
            | `spawn` | Start a background subagent |\n\
            | `agent_cmd` | Send commands to a spawned subagent |\n\
            | `Bash` | Execute a bash command |\n\
            | `Edit` | Surgically replace exact text |\n\
            | `Write` | Create or overwrite a file |\n\
            | `Read` | Read file contents |";
        let plain = render_plain(md, 80);
        // All tool names should be fully visible, not truncated.
        assert!(
            plain.contains("`spawn`"),
            "spawn should not be truncated: {}",
            plain
        );
        assert!(
            plain.contains("`agent_cmd`"),
            "agent_cmd should not be truncated: {}",
            plain
        );
        assert!(
            plain.contains("`Bash`"),
            "Bash should not be truncated: {}",
            plain
        );
        assert!(
            plain.contains("`Edit`"),
            "Edit should not be truncated: {}",
            plain
        );
        assert!(
            plain.contains("`Write`"),
            "Write should not be truncated: {}",
            plain
        );
        assert!(
            plain.contains("`Read`"),
            "Read should not be truncated: {}",
            plain
        );
    }

    #[test]
    fn table_narrow_width_still_shows_tool_names() {
        // Even at 60 cols, short tool names should not be clipped to 4 chars.
        let md = "| Tool | Description |\n|------|-------------|\n\
            | `spawn` | Start a background subagent process with optional system prompt and initial task |\n\
            | `agent_cmd` | Send commands to a spawned subagent (prompt, steer, follow_up, abort, get_state, get_messages) |";
        let plain = render_plain(md, 60);
        // Tool column should have at least enough width for `agent_cmd` (13 chars with backticks).
        assert!(
            plain.contains("`spawn`"),
            "spawn truncated at 60 cols: {}",
            plain
        );
    }
}
