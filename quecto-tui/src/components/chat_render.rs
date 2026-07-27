use std::borrow::Cow;

use super::*;
use crate::components::ansi::AnsiSegment;

pub(super) struct ToolRenderArgs<'a> {
    pub tool_name: &'a str,
    pub args_json: &'a Option<serde_json::Value>,
    pub result: Option<&'a str>,
    pub is_error: bool,
    pub duration_ms: Option<u64>,
    pub expanded: bool,
    pub width: usize,
}

pub(super) fn render_tool_execution(args: ToolRenderArgs<'_>) -> Vec<String> {
    let ToolRenderArgs {
        tool_name,
        args_json,
        result,
        is_error,
        duration_ms,
        expanded,
        width,
    } = args;
    // Build content lines. Tool blocks deliberately inherit the terminal's
    // foreground/background so they stay readable in light and dark themes.
    let mut content: Vec<String> = Vec::new();
    let inner_width = width.saturating_sub(2); // 1 char padding each side

    // Duration string.
    let dur = duration_ms
        .map(|ms| theme::dim(&format!("  {}ms", ms)))
        .unwrap_or_default();

    // Status icon.
    let icon = if result.is_none() {
        theme::spinner("⠋")
    } else if is_error {
        theme::error("✗")
    } else {
        theme::success("✓")
    };

    // Tool-specific rendering.
    match tool_name {
        "bash" => render_bash(
            &mut content,
            &icon,
            &dur,
            args_json,
            result,
            is_error,
            expanded,
            inner_width,
        ),
        "read" => render_read(
            &mut content,
            &icon,
            &dur,
            args_json,
            result,
            expanded,
            inner_width,
        ),
        "write" => render_write(
            &mut content,
            &icon,
            &dur,
            args_json,
            result,
            expanded,
            inner_width,
        ),
        "edit" => render_edit(
            &mut content,
            &icon,
            &dur,
            args_json,
            result,
            is_error,
            expanded,
            inner_width,
        ),
        "spawn" | "agent_cmd" => render_subagent(
            &mut content,
            tool_name,
            &icon,
            &dur,
            args_json,
            result,
            is_error,
            inner_width,
        ),
        "workflow" => render_workflow(&mut content, &icon, &dur, args_json, result, inner_width),
        _ => render_generic(
            &mut content,
            tool_name,
            &icon,
            &dur,
            args_json,
            result,
            is_error,
            expanded,
            inner_width,
        ),
    }

    // A single quiet rule keeps each tool call visually distinct without a
    // filled panel. Status glyphs provide a second, non-colour state cue.
    let mut result_lines = Vec::with_capacity(content.len());
    for line in &content {
        let expanded = expand_tabs(line);
        let text = truncate_to_width(&expanded, inner_width, None);
        result_lines.push(format!("│ {text}"));
    }
    result_lines
}

/// Render bash tool: `$ command` header + output tail.
#[expect(
    clippy::too_many_arguments,
    reason = "renderer helper groups display context without heap allocation"
)]
pub(super) fn render_bash(
    lines: &mut Vec<String>,
    icon: &str,
    dur: &str,
    args: &Option<serde_json::Value>,
    result: Option<&str>,
    is_error: bool,
    expanded: bool,
    width: usize,
) {
    let display = crate::protocol::presentation_payloads::tool_display_args(args.as_ref());
    let command = sanitize(display.command.unwrap_or(""));

    // Header: ✓ $ command  42ms
    push_header(
        lines,
        icon,
        &theme::tool_title(&format!("$ {}", command)),
        "",
        dur,
        width,
    );

    if let Some(output) = result {
        if output.is_empty() {
            return;
        }
        let output_lines: Vec<&str> = output.lines().collect();
        let total = output_lines.len();

        let color_fn: fn(&str) -> String = if is_error {
            theme::error
        } else {
            theme::tool_output
        };

        if expanded {
            // Show all lines.
            push_full_output(lines, &output_lines, color_fn, width);
        } else {
            // Show tail (last N source lines) with a bounded number of wrapped
            // rows, so one huge logical line cannot flood the collapsed tool
            // card. If the output has few source lines we still use the same
            // bounded collapsed preview path.
            let shown = BASH_PREVIEW_LINES.min(total);
            let hidden = total.saturating_sub(shown);
            if hidden > 0 {
                let hint = format!("... ({} earlier lines, Ctrl+O to expand)", hidden);
                push_dim_wrapped(lines, &hint, width);
            }
            push_preview(
                lines,
                &output_lines[total - shown..],
                shown,
                color_fn,
                width,
            );
        }
    }
}

