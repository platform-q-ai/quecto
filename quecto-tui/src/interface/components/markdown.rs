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
}

impl Markdown {
    pub fn new(text: &str, padding_x: usize) -> Self {
        Self {
            text: text.to_string(),
            padding_x,
        }
    }

    pub fn set_text(&mut self, text: &str) {
        self.text = text.to_string();
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
        let mut code_block_indented = false;
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
                    Tag::Paragraph if !in_blockquote => {
                        flush_current_line!();
                    }
                    Tag::Paragraph => {}
                    Tag::CodeBlock(kind) => {
                        flush_current_line!();
                        in_code_block = true;
                        code_block_lang = match kind {
                            pulldown_cmark::CodeBlockKind::Fenced(lang) => {
                                code_block_indented = false;
                                sanitize_for_display(&lang)
                            }
                            _ => {
                                code_block_indented = true;
                                String::new()
                            }
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
                        // Sanitize URL to prevent ANSI/OSC escape injection. If sanitizing
                        // changes the destination, omit the rendered URL entirely so attacker
                        // payload text embedded inside a terminal control sequence cannot be
                        // surfaced as a misleading link target.
                        let safe_url = sanitize_for_display(&dest_url);
                        let safe_url = if safe_url == dest_url.as_ref()
                            && is_safe_link_destination(&safe_url)
                        {
                            safe_url
                        } else {
                            String::new()
                        };
                        style_stack.push(InlineStyle::Link(safe_url));
                    }
                    _ => {}
                },

                Event::End(tag_end) => match tag_end {
                    TagEnd::Table => {
                        flush_table(&table_rows, content_width, &mut lines);
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
                            _ => theme::bold(&theme::accent(&text)),
                        };
                        lines.push(RenderedLine::plain(styled));
                        lines.push(RenderedLine::blank()); // spacing after heading
                        heading_level = 0;
                    }
                    TagEnd::Paragraph => {
                        if in_blockquote {
                            flush_blockquote_line(&mut current_line, &mut blockquote_lines);
                        } else {
                            flush_current_line!();
                            lines.push(RenderedLine::blank()); // spacing after paragraph
                        }
                    }
                    TagEnd::CodeBlock => {
                        flush_code_block(
                            &code_block_lang,
                            &code_block_content,
                            code_block_indented,
                            &mut lines,
                        );
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
                        flush_blockquote_line(&mut current_line, &mut blockquote_lines);
                        let gutter = theme::dim("│ ");
                        let gutter_width = visible_width("│ ");
                        for ql in &blockquote_lines {
                            lines.push(RenderedLine::wrapped(
                                format!("{}{}", gutter, theme::italic(&theme::dim(ql))),
                                gutter_width,
                            ));
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
                            if !url.is_empty() {
                                current_line.push_str(&theme::dim(&format!(" ({})", url)));
                            }
                        }
                    }
                    _ => {}
                },

                Event::Text(text) => {
                    if in_table {
                        current_cell.push_str(&sanitize_for_display(&text));
                    } else if in_code_block {
                        // Preserve newlines: an *indented* fence arrives as Text
                        // events with embedded `\n` (rather than SoftBreaks), and
                        // those newlines are needed to detect/strip a literal
                        // inner fence in `flush_code_block` (#799).
                        code_block_content.push_str(
                            &crate::interface::ansi::sanitize_control_keep_newlines(&text),
                        );
                    } else if in_blockquote {
                        current_line.push_str(&sanitize_for_display(&text));
                    } else {
                        let sanitized = sanitize_for_display(&text);
                        let styled = apply_inline_styles(&sanitized, &style_stack);
                        current_line.push_str(&styled);
                    }
                }

                Event::Code(code) => {
                    let sanitized = sanitize_for_display(&code);
                    if in_table {
                        // Append code text to the table cell (#550).
                        // Backticks preserved for display as plain text.
                        current_cell.push('`');
                        current_cell.push_str(&sanitized);
                        current_cell.push('`');
                    } else {
                        let styled = theme::cyan(&format!("`{}`", sanitized));
                        current_line.push_str(&styled);
                    }
                }

