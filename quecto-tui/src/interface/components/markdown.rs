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
    fn render_markdown(&self, width: usize) -> Vec<String> {
        let content_width = width.saturating_sub(self.padding_x * 2).max(1);
        let pad = " ".repeat(self.padding_x);

        if self.text.trim().is_empty() {
            return vec![];
        }

        let opts = Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TABLES;
        let sanitized_text = sanitize_markdown_source(&self.text);
        let parser = Parser::new_ext(&sanitized_text, opts);

        let mut lines: Vec<RenderedLine> = Vec::new();
        let mut current_line = String::new();
        let mut current_line_hanging_indent: Option<usize> = None;
        let mut in_code_block = false;
        let mut code_block_lang = String::new();
        let mut code_block_content = String::new();
        let mut list_depth: usize = 0;
        let mut list_stack: Vec<ListState> = Vec::new();
        let mut in_blockquote = false;
        let mut blockquote_lines: Vec<String> = Vec::new();
        let mut heading_level: u8 = 0;
        let mut in_table = false;
        let mut table_rows: Vec<Vec<String>> = Vec::new();
        let mut current_row: Vec<String> = Vec::new();
        let mut current_cell = String::new();

        macro_rules! flush_current_line {
            () => {
                flush_line(
                    &mut current_line,
                    &mut lines,
                    &mut current_line_hanging_indent,
                )
            };
        }

        // Style stack for nested inline styles.
        let mut style_stack: Vec<InlineStyle> = Vec::new();

        for event in parser {
            match event {
                Event::Start(tag) => match tag {
                    Tag::Heading { level, .. } => {
                        flush_current_line!();
                        heading_level = level as u8;
                    }
                    Tag::Paragraph => {
                        flush_current_line!();
                    }
                    Tag::CodeBlock(kind) => {
                        flush_current_line!();
                        in_code_block = true;
                        code_block_lang = match kind {
                            pulldown_cmark::CodeBlockKind::Fenced(lang) => {
                                sanitize_for_display(&lang)
                            }
                            _ => String::new(),
                        };
                        code_block_content.clear();
                    }
                    Tag::Table(_alignments) => {
                        flush_current_line!();
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
                        flush_current_line!();
                        list_depth += 1;
                        list_stack.push(ListState::from_start(start));
                    }
                    Tag::Item => {
                        flush_current_line!();
                        let indent = "  ".repeat(list_depth.saturating_sub(1));
                        let marker = match list_stack.last_mut() {
                            Some(ListState::Ordered { next }) => {
                                let num = *next;
                                *next = next.saturating_add(1);
                                format!("{}{}. ", indent, num)
                            }
                            _ => format!("{}{} ", indent, theme::accent("•")),
                        };
                        let marker_width = visible_width(&marker);
                        current_line.push_str(&marker);
                        current_line_hanging_indent = Some(marker_width);
                    }
                    Tag::BlockQuote(_) => {
                        flush_current_line!();
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
                            lines.extend(rendered.into_iter().map(RenderedLine::plain));
                            lines.push(RenderedLine::blank());
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
                        lines.push(RenderedLine::plain(styled));
                        lines.push(RenderedLine::blank()); // spacing after heading
                        heading_level = 0;
                    }
                    TagEnd::Paragraph => {
                        flush_current_line!();
                        lines.push(RenderedLine::blank()); // spacing after paragraph
                    }
                    TagEnd::CodeBlock => {
                        // Render the code block with borders.
                        let border_text = format!("```{}", code_block_lang);
                        lines.push(RenderedLine::plain(theme::dim(&border_text)));
                        for code_line in code_block_content.lines() {
                            lines.push(RenderedLine::plain(format!("  {}", theme::dim(code_line))));
                        }
                        lines.push(RenderedLine::plain(theme::dim("```")));
                        lines.push(RenderedLine::blank());
                        in_code_block = false;
                        code_block_content.clear();
                    }
                    TagEnd::List(_) => {
                        list_depth = list_depth.saturating_sub(1);
                        list_stack.pop();
                        if list_depth == 0 {
                            flush_line(
                                &mut current_line,
                                &mut lines,
                                &mut current_line_hanging_indent,
                            );
                            lines.push(RenderedLine::blank());
                        }
                    }
                    TagEnd::Item => {
                        flush_current_line!();
                    }
                    TagEnd::BlockQuote(_) => {
                        flush_current_line!();
                        // Prefix each blockquote line with a border.
                        for ql in &blockquote_lines {
                            lines.push(RenderedLine::plain(format!(
                                "{} {}",
                                theme::dim("│"),
                                theme::italic(&theme::dim(ql))
                            )));
                        }
                        lines.push(RenderedLine::blank());
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
                        current_cell.push_str(&sanitize_for_display(&text));
                    } else if in_code_block {
                        code_block_content.push_str(&sanitize_for_display(&text));
                    } else if in_blockquote {
                        blockquote_lines.push(sanitize_for_display(&text));
                    } else {
                        let sanitized = sanitize_for_display(&text);
                        let styled = apply_inline_styles(&sanitized, &style_stack);
                        current_line.push_str(&styled);
                    }
                }

                Event::Code(code) => {
                    if in_table {
                        // Append code text to the table cell (#550).
                        // Backticks preserved for display as plain text.
                        let sanitized = sanitize_for_display(&code);
                        current_cell.push('`');
                        current_cell.push_str(&sanitized);
                        current_cell.push('`');
                    } else {
                        let sanitized = sanitize_for_display(&code);
                        let styled = theme::cyan(&format!("`{}`", sanitized));
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
                        flush_current_line!();
                    }
                }

                Event::Rule => {
                    flush_current_line!();
                    lines.push(RenderedLine::plain(theme::dim(
                        &"─".repeat(content_width.min(60)),
                    )));
                    lines.push(RenderedLine::blank());
                }

                _ => {}
            }
        }

        // Flush remaining content.
        flush_current_line!();

        // Remove trailing empty lines.
        while lines.last().map(|l| l.text.is_empty()).unwrap_or(false) {
            lines.pop();
        }

        // Wrap and pad each line.
        let mut result = Vec::new();
        for line in &lines {
            if line.text.is_empty() {
                result.push(String::new());
            } else if visible_width(&line.text) > content_width {
                if let Some(hanging_indent) = line.hanging_indent {
                    push_wrapped_with_hanging_indent(
                        &mut result,
                        &pad,
                        &line.text,
                        hanging_indent,
                        content_width,
                    );
                } else {
                    for wl in wrap_text(&line.text, content_width) {
                        result.push(format!("{}{}", pad, wl));
                    }
                }
            } else {
                result.push(format!("{}{}", pad, line.text));
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
struct RenderedLine {
    text: String,
    hanging_indent: Option<usize>,
}

impl RenderedLine {
    fn plain(text: String) -> Self {
        Self {
            text,
            hanging_indent: None,
        }
    }

    fn wrapped(text: String, hanging_indent: usize) -> Self {
        Self {
            text,
            hanging_indent: Some(hanging_indent),
        }
    }

    fn blank() -> Self {
        Self::plain(String::new())
    }
}

#[derive(Debug, Clone)]
enum ListState {
    Ordered { next: u64 },
    Unordered,
}

impl ListState {
    fn from_start(start: Option<u64>) -> Self {
        match start {
            Some(next) => Self::Ordered { next },
            None => Self::Unordered,
        }
    }
}

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

/// Strip terminal control sequences from markdown source before parsing.
///
/// This keeps `\n` intact so block/list structure still reaches pulldown-cmark,
/// while removing terminal escapes before they can affect display.
fn sanitize_markdown_source(s: &str) -> String {
    sanitize_for_display_preserving_newlines(s)
}

fn sanitize_for_display_preserving_newlines(s: &str) -> String {
    let mut result = String::new();
    let mut chars = s.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '\x1b' {
            consume_escape_sequence(&mut chars);
            continue;
        }

        if ch == '\n' || (ch >= '\u{0020}' && ch != '\u{007F}') {
            result.push(ch);
        }
    }

    result
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
            consume_escape_sequence(&mut chars);
            continue;
        }

        // Filter out control characters (C0: 0x00-0x1F, DEL: 0x7F).
        if ch >= '\u{0020}' && ch != '\u{007F}' {
            result.push(ch);
        }
    }

    result
}