/// Render read tool: `read path` + content preview (head).
#[expect(
    clippy::too_many_arguments,
    reason = "renderer helper groups display context without heap allocation"
)]
pub(super) fn render_read(
    lines: &mut Vec<String>,
    icon: &str,
    dur: &str,
    args: &Option<serde_json::Value>,
    result: Option<&str>,
    expanded: bool,
    width: usize,
) {
    let path = extract_path(args);

    // Header: ✓ read path  42ms
    push_header(
        lines,
        icon,
        &theme::tool_title("read"),
        &theme::accent(&path),
        dur,
        width,
    );

    if let Some(content) = result {
        render_file_preview(lines, content, expanded, width, false);
    }
}

/// Render write tool: `write path` + content preview (head).
#[expect(
    clippy::too_many_arguments,
    reason = "renderer helper groups display context without heap allocation"
)]
pub(super) fn render_write(
    lines: &mut Vec<String>,
    icon: &str,
    dur: &str,
    args: &Option<serde_json::Value>,
    result: Option<&str>,
    expanded: bool,
    width: usize,
) {
    let path = extract_path(args);

    // For write, the content is in the args, not the result.
    let display = crate::protocol::presentation_payloads::tool_display_args(args.as_ref());
    let content = display.content.unwrap_or("");

    // Header: ✓ write path  42ms
    push_header(
        lines,
        icon,
        &theme::tool_title("write"),
        &theme::accent(&path),
        dur,
        width,
    );

    if !content.is_empty() {
        render_file_preview(lines, content, expanded, width, false);
    } else if let Some(r) = result {
        // Show result (e.g. error message).
        if !r.is_empty() {
            lines.push(truncate_to_width(&theme::tool_output(r), width, None));
        }
    }
}

/// Render edit tool: `edit path` + diff preview.
#[expect(
    clippy::too_many_arguments,
    reason = "renderer helper groups display context without heap allocation"
)]
pub(super) fn render_edit(
    lines: &mut Vec<String>,
    icon: &str,
    dur: &str,
    args: &Option<serde_json::Value>,
    result: Option<&str>,
    is_error: bool,
    expanded: bool,
    width: usize,
) {
    let path = extract_path(args);

    // Header: ✓ edit path  42ms
    push_header(
        lines,
        icon,
        &theme::tool_title("edit"),
        &theme::accent(&path),
        dur,
        width,
    );

    if let Some(output) = result {
        if is_error {
            lines.push(truncate_to_width(&theme::error(output), width, None));
        } else if !output.is_empty() {
            // Skip "Successfully edited ..." and blank lines / code fences —
            // the header already shows the tool name + path.
            let diff_lines: Vec<&str> = output
                .lines()
                .filter(|l| {
                    !l.starts_with("Successfully edited") && !l.starts_with("```") && !l.is_empty()
                })
                .collect();
            if expanded {
                push_full_output(lines, &diff_lines, style_diff_line, width);
            } else {
                push_preview(
                    lines,
                    &diff_lines,
                    FILE_PREVIEW_LINES,
                    style_diff_line,
                    width,
                );
            }
        }
    }
}