                Event::SoftBreak => {
                    if in_table {
                        current_cell.push(' ');
                    } else if in_code_block {
                        code_block_content.push('\n');
                    } else if in_blockquote {
                        flush_blockquote_line(&mut current_line, &mut blockquote_lines);
                    } else {
                        current_line.push(' ');
                    }
                }

                Event::HardBreak => {
                    if in_table {
                        current_cell.push(' ');
                    } else if in_blockquote {
                        flush_blockquote_line(&mut current_line, &mut blockquote_lines);
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

        flush_current_line!();

        while lines.last().map(|l| l.text.is_empty()).unwrap_or(false) {
            lines.pop();
        }

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
        self.render_markdown(width)
    }

    fn invalidate(&mut self) {}
}

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

fn is_safe_link_destination(dest: &str) -> bool {
    dest.starts_with("http://")
        || dest.starts_with("https://")
        || dest.starts_with("mailto:")
        || dest.starts_with('/')
        || dest.starts_with('#')
}

fn apply_inline_styles(text: &str, stack: &[InlineStyle]) -> String {
    let mut result = text.to_string();
    for style in stack.iter().rev() {
        result = match style {
            InlineStyle::Bold => theme::bold(&result),
            InlineStyle::Italic => theme::italic(&result),
            InlineStyle::Strikethrough => format!("\x1b[9m{}\x1b[29m", result),
            InlineStyle::Link(url) => apply_link_style(&result, url),
        };
    }
    result
}
fn apply_link_style(text: &str, url: &str) -> String {
    if url.is_empty() {
        text.to_string()
    } else {
        format!("\x1b]8;;{url}\x07\x1b[4m\x1b[34m{text}\x1b[0m\x1b]8;;\x07")
    }
}

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

/// Flush a collected markdown table into rendered lines (with trailing blank).
fn flush_table(table_rows: &[Vec<String>], content_width: usize, lines: &mut Vec<RenderedLine>) {
    if table_rows.is_empty() {
        return;
    }
    let rendered = render_table(table_rows, content_width);
    lines.extend(rendered.into_iter().map(RenderedLine::plain));
    lines.push(RenderedLine::blank());
}

fn flush_code_block(lang: &str, content: &str, indented: bool, lines: &mut Vec<RenderedLine>) {
    // CommonMark demotes a 4+-space-indented fence to an *indented* code block,
    // so the literal ```` ```lang ````/```` ``` ```` markers survive as body text
    // with an empty language (#799). Detect that and re-parse the inner fence so
    // those markers are suppressed rather than shown verbatim. Gate on `indented`
    // so a genuine empty-info-string *fenced* block (``` with no language) whose
    // body merely looks like a fence is rendered verbatim, not silently stripped.
    if indented {
        if let Some((inner_lang, inner_body)) = strip_literal_fence(content) {
            flush_code_block(&inner_lang, &inner_body, false, lines);
            return;
        }
    }

    if !lang.is_empty() {
        lines.push(RenderedLine::plain(theme::dim(lang)));
    }
    let gutter = theme::dim("│ ");
    let gutter_width = visible_width("│ ");
    for code_line in content.lines() {
        lines.push(RenderedLine::wrapped(
            format!("{}{}", gutter, theme::dim(code_line)),
            gutter_width,
        ));
    }
    lines.push(RenderedLine::blank());
}

/// Detect an indented code block whose body is itself a literal ```` ``` ````
/// fence and split it into `(language, body)` so it can be rendered as a proper
/// fenced block (#799). Returns `None` when the content is not a wrapped fence.
fn strip_literal_fence(content: &str) -> Option<(String, String)> {
    let lines: Vec<&str> = content.lines().collect();
    let first_idx = lines.iter().position(|l| !l.trim().is_empty())?;
    let last_idx = lines.iter().rposition(|l| !l.trim().is_empty())?;
    if last_idx <= first_idx {
        return None;
    }
    let lang = lines[first_idx].trim_start().strip_prefix("```")?.trim();
    if lines[last_idx].trim() != "```" {
        return None;
    }
    let body = lines[first_idx + 1..last_idx].join("\n");
    Some((lang.to_string(), body))
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

    // Rows to which one over-long cell may grow before being cut with an
    // ellipsis, so a single huge cell cannot flood the transcript.
    const MAX_CELL_ROWS: usize = 4;

    for (row_idx, row) in rows.iter().enumerate() {
        // Wrap each cell inside its own column, so an over-long cell stacks
        // vertically within that column instead of pushing later cells out of
        // alignment or bleeding into them on wrapped continuation lines.
        let cell_rows: Vec<Vec<String>> = (0..num_cols)
            .map(|i| {
                let w = col_widths.get(i).copied().unwrap_or(10).max(1);
                let cell = row.get(i).map(String::as_str).unwrap_or("");
                let mut wrapped = wrap_text(cell, w);
                if wrapped.len() > MAX_CELL_ROWS {
                    wrapped.truncate(MAX_CELL_ROWS);
                    let tail = wrapped.pop().unwrap_or_default();
                    // Re-truncate the final row to make room for the ellipsis
                    // without exceeding the column width. Columns too narrow
                    // for "..." just cut at the width instead. Cell text is
                    // already control-stripped, so drop the SGR reset that
                    // truncate_to_width appends — mid-line it would cancel the
                    // header's bold styling.
                    let strip_reset = |mut s: String| {
                        if let Some(stripped) = s.strip_suffix("\x1b[0m") {
                            s.truncate(stripped.len());
                        }
                        s
                    };
                    wrapped.push(if w >= 3 {
                        let head = strip_reset(truncate_to_width(&tail, w.saturating_sub(3), None));
                        format!("{head}...")
                    } else {
                        strip_reset(truncate_to_width(&tail, w, None))
                    });
                }
                wrapped
            })
            .collect();

        let height = cell_rows.iter().map(Vec::len).max().unwrap_or(1).max(1);
        for r in 0..height {
            let parts: Vec<String> = cell_rows
                .iter()
                .enumerate()
                .map(|(i, cell)| {
                    let w = col_widths.get(i).copied().unwrap_or(10).max(1);
                    let seg = cell.get(r).map(String::as_str).unwrap_or("");
                    let padding = w.saturating_sub(visible_width(seg));
                    format!("{}{}", seg, " ".repeat(padding))
                })
                .collect();
            let line = parts.join(&" ".repeat(gap));
            if row_idx == 0 {
                // Header row — bold.
                lines.push(theme::bold(&line));
            } else {
                lines.push(line);
            }
        }
        if row_idx == 0 {
            // Separator. Match the `.max(1)` floor used for cell layout so a
            // fully-empty column does not shift later separator segments.
            let sep_parts: Vec<String> = col_widths.iter().map(|&w| "─".repeat(w.max(1))).collect();
            lines.push(theme::dim(&sep_parts.join(&" ".repeat(gap))));
        }
    }

    lines
}

/// Strip terminal control sequences from markdown source before parsing.
///
/// This keeps `\n` intact so block/list structure still reaches pulldown-cmark,
/// while removing terminal escapes before they can affect display.
fn sanitize_markdown_source(s: &str) -> String {
    crate::interface::ansi::sanitize_control_keep_newlines(s)
}

/// Strip ANSI escape sequences and control characters from text for safe display.
///
/// Removes complete CSI sequences (ESC[...letter), OSC sequences
/// (ESC]...BEL/ST), bare ESC, and all C0/C1 control characters.
fn sanitize_for_display(s: &str) -> String {
    crate::interface::ansi::sanitize_control(s)
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
    let plain_prefix = crate::interface::ansi::sanitize_control(prefix);
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
        let continuation_prefix = if plain_prefix == "│ " {
            plain_prefix
        } else {
            " ".repeat(hanging_indent)
        };
        for wl in tail {
            result.push(format!("{}{}{}", pad, continuation_prefix, wl));
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

fn flush_blockquote_line(current: &mut String, lines: &mut Vec<String>) {
    let text = std::mem::take(current);
    if !text.is_empty() {
        lines.push(text);
    }
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

#[cfg(test)]
#[path = "markdown_link_tests.rs"]
mod link_tests;

#[cfg(test)]
#[path = "markdown_table_tests.rs"]
mod table_tests;