fn consume_escape_sequence<I>(chars: &mut std::iter::Peekable<I>)
where
    I: Iterator<Item = char>,
{
    match chars.peek() {
        Some(&'[') => {
            // CSI sequence: ESC [ ... (letter or ~)
            chars.next();
            loop {
                match chars.next() {
                    Some(c) if c.is_ascii_alphabetic() || c == '~' => break,
                    None => break,
                    _ => {}
                }
            }
        }
        Some(&']') => {
            // OSC sequence: ESC ] ... (BEL or ST)
            chars.next();
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
}

fn push_wrapped_with_hanging_indent(
    result: &mut Vec<String>,
    pad: &str,
    line: &str,
    hanging_indent: usize,
    content_width: usize,
) {
    let split_at = byte_index_for_visible_width(line, hanging_indent);
    let (prefix, rest) = line.split_at(split_at);
    let available = content_width.saturating_sub(hanging_indent);

    if available == 0 {
        for wl in wrap_text(line, content_width) {
            result.push(format!("{}{}", pad, wl));
        }
        return;
    }

    let wrapped = wrap_text(rest, available);
    if let Some((first, tail)) = wrapped.split_first() {
        result.push(format!("{}{}{}", pad, prefix, first));
        let continuation_indent = " ".repeat(hanging_indent);
        for wl in tail {
            result.push(format!("{}{}{}", pad, continuation_indent, wl));
        }
    }
}

fn byte_index_for_visible_width(s: &str, target_width: usize) -> usize {
    for (idx, _) in s.char_indices() {
        if visible_width(&s[..idx]) >= target_width {
            return idx;
        }
    }
    s.len()
}

fn flush_line(
    current: &mut String,
    lines: &mut Vec<RenderedLine>,
    hanging_indent: &mut Option<usize>,
) {
    if !current.is_empty() {
        let text = std::mem::take(current);
        if let Some(indent) = hanging_indent.take() {
            lines.push(RenderedLine::wrapped(text, indent));
        } else {
            lines.push(RenderedLine::plain(text));
        }
    } else {
        *hanging_indent = None;
    }
}

#[cfg(test)]
#[path = "markdown_tests.rs"]
mod tests;