/// Render subagent tools (spawn, agent_cmd) with distinct styling.
#[expect(
    clippy::too_many_arguments,
    reason = "renderer helper groups display context without heap allocation"
)]
pub(super) fn render_subagent(
    lines: &mut Vec<String>,
    tool_name: &str,
    icon: &str,
    dur: &str,
    args: &Option<serde_json::Value>,
    result: Option<&str>,
    is_error: bool,
    width: usize,
) {
    let display = crate::protocol::presentation_payloads::tool_display_args(args.as_ref());
    let (header_detail, _agent_label) = match tool_name {
        "spawn" => {
            let agent = sanitize(display.agent_id.unwrap_or("agent"));
            let task = sanitize(display.task.unwrap_or(""));
            let detail = if task.is_empty() {
                agent.clone()
            } else {
                format!(
                    "{} — {}",
                    agent,
                    crate::components::utils::truncate_to_width(&task, 50, Some("..."))
                )
            };
            (detail, Some(agent))
        }
        "agent_cmd" => {
            let command = sanitize(display.command.unwrap_or("?"));
            let agent_id = sanitize(display.agent_id.unwrap_or("?"));
            (format!("{} → {}", command, agent_id), Some(agent_id))
        }
        _ => (String::new(), None),
    };

    // Header: ✓ spawn reviewer — Review PR  42ms
    push_header(
        lines,
        icon,
        &theme::magenta(&theme::tool_title(tool_name)),
        &theme::tool_output(&header_detail),
        dur,
        width,
    );

    // Show result preview — for spawn, show agent output; for agent_cmd, show response.
    if let Some(output) = result {
        if !output.is_empty() {
            let color_fn: fn(&str) -> String = if is_error {
                theme::error
            } else {
                theme::tool_output
            };
            let output_lines: Vec<&str> = output.lines().collect();
            push_preview(lines, &output_lines, FILE_PREVIEW_LINES, color_fn, width);
        }
    }
}

/// Render workflow tool: styled action + result summary.
pub(super) fn render_workflow(
    lines: &mut Vec<String>,
    icon: &str,
    dur: &str,
    args: &Option<serde_json::Value>,
    result: Option<&str>,
    width: usize,
) {
    let display = crate::protocol::presentation_payloads::tool_display_args(args.as_ref());
    let action = display.action.unwrap_or("workflow");
    let detail = match action {
        "check" | "uncheck" | "skip" => format!(
            "{action}{}",
            display
                .step
                .map(|n| format!(" step {n}"))
                .unwrap_or_default()
        ),
        "select_template" => format!("select_template {}", display.template.unwrap_or("?")),
        "set_issue" => format!(
            "set_issue{}",
            display
                .issue_number
                .map(|n| format!(" #{n}"))
                .unwrap_or_default()
        ),
        _ => action.to_string(),
    };

    push_header(
        lines,
        icon,
        &theme::bold(&theme::accent("workflow")),
        &theme::dim(&detail),
        dur,
        width,
    );

    if let Some(text) = result {
        let preview: String = text
            .lines()
            .next()
            .unwrap_or("")
            .chars()
            .take(120)
            .collect();
        if !preview.is_empty() {
            lines.push(truncate_to_width(
                &theme::dim(&format!("  {preview}")),
                width,
                None,
            ));
        }
    }
}

/// Render generic/unknown tools.
#[expect(
    clippy::too_many_arguments,
    reason = "renderer helper groups display context without heap allocation"
)]
pub(super) fn render_generic(
    lines: &mut Vec<String>,
    tool_name: &str,
    icon: &str,
    dur: &str,
    args: &Option<serde_json::Value>,
    result: Option<&str>,
    is_error: bool,
    expanded: bool,
    width: usize,
) {
    // Extract most useful arg for summary.
    let summary = if let Some(v) = args {
        extract_best_arg(v)
    } else {
        String::new()
    };

    // Header: ✓ tool_name summary  42ms
    push_header(
        lines,
        icon,
        &theme::tool_title(tool_name),
        &theme::dim(&summary),
        dur,
        width,
    );

    if let Some(output) = result {
        if !output.is_empty() {
            render_file_preview(lines, output, expanded, width, is_error);
        }
    }
}

// ── Shared rendering helpers ─────────────────────────────────────────────────

/// Push a tool header line of the form `icon title detail  dur`.
///
/// `detail` and `dur` are omitted when empty, so callers that have no detail
/// (e.g. bash, whose title already carries the `$ command`) share one idiom.
pub(super) fn push_header(
    lines: &mut Vec<String>,
    icon: &str,
    title: &str,
    detail: &str,
    dur: &str,
    width: usize,
) {
    let header = if detail.is_empty() {
        format!("{icon} {title}{dur}")
    } else {
        format!("{icon} {title} {detail}{dur}")
    };
    lines.extend(wrap_tool_line(&header, width));
}

