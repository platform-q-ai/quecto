use std::borrow::Cow;

use super::*;
use crate::interface::ansi::AnsiSegment;

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
    // Select background color based on state.
    let bg_fn: fn(&str) -> String = if result.is_none() {
        theme::tool_pending_bg
    } else if is_error {
        theme::tool_error_bg
    } else {
        theme::tool_success_bg
    };

    // Build content lines (without background — applied after).
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

    // Apply background color and padding to every line,
    // with an empty bg line above and below to frame the box.
    let empty_bg_line = theme::apply_bg("", width, bg_fn);
    let mut result_lines = Vec::with_capacity(content.len() + 2);
    result_lines.push(empty_bg_line.clone());
    for line in &content {
        // Expand tabs to spaces before width/background handling. A literal
        // tab advances the terminal cursor without painting the background,
        // leaving dark gaps mid-box, and `visible_width` counts it as zero
        // columns, throwing off the padding math. Expanding here keeps the
        // box background contiguous and width calculations correct.
        let expanded = expand_tabs(line);
        let padded = format!(" {} ", truncate_to_width(&expanded, inner_width, None));
        result_lines.push(theme::apply_bg(&padded, width, bg_fn));
    }
    result_lines.push(empty_bg_line);
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
    let command = args
        .as_ref()
        .and_then(|v| v.get("command").and_then(|c| c.as_str()))
        .unwrap_or("");
    let command = sanitize(command);

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

        if expanded || total <= BASH_PREVIEW_LINES {
            // Show all lines.
            for line in &output_lines {
                lines.push(truncate_to_width(&color_fn(line), width, None));
            }
        } else {
            // Show tail (last N lines) with count of hidden earlier lines.
            let hidden = total - BASH_PREVIEW_LINES;
            lines.push(theme::dim(&format!(
                "... ({} earlier lines, Ctrl+O to expand)",
                hidden
            )));
            for line in &output_lines[hidden..] {
                lines.push(truncate_to_width(&color_fn(line), width, None));
            }
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
    let content = args
        .as_ref()
        .and_then(|v| v.get("content").and_then(|c| c.as_str()))
        .unwrap_or("");

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
            let max = if expanded {
                diff_lines.len()
            } else {
                FILE_PREVIEW_LINES
            };
            push_preview(lines, &diff_lines, max, style_diff_line, width);
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
    let (header_detail, _agent_label) = if let Some(v) = args {
        match tool_name {
            "spawn" => {
                let agent = sanitize(
                    v.get("agent_id")
                        .and_then(|v| v.as_str())
                        .unwrap_or("agent"),
                );
                let task = sanitize(v.get("task").and_then(|v| v.as_str()).unwrap_or(""));
                let detail = if task.is_empty() {
                    agent.clone()
                } else {
                    format!(
                        "{} — {}",
                        agent,
                        crate::interface::utils::truncate_chars_with_ellipsis(&task, 50, "...")
                    )
                };
                (detail, Some(agent))
            }
            "agent_cmd" => {
                let command = sanitize(v.get("command").and_then(|v| v.as_str()).unwrap_or("?"));
                let agent_id = sanitize(v.get("agent_id").and_then(|v| v.as_str()).unwrap_or("?"));
                (format!("{} → {}", command, agent_id), Some(agent_id))
            }
            _ => (String::new(), None),
        }
    } else {
        (String::new(), None)
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
    let action = args
        .as_ref()
        .and_then(|v| v.get("action").and_then(|a| a.as_str()))
        .unwrap_or("workflow");

    let detail = match action {
        "check" | "uncheck" | "skip" => {
            let step = args
                .as_ref()
                .and_then(|v| v.get("step"))
                .and_then(|s| s.as_u64())
                .map(|n| format!(" step {n}"))
                .unwrap_or_default();
            format!("{action}{step}")
        }
        "select_template" => {
            let tpl = args
                .as_ref()
                .and_then(|v| v.get("template").and_then(|t| t.as_str()))
                .unwrap_or("?");
            format!("select_template {tpl}")
        }
        "set_issue" => {
            let num = args
                .as_ref()
                .and_then(|v| v.get("issueNumber"))
                .and_then(|n| n.as_u64())
                .map(|n| format!(" #{n}"))
                .unwrap_or_default();
            format!("set_issue{num}")
        }
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
    lines.push(truncate_to_width(&header, width, None));
}

/// Push a head preview of `content_lines`: the first `max` lines styled by
/// `color_fn`, followed by a dimmed "… (N more lines, Ctrl+O to expand)" hint
/// when the content was truncated. Centralises the preview idiom repeated
/// across the file/diff/subagent/generic renderers.
pub(super) fn push_preview(
    lines: &mut Vec<String>,
    content_lines: &[&str],
    max: usize,
    color_fn: fn(&str) -> String,
    width: usize,
) {
    let total = content_lines.len();
    let shown = max.min(total);
    for line in &content_lines[..shown] {
        // Strip sub-agent/extension-influenced terminal control sequences from
        // the result body before colouring (#865 security review): otherwise a
        // malicious sub-agent could inject ANSI/OSC escapes (cursor control,
        // title/clipboard spoofing) into the parent operator's terminal, since
        // truncate_to_width preserves escape sequences verbatim.
        lines.push(truncate_to_width(
            &color_fn(&crate::interface::ansi::sanitize_control(line)),
            width,
            None,
        ));
    }
    if total > shown {
        lines.push(theme::dim(&format!(
            "... ({} more lines, Ctrl+O to expand)",
            total - shown
        )));
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
    let total = content_lines.len();
    let color_fn: fn(&str) -> String = if is_error {
        theme::error
    } else {
        theme::tool_output
    };
    let max = if expanded { total } else { FILE_PREVIEW_LINES };
    push_preview(lines, &content_lines, max, color_fn, width);
}

/// Extract the file path from tool args (tries "path", "file_path").
pub(super) fn extract_path(args: &Option<serde_json::Value>) -> String {
    args.as_ref()
        .and_then(|v| {
            v.get("path")
                .or_else(|| v.get("file_path"))
                .and_then(|p| p.as_str())
        })
        .map(sanitize)
        .unwrap_or_default()
}

/// Extract the most informative arg value for display.
pub(super) fn extract_best_arg(v: &serde_json::Value) -> String {
    for key in &["command", "path", "query", "url", "content", "oldText"] {
        if let Some(val) = v.get(key).and_then(|v| v.as_str()) {
            return sanitize(&crate::interface::utils::truncate_chars_with_ellipsis(
                val, 60, "...",
            ));
        }
    }
    String::new()
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
    crate::interface::ansi::sanitize_control(s)
}

#[cfg(test)]
pub(super) fn truncate_with_ellipsis(s: &str, max_chars: usize) -> String {
    crate::interface::utils::truncate_chars_with_ellipsis(s, max_chars, "...")
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
    for seg in crate::interface::ansi::ansi_segments_legacy_csi(s) {
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