const TOOL_HEADER_MAX_ROWS: usize = 8;

fn wrap_tool_line(line: &str, width: usize) -> Vec<String> {
    let mut rows: Vec<String> = crate::components::utils::wrap_text(line, width)
        .into_iter()
        .map(|segment| truncate_to_width(&segment, width, None))
        .collect();
    if rows.len() > TOOL_HEADER_MAX_ROWS {
        rows.truncate(TOOL_HEADER_MAX_ROWS);
        if let Some(last) = rows.last_mut() {
            *last = truncate_to_width("… header truncated", width, None);
        }
    }
    rows
}

/// Wrapped rows a single over-long source line may occupy in a collapsed
/// preview before being cut, so one huge logical line cannot flood the card.
const PREVIEW_ROWS_PER_LINE: usize = 4;

/// Push a dimmed hint, wrapped so it stays readable at narrow widths.
fn push_dim_wrapped(lines: &mut Vec<String>, hint: &str, width: usize) {
    for segment in crate::components::utils::wrap_text(hint, width) {
        lines.push(theme::dim(&segment));
    }
}

fn push_wrapped_output(
    lines: &mut Vec<String>,
    text: &str,
    color_fn: fn(&str) -> String,
    width: usize,
) {
    for segment in crate::components::utils::wrap_text(text, width) {
        lines.push(color_fn(&segment));
    }
}

/// Push ALL of `content_lines`, sanitized and wrapped to `width` with no row
/// budget — the expanded-view counterpart of [`push_preview`].
pub(super) fn push_full_output(
    lines: &mut Vec<String>,
    content_lines: &[&str],
    color_fn: fn(&str) -> String,
    width: usize,
) {
    for line in content_lines {
        let sanitized = crate::components::ansi::sanitize_control(line);
        push_wrapped_output(lines, &sanitized, color_fn, width);
    }
}

fn push_wrapped_output_limited(
    lines: &mut Vec<String>,
    text: &str,
    color_fn: fn(&str) -> String,
    width: usize,
    max_rows: usize,
) -> (usize, bool) {
    if max_rows == 0 {
        return (0, true);
    }
    let max_width = width.saturating_mul(max_rows).max(width);
    let mut bounded = truncate_to_width(text, max_width, Some("..."));
    // truncate_to_width appends an SGR reset on truncation, but this text is
    // already control-stripped and wrap_text would split the escape across
    // rows, leaking a bare ESC into the rendered line — drop it.
    if let Some(stripped) = bounded.strip_suffix("\x1b[0m") {
        bounded.truncate(stripped.len());
    }
    let mut truncated = bounded != text;
    let mut pushed = 0;
    for segment in crate::components::utils::wrap_text(&bounded, width) {
        if pushed == max_rows {
            // Word-wrap can spend fewer columns per row than the width bound
            // assumed, leaving segments beyond the row budget.
            truncated = true;
            break;
        }
        lines.push(color_fn(&segment));
        pushed += 1;
    }
    (pushed, truncated)
}

/// Push a head preview of `content_lines`: the first `max` lines, wrapped to
/// `width` and styled by `color_fn`, under a shared row budget of
/// [`PREVIEW_ROWS_PER_LINE`] rows per shown line. When lines were hidden or
/// the budget cut wrapped rows, a dimmed "… (N more lines, Ctrl+O to expand)"
/// (or "… (output truncated, …)") hint follows. Centralises the preview idiom
/// repeated across the file/diff/subagent/generic renderers.
pub(super) fn push_preview(
    lines: &mut Vec<String>,
    content_lines: &[&str],
    max: usize,
    color_fn: fn(&str) -> String,
    width: usize,
) {
    let total = content_lines.len();
    let shown = max.min(total);
    let mut rows_left = shown.saturating_mul(PREVIEW_ROWS_PER_LINE);
    let mut truncated = total > shown;
    for line in &content_lines[..shown] {
        if rows_left == 0 {
            truncated = true;
            break;
        }
        // Strip sub-agent/extension-influenced terminal control sequences from
        // the result body before colouring (#865 security review): otherwise a
        // malicious sub-agent could inject ANSI/OSC escapes (cursor control,
        // title/clipboard spoofing) into the parent operator's terminal.
        let sanitized = crate::components::ansi::sanitize_control(line);
        let (pushed, line_truncated) =
            push_wrapped_output_limited(lines, &sanitized, color_fn, width, rows_left);
        rows_left -= pushed;
        if line_truncated {
            truncated = true;
        }
    }
    if truncated {
        let hidden = total - shown;
        let hint = if hidden == 0 {
            "... (output truncated, Ctrl+O to expand)".to_string()
        } else {
            format!("... ({} more lines, Ctrl+O to expand)", hidden)
        };
        push_dim_wrapped(lines, &hint, width);
    }
}

/// Render a file content preview — first N lines with count of remaining.
pub(super) fn render_file_preview(
    lines: &mut Vec<String>,
    content: &str,
    expanded: bool,
    width: usize,
    is_error: bool,
) {
    let content_lines: Vec<&str> = content.lines().collect();
    let color_fn: fn(&str) -> String = if is_error {
        theme::error
    } else {
        theme::tool_output
    };
    if expanded {
        push_full_output(lines, &content_lines, color_fn, width);
    } else {
        push_preview(lines, &content_lines, FILE_PREVIEW_LINES, color_fn, width);
    }
}

/// Extract the file path from tool args (tries "path", "file_path").
pub(super) fn extract_path(args: &Option<serde_json::Value>) -> String {
    let display = crate::protocol::presentation_payloads::tool_display_args(args.as_ref());
    display.path.map(sanitize).unwrap_or_default()
}

/// Extract the most informative arg value for display.
pub(super) fn extract_best_arg(v: &serde_json::Value) -> String {
    let display = crate::protocol::presentation_payloads::tool_display_args(Some(v));
    [
        display.command,
        display.path,
        display.query,
        display.url,
        display.content,
        display.old_text,
    ]
    .into_iter()
    .flatten()
    .next()
    .map(|value| {
        sanitize(&crate::components::utils::truncate_to_width(
            value,
            60,
            Some("..."),
        ))
    })
    .unwrap_or_default()
}

/// Style a diff line with color (green for +, red for -, cyan for @@).
pub(super) fn style_diff_line(line: &str) -> String {
    if line.starts_with('+') {
        theme::green(line)
    } else if line.starts_with('-') {
        theme::red(line)
    } else {
        theme::tool_output(line)
    }
}

/// Sanitize a string by stripping terminal control sequences.
pub(super) fn sanitize(s: &str) -> String {
    crate::components::ansi::sanitize_control(s)
}

#[cfg(test)]
pub(super) fn truncate_with_ellipsis(s: &str, max_chars: usize) -> String {
    crate::components::utils::truncate_to_width(s, max_chars, Some("..."))
}

/// Expand tab characters to spaces using 8-column tab stops, ANSI-aware.
///
/// Tabs in tool output (e.g. source code, `git` output) otherwise advance the
/// terminal cursor without painting the box background, leaving dark gaps, and
/// `visible_width` counts them as zero columns. Expanding to spaces against the
/// visible column position keeps the background contiguous and width math
/// correct. ANSI escape sequences are passed through without consuming columns.
pub(super) fn expand_tabs(s: &str) -> Cow<'_, str> {
    const TAB_STOP: usize = 8;
    if !s.contains('\t') {
        return Cow::Borrowed(s);
    }
    let mut out = String::with_capacity(s.len());
    let mut col = 0;
    for seg in crate::components::ansi::ansi_segments_legacy_csi(s) {
        match seg {
            AnsiSegment::Escape(esc) => out.push_str(esc),
            AnsiSegment::Text(text) => {
                for ch in text.chars() {
                    if ch == '\t' {
                        let spaces = TAB_STOP - (col % TAB_STOP);
                        for _ in 0..spaces {
                            out.push(' ');
                        }
                        col += spaces;
                    } else {
                        out.push(ch);
                        col += unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
                    }
                }
            }
        }
    }
    Cow::Owned(out)
}
