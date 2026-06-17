//! Chat display component — renders conversation history.
//!
//! Displays user messages, assistant responses (with streaming), and
//! tool execution results in a scrollable vertical layout.
//!
//! Tool rendering uses Pi-style background-colored boxes with tool-specific
//! formatting (#510): bash shows `$ command` + output tail, read/write show
//! file path + content preview, edit shows diff.

use crate::interface::component::Component;
use crate::interface::components::markdown::Markdown;
use crate::interface::theme;
#[cfg(test)]
use crate::interface::utils::visible_width;
use crate::interface::utils::{truncate_to_width, wrap_text};

/// Number of output lines shown for bash in collapsed mode (tail).
const BASH_PREVIEW_LINES: usize = 5;
/// Number of content lines shown for read/write in collapsed mode (head).
const FILE_PREVIEW_LINES: usize = 10;

/// A single chat entry (user message, assistant message, tool execution, or status).
#[derive(Debug, Clone)]
pub enum ChatEntry {
    User {
        text: String,
    },
    Assistant {
        text: String,
        /// Whether this message is still being streamed.
        streaming: bool,
    },
    /// Unified tool execution — created on ToolStart, updated in place on ToolEnd.
    ToolExecution {
        tool_call_id: String,
        tool_name: String,
        args: String,
        /// Cached parsed args (avoids re-parsing JSON on every render).
        parsed_args: Option<serde_json::Value>,
        result: Option<String>,
        is_error: bool,
        duration_ms: Option<u64>,
    },
    Status {
        text: String,
    },
}

/// Chat display component.
#[derive(Debug)]
pub struct Chat {
    entries: Vec<ChatEntry>,
    /// Scroll offset from the bottom (0 = at bottom, showing most recent).
    scroll_offset: usize,
    /// Width used for the most recent full chat render.
    last_render_width: Option<usize>,
    /// Full line count from the most recent chat render, before viewport scrolling.
    last_render_line_count: usize,
    /// Available chat viewport height, when the parent layout knows it.
    viewport_height: Option<usize>,
    /// Global tool expand state (toggled by Ctrl+O).
    pub tool_expanded: bool,
}

impl Default for Chat {
    fn default() -> Self {
        Self::new()
    }
}

impl Chat {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
            scroll_offset: 0,
            last_render_width: None,
            last_render_line_count: 0,
            viewport_height: None,
            tool_expanded: false,
        }
    }

    pub fn add_entry(&mut self, entry: ChatEntry) {
        self.entries.push(entry);
        self.scroll_offset = 0;
    }

    /// Append streaming token to the last assistant message, or create one.
    pub fn append_token(&mut self, token: &str) {
        if let Some(ChatEntry::Assistant { text, streaming }) = self.entries.last_mut() {
            if *streaming {
                text.push_str(token);
                return;
            }
        }
        self.entries.push(ChatEntry::Assistant {
            text: token.to_string(),
            streaming: true,
        });
    }

    /// Finalize the current streaming message.
    pub fn finalize_assistant(&mut self) {
        if let Some(ChatEntry::Assistant { streaming, .. }) = self.entries.last_mut() {
            *streaming = false;
        }
    }

    /// Start a tool execution — creates a ToolExecution entry.
    pub fn start_tool(&mut self, tool_call_id: String, tool_name: String, args: String) {
        let parsed_args = serde_json::from_str(&args).ok();
        self.entries.push(ChatEntry::ToolExecution {
            tool_call_id,
            tool_name,
            args,
            parsed_args,
            result: None,
            is_error: false,
            duration_ms: None,
        });
    }

    /// Complete a tool execution — updates existing entry in place.
    #[allow(clippy::too_many_arguments)]
    pub fn complete_tool(
        &mut self,
        tool_call_id: &str,
        result: &str,
        is_error: bool,
        duration_ms: Option<u64>,
    ) {
        // Find the ToolExecution entry and update it.
        for entry in self.entries.iter_mut().rev() {
            if let ChatEntry::ToolExecution {
                tool_call_id: id,
                result: r,
                is_error: e,
                duration_ms: d,
                ..
            } = entry
            {
                if id == tool_call_id {
                    *r = Some(result.to_string());
                    *e = is_error;
                    *d = duration_ms;
                    break;
                }
            }
        }
    }

    /// Toggle expand/collapse on all tool entries.
    pub fn toggle_tool_expand(&mut self) {
        self.tool_expanded = !self.tool_expanded;
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.scroll_offset = 0;
    }

    pub fn set_viewport_height(&mut self, height: usize) {
        self.viewport_height = Some(height);
    }

    pub fn scroll_up(&mut self, amount: usize) {
        self.scroll_offset = self.scroll_offset.saturating_add(amount);
    }

    pub fn scroll_down(&mut self, amount: usize) {
        self.scroll_offset = self.scroll_offset.saturating_sub(amount);
    }

    pub fn entry_count(&self) -> usize {
        self.entries.len()
    }
}

impl Component for Chat {
    #[allow(clippy::cognitive_complexity)]
    fn render(&mut self, width: usize) -> Vec<String> {
        let mut all_lines: Vec<String> = Vec::new();
        let tool_expanded = self.tool_expanded;

        for entry in &self.entries {
            match entry {
                ChatEntry::User { text } => {
                    all_lines.push(String::new());
                    let label = theme::bold(&theme::accent("> "));
                    let wrapped = wrap_text(text, width.saturating_sub(2));
                    for (i, line) in wrapped.iter().enumerate() {
                        if i == 0 {
                            all_lines.push(truncate_to_width(
                                &format!("{}{}", label, line),
                                width,
                                None,
                            ));
                        } else {
                            all_lines.push(truncate_to_width(&format!("  {}", line), width, None));
                        }
                    }
                }
                ChatEntry::Assistant { text, streaming } => {
                    if text.is_empty() && *streaming {
                        continue;
                    }
                    all_lines.push(String::new());
                    let mut md = Markdown::new(text, 0);
                    let md_lines = md.render(width);
                    if md_lines.is_empty() {
                        let wrapped = wrap_text(text, width);
                        for line in &wrapped {
                            all_lines.push(truncate_to_width(line, width, None));
                        }
                    } else {
                        all_lines.extend(md_lines);
                    }
                    if *streaming {
                        if let Some(l) = all_lines.last_mut() {
                            l.push_str(&theme::dim("▌"));
                        }
                    }
                }
                ChatEntry::ToolExecution {
                    tool_name,
                    parsed_args,
                    result,
                    is_error,
                    duration_ms,
                    ..
                } => {
                    let tool_lines = render_tool_execution(
                        tool_name,
                        parsed_args,
                        result.as_deref(),
                        *is_error,
                        *duration_ms,
                        tool_expanded,
                        width,
                    );
                    all_lines.push(String::new()); // spacer
                    all_lines.extend(tool_lines);
                }
                ChatEntry::Status { text } => {
                    all_lines.push(String::new());
                    for status_line in text.lines() {
                        all_lines.push(truncate_to_width(&theme::dim(status_line), width, None));
                    }
                }
            }
        }

        // If the user is scrolled into history, keep the visible viewport anchored
        // while new streaming/tool lines are appended below it. A fixed
        // distance-from-bottom offset would otherwise shrink as content grows,
        // pulling the viewport back toward the live response on every render.
        let full_line_count = all_lines.len();
        if self.scroll_offset > 0
            && self.last_render_width == Some(width)
            && full_line_count > self.last_render_line_count
        {
            self.scroll_offset = self
                .scroll_offset
                .saturating_add(full_line_count - self.last_render_line_count);
        }
        self.last_render_width = Some(width);
        self.last_render_line_count = full_line_count;

        if let Some(height) = self.viewport_height {
            if height == 0 {
                return Vec::new();
            }
            let max_scroll = all_lines.len().saturating_sub(height);
            let effective = self.scroll_offset.min(max_scroll);
            self.scroll_offset = effective;
            let end = all_lines.len().saturating_sub(effective);
            let start = end.saturating_sub(height);
            return all_lines[start..end].to_vec();
        }

        // Apply scroll offset for callers that render the full chat without a
        // known viewport. Clamp to at least one line to avoid blanking out.
        if self.scroll_offset > 0 && !all_lines.is_empty() {
            let max_scroll = all_lines.len().saturating_sub(1);
            let effective = self.scroll_offset.min(max_scroll);
            self.scroll_offset = effective;
            let end = all_lines.len().saturating_sub(effective);
            all_lines.truncate(end.max(1));
        }

        all_lines
    }

    fn invalidate(&mut self) {}
}

// ── Pi-style tool rendering ──────────────────────────────────────────────────

/// Render a complete tool execution block with background color.
#[allow(clippy::too_many_arguments)]
fn render_tool_execution(
    tool_name: &str,
    args_json: &Option<serde_json::Value>,
    result: Option<&str>,
    is_error: bool,
    duration_ms: Option<u64>,
    expanded: bool,
    width: usize,
) -> Vec<String> {
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
        let padded = format!(" {} ", truncate_to_width(line, inner_width, None));
        result_lines.push(theme::apply_bg(&padded, width, bg_fn));
    }
    result_lines.push(empty_bg_line);
    result_lines
}

/// Render bash tool: `$ command` header + output tail.
#[allow(clippy::too_many_arguments)]
fn render_bash(
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
    lines.push(truncate_to_width(
        &format!(
            "{} {}{}",
            icon,
            theme::tool_title(&format!("$ {}", command)),
            dur
        ),
        width,
        None,
    ));

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
#[allow(clippy::too_many_arguments)]
fn render_read(
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
    lines.push(truncate_to_width(
        &format!(
            "{} {} {}{}",
            icon,
            theme::tool_title("read"),
            theme::accent(&path),
            dur
        ),
        width,
        None,
    ));

    if let Some(content) = result {
        render_file_preview(lines, content, expanded, width, false);
    }
}

/// Render write tool: `write path` + content preview (head).
#[allow(clippy::too_many_arguments)]
fn render_write(
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
    lines.push(truncate_to_width(
        &format!(
            "{} {} {}{}",
            icon,
            theme::tool_title("write"),
            theme::accent(&path),
            dur
        ),
        width,
        None,
    ));

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
#[allow(clippy::too_many_arguments)]
fn render_edit(
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
    lines.push(truncate_to_width(
        &format!(
            "{} {} {}{}",
            icon,
            theme::tool_title("edit"),
            theme::accent(&path),
            dur
        ),
        width,
        None,
    ));

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
            let total = diff_lines.len();
            let max = if expanded { total } else { FILE_PREVIEW_LINES };

            for line in diff_lines.iter().take(max) {
                let styled = style_diff_line(line);
                lines.push(truncate_to_width(&styled, width, None));
            }
            if total > max {
                lines.push(theme::dim(&format!(
                    "... ({} more lines, Ctrl+O to expand)",
                    total - max
                )));
            }
        }
    }
}

/// Render subagent tools (spawn, agent_cmd) with distinct styling.
#[allow(clippy::too_many_arguments)]
fn render_subagent(
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
                    format!("{} — {}", agent, truncate_with_ellipsis(&task, 50))
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
    lines.push(truncate_to_width(
        &format!(
            "{} {} {}{}",
            icon,
            theme::magenta(&theme::tool_title(tool_name)),
            theme::tool_output(&header_detail),
            dur,
        ),
        width,
        None,
    ));

    // Show result preview — for spawn, show agent output; for agent_cmd, show response.
    if let Some(output) = result {
        if !output.is_empty() {
            let color_fn: fn(&str) -> String = if is_error {
                theme::error
            } else {
                theme::tool_output
            };
            let output_lines: Vec<&str> = output.lines().collect();
            let max = FILE_PREVIEW_LINES.min(output_lines.len());
            for line in output_lines.iter().take(max) {
                lines.push(truncate_to_width(&color_fn(line), width, None));
            }
            if output_lines.len() > max {
                lines.push(theme::dim(&format!(
                    "... ({} more lines, Ctrl+O to expand)",
                    output_lines.len() - max
                )));
            }
        }
    }
}

/// Render generic/unknown tools.
#[allow(clippy::too_many_arguments)]
/// Render workflow tool: styled action + result summary.
fn render_workflow(
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

    lines.push(truncate_to_width(
        &format!(
            "{} {} {}{}",
            icon,
            theme::bold(&theme::accent("workflow")),
            theme::dim(&detail),
            dur
        ),
        width,
        None,
    ));

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

#[allow(clippy::too_many_arguments)]
fn render_generic(
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
    lines.push(truncate_to_width(
        &format!(
            "{} {} {}{}",
            icon,
            theme::tool_title(tool_name),
            theme::dim(&summary),
            dur,
        ),
        width,
        None,
    ));

    if let Some(output) = result {
        if !output.is_empty() {
            render_file_preview(lines, output, expanded, width, is_error);
        }
    }
}

// ── Shared rendering helpers ─────────────────────────────────────────────────

/// Render a file content preview — first N lines with count of remaining.
#[allow(clippy::too_many_arguments)]
fn render_file_preview(
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

    if expanded || total <= FILE_PREVIEW_LINES {
        for line in &content_lines {
            lines.push(truncate_to_width(&color_fn(line), width, None));
        }
    } else {
        for line in content_lines.iter().take(FILE_PREVIEW_LINES) {
            lines.push(truncate_to_width(&color_fn(line), width, None));
        }
        lines.push(theme::dim(&format!(
            "... ({} more lines, Ctrl+O to expand)",
            total - FILE_PREVIEW_LINES
        )));
    }
}

/// Extract the file path from tool args (tries "path", "file_path").
fn extract_path(args: &Option<serde_json::Value>) -> String {
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
fn extract_best_arg(v: &serde_json::Value) -> String {
    for key in &["command", "path", "query", "url", "content", "oldText"] {
        if let Some(val) = v.get(key).and_then(|v| v.as_str()) {
            return sanitize(&truncate_with_ellipsis(val, 60));
        }
    }
    String::new()
}

/// Style a diff line with color (green for +, red for -, cyan for @@).
fn style_diff_line(line: &str) -> String {
    if line.starts_with('+') {
        theme::green(line)
    } else if line.starts_with('-') {
        theme::red(line)
    } else {
        theme::tool_output(line)
    }
}

/// Sanitize a string by stripping control characters.
fn sanitize(s: &str) -> String {
    s.chars().filter(|c| !c.is_control()).collect()
}

/// Truncate a string to max_chars, appending "..." if truncated.
fn truncate_with_ellipsis(s: &str, max_chars: usize) -> String {
    if s.chars().count() > max_chars {
        let truncated: String = s.chars().take(max_chars).collect();
        format!("{}...", truncated)
    } else {
        s.to_string()
    }
}

// ══════════════════════════════════════════════════════════════════════════════
// Tests
// ══════════════════════════════════════════════════════════════════════════════

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

    fn render_plain(chat: &mut Chat, width: usize) -> String {
        let lines = chat.render(width);
        lines
            .iter()
            .map(|l| strip_ansi(l))
            .collect::<Vec<_>>()
            .join("\n")
    }

    // ── Basic rendering ──────────────────────────────────────────────

    #[test]
    fn empty_chat_renders_empty() {
        let mut chat = Chat::new();
        assert!(chat.render(80).is_empty());
    }

    #[test]
    fn user_message_rendered() {
        let mut chat = Chat::new();
        chat.add_entry(ChatEntry::User {
            text: "Hello".to_string(),
        });
        let plain = render_plain(&mut chat, 80);
        assert!(
            plain.contains("Hello"),
            "should contain user message: {}",
            plain
        );
    }

    #[test]
    fn streaming_tokens() {
        let mut chat = Chat::new();
        chat.append_token("Hello");
        chat.append_token(" world");
        let plain = render_plain(&mut chat, 80);
        assert!(
            plain.contains("Hello world"),
            "should contain streamed text: {}",
            plain
        );
    }

    #[test]
    fn finalize_assistant_stops_cursor() {
        let mut chat = Chat::new();
        chat.append_token("Done");
        chat.finalize_assistant();
        let lines = chat.render(80);
        let joined = lines.join("");
        assert!(
            !joined.contains('▌'),
            "finalized message should not have cursor"
        );
    }

    #[test]
    fn entry_count() {
        let mut chat = Chat::new();
        assert_eq!(chat.entry_count(), 0);
        chat.add_entry(ChatEntry::User {
            text: "hi".to_string(),
        });
        assert_eq!(chat.entry_count(), 1);
    }

    // ── Unified tool execution (#510) ────────────────────────────────

    #[test]
    fn tool_start_creates_single_entry() {
        let mut chat = Chat::new();
        chat.start_tool(
            "c-1".into(),
            "bash".into(),
            r#"{"command":"ls -la"}"#.into(),
        );
        assert_eq!(chat.entry_count(), 1);
    }

    #[test]
    fn tool_complete_updates_in_place() {
        let mut chat = Chat::new();
        chat.start_tool(
            "c-1".into(),
            "bash".into(),
            r#"{"command":"ls -la"}"#.into(),
        );
        chat.complete_tool("c-1", "file.txt", false, Some(42));
        // Still just one entry, not two.
        assert_eq!(chat.entry_count(), 1);
    }

    #[test]
    fn bash_tool_shows_command_header() {
        let mut chat = Chat::new();
        chat.start_tool(
            "c-1".into(),
            "bash".into(),
            r#"{"command":"ls -la"}"#.into(),
        );
        chat.complete_tool("c-1", "file.txt", false, Some(42));
        let plain = render_plain(&mut chat, 80);
        assert!(plain.contains("$ ls -la"), "should show command: {}", plain);
        assert!(plain.contains("42ms"), "should show duration: {}", plain);
    }

    #[test]
    fn bash_tool_shows_output() {
        let mut chat = Chat::new();
        chat.start_tool("c-1".into(), "bash".into(), r#"{"command":"ls"}"#.into());
        chat.complete_tool("c-1", "file1.txt\nfile2.txt", false, None);
        let plain = render_plain(&mut chat, 80);
        assert!(plain.contains("file1.txt"), "should show output: {}", plain);
    }

    #[test]
    fn bash_collapsed_shows_tail() {
        let mut chat = Chat::new();
        chat.start_tool(
            "c-1".into(),
            "bash".into(),
            r#"{"command":"cargo test"}"#.into(),
        );
        let output: String = (0..50)
            .map(|i| format!("line {}", i))
            .collect::<Vec<_>>()
            .join("\n");
        chat.complete_tool("c-1", &output, false, None);
        let plain = render_plain(&mut chat, 80);
        // Should show the LAST lines (tail), not the first.
        assert!(
            plain.contains("line 49"),
            "should show last line: {}",
            plain
        );
        assert!(
            plain.contains("line 45"),
            "should show near-last line: {}",
            plain
        );
        // Should show count of hidden earlier lines.
        assert!(
            plain.contains("earlier lines"),
            "should show hidden count: {}",
            plain
        );
        assert!(
            plain.contains("Ctrl+O"),
            "should show expand hint: {}",
            plain
        );
        // Should NOT show early lines.
        assert!(
            !plain.contains("line 0"),
            "should NOT show first line: {}",
            plain
        );
    }

    #[test]
    fn bash_expanded_shows_all() {
        let mut chat = Chat::new();
        chat.start_tool(
            "c-1".into(),
            "bash".into(),
            r#"{"command":"cargo test"}"#.into(),
        );
        let output: String = (0..50)
            .map(|i| format!("line {}", i))
            .collect::<Vec<_>>()
            .join("\n");
        chat.complete_tool("c-1", &output, false, None);
        chat.tool_expanded = true;
        let plain = render_plain(&mut chat, 80);
        assert!(
            plain.contains("line 0"),
            "should show first line: {}",
            plain
        );
        assert!(
            plain.contains("line 49"),
            "should show last line: {}",
            plain
        );
        assert!(
            !plain.contains("earlier lines"),
            "should NOT show hidden count: {}",
            plain
        );
    }

    #[test]
    fn read_tool_shows_path_and_content() {
        let mut chat = Chat::new();
        chat.start_tool(
            "c-1".into(),
            "read".into(),
            r#"{"path":"src/main.rs"}"#.into(),
        );
        chat.complete_tool(
            "c-1",
            "fn main() {\n    println!(\"hello\");\n}",
            false,
            None,
        );
        let plain = render_plain(&mut chat, 80);
        assert!(plain.contains("read"), "should show tool name: {}", plain);
        assert!(plain.contains("src/main.rs"), "should show path: {}", plain);
        assert!(
            plain.contains("fn main()"),
            "should show content: {}",
            plain
        );
    }

    #[test]
    fn read_collapsed_shows_head_with_count() {
        let mut chat = Chat::new();
        chat.start_tool("c-1".into(), "read".into(), r#"{"path":"big.rs"}"#.into());
        let content: String = (0..30)
            .map(|i| format!("line {}", i))
            .collect::<Vec<_>>()
            .join("\n");
        chat.complete_tool("c-1", &content, false, None);
        let plain = render_plain(&mut chat, 80);
        assert!(plain.contains("line 0"), "should show first line");
        assert!(plain.contains("line 9"), "should show 10th line");
        assert!(!plain.contains("line 10"), "should NOT show 11th line");
        assert!(plain.contains("more lines"), "should show count: {}", plain);
    }

    #[test]
    fn write_tool_shows_path_and_content() {
        let mut chat = Chat::new();
        chat.start_tool(
            "c-1".into(),
            "write".into(),
            r#"{"path":"src/lib.rs","content":"pub fn hello() {}\n"}"#.into(),
        );
        chat.complete_tool("c-1", "Written successfully", false, None);
        let plain = render_plain(&mut chat, 80);
        assert!(plain.contains("write"), "should show tool name: {}", plain);
        assert!(plain.contains("src/lib.rs"), "should show path: {}", plain);
        assert!(
            plain.contains("pub fn hello"),
            "should show content: {}",
            plain
        );
    }

    #[test]
    fn edit_tool_shows_diff() {
        let mut chat = Chat::new();
        chat.start_tool(
            "c-1".into(),
            "edit".into(),
            r#"{"path":"src/main.rs"}"#.into(),
        );
        chat.complete_tool("c-1", "+added\n-removed\n context", false, None);
        let lines = chat.render(80);
        let joined = lines.join("");
        // Green for added.
        assert!(joined.contains("\x1b[32m"), "added should be green");
        // Red for removed.
        assert!(joined.contains("\x1b[31m"), "removed should be red");
    }

    #[test]
    fn edit_tool_shows_path() {
        let mut chat = Chat::new();
        chat.start_tool(
            "c-1".into(),
            "edit".into(),
            r#"{"path":"src/main.rs"}"#.into(),
        );
        chat.complete_tool("c-1", "+added", false, None);
        let plain = render_plain(&mut chat, 80);
        assert!(plain.contains("edit"), "should show tool name");
        assert!(plain.contains("src/main.rs"), "should show path: {}", plain);
    }

    #[test]
    fn generic_tool_shows_name_and_summary() {
        let mut chat = Chat::new();
        chat.start_tool(
            "c-1".into(),
            "web_fetch".into(),
            r#"{"url":"https://example.com"}"#.into(),
        );
        chat.complete_tool("c-1", "HTML content here", false, None);
        let plain = render_plain(&mut chat, 80);
        assert!(
            plain.contains("web_fetch"),
            "should show tool name: {}",
            plain
        );
        assert!(
            plain.contains("https://example.com"),
            "should show url: {}",
            plain
        );
    }

    // ── Background colors ────────────────────────────────────────────

    #[test]
    fn running_tool_has_pending_bg() {
        let mut chat = Chat::new();
        chat.start_tool("c-1".into(), "bash".into(), r#"{"command":"ls"}"#.into());
        let lines = chat.render(80);
        let tool_lines: Vec<_> = lines.iter().filter(|l| !l.is_empty()).collect();
        assert!(!tool_lines.is_empty());
        assert!(
            tool_lines.iter().any(|l| l.contains(theme::BG_PENDING)),
            "should have pending bg: {:?}",
            tool_lines
        );
    }

    #[test]
    fn success_tool_has_success_bg() {
        let mut chat = Chat::new();
        chat.start_tool("c-1".into(), "bash".into(), r#"{"command":"ls"}"#.into());
        chat.complete_tool("c-1", "ok", false, None);
        let lines = chat.render(80);
        let tool_lines: Vec<_> = lines.iter().filter(|l| !l.is_empty()).collect();
        assert!(
            tool_lines.iter().any(|l| l.contains(theme::BG_SUCCESS)),
            "should have success bg: {:?}",
            tool_lines
        );
    }

    #[test]
    fn error_tool_has_error_bg() {
        let mut chat = Chat::new();
        chat.start_tool("c-1".into(), "bash".into(), r#"{"command":"ls"}"#.into());
        chat.complete_tool("c-1", "command not found", true, None);
        let lines = chat.render(80);
        let tool_lines: Vec<_> = lines.iter().filter(|l| !l.is_empty()).collect();
        assert!(
            tool_lines.iter().any(|l| l.contains(theme::BG_ERROR)),
            "should have error bg: {:?}",
            tool_lines
        );
    }

    // ── Subagent rendering ───────────────────────────────────────────

    #[test]
    fn spawn_tool_shows_agent_label() {
        let mut chat = Chat::new();
        chat.start_tool(
            "c-1".into(),
            "spawn".into(),
            r#"{"agent_id":"reviewer","task":"Review PR"}"#.into(),
        );
        let plain = render_plain(&mut chat, 80);
        assert!(plain.contains("reviewer"), "should show agent: {}", plain);
        assert!(plain.contains("Review PR"), "should show task: {}", plain);
    }

    #[test]
    fn agent_cmd_shows_command_and_target() {
        let mut chat = Chat::new();
        chat.start_tool(
            "c-1".into(),
            "agent_cmd".into(),
            r#"{"command":"prompt","agent_id":"reviewer"}"#.into(),
        );
        let plain = render_plain(&mut chat, 80);
        assert!(plain.contains("prompt"), "should show command: {}", plain);
        assert!(plain.contains("reviewer"), "should show agent: {}", plain);
    }

    // ── Width compliance ─────────────────────────────────────────────

    #[test]
    fn tool_lines_respect_width() {
        let mut chat = Chat::new();
        chat.start_tool(
            "c-1".into(),
            "bash".into(),
            r#"{"command":"very long command string here"}"#.into(),
        );
        let output: String = (0..20)
            .map(|i| format!("line {} with some content here", i))
            .collect::<Vec<_>>()
            .join("\n");
        chat.complete_tool("c-1", &output, false, Some(42));
        let lines = chat.render(40);
        for line in &lines {
            assert!(
                visible_width(line) <= 40,
                "line exceeds width: {} (width={})",
                strip_ansi(line),
                visible_width(line)
            );
        }
    }

    #[test]
    fn respects_width() {
        let mut chat = Chat::new();
        chat.add_entry(ChatEntry::User {
            text: "A very long message that should be wrapped to fit within the width constraint properly".to_string(),
        });
        let lines = chat.render(40);
        for line in &lines {
            assert!(
                visible_width(line) <= 40,
                "line exceeds width: {} (width={})",
                strip_ansi(line),
                visible_width(line)
            );
        }
    }

    // ── Scroll tests (from #500) ─────────────────────────────────────

    #[test]
    fn scroll_offset_not_artificially_clamped() {
        let mut chat = Chat::new();
        let long_text = (0..100)
            .map(|i| format!("Line {}", i))
            .collect::<Vec<_>>()
            .join("\n");
        chat.add_entry(ChatEntry::Assistant {
            text: long_text,
            streaming: false,
        });
        chat.scroll_up(50);
        assert!(
            chat.scroll_offset >= 50,
            "scroll_offset should be at least 50, got {}",
            chat.scroll_offset
        );
    }

    fn chat_with_streaming_history() -> Chat {
        let mut chat = Chat::new();
        for i in 0..30 {
            chat.add_entry(ChatEntry::User {
                text: format!("history line {i}"),
            });
        }
        chat.append_token("initial streamed response");
        chat
    }

    #[test]
    fn scroll_down_from_scrolled_position() {
        let mut chat = Chat::new();
        for i in 0..30 {
            chat.add_entry(ChatEntry::User {
                text: format!("Message number {}", i),
            });
        }
        let full = chat.render(80);
        chat.scroll_up(15);
        let scrolled_up = chat.render(80);
        assert!(scrolled_up.len() < full.len());
        chat.scroll_down(10);
        let after_down = chat.render(80);
        assert!(
            after_down.len() > scrolled_up.len(),
            "scrolling down should show more lines"
        );
    }

    #[test]
    fn scrolled_streaming_viewport_stays_anchored_when_tokens_add_lines() {
        let mut chat = chat_with_streaming_history();

        let height = 10;
        chat.set_viewport_height(height);
        chat.scroll_up(15);
        let before = chat.render(80);

        chat.append_token("\nnew streamed line 1\nnew streamed line 2\nnew streamed line 3");
        let after = chat.render(80);

        assert_eq!(
            after, before,
            "streaming output should not drag a scrolled viewport toward the bottom"
        );
        assert!(
            chat.scroll_offset > 15,
            "scroll offset should grow with streamed content while scrolled away from bottom"
        );
    }

    #[test]
    fn scrolled_viewport_stays_anchored_when_tool_entries_arrive() {
        let mut chat = chat_with_streaming_history();

        let height = 10;
        chat.set_viewport_height(height);
        chat.scroll_up(15);
        let before = chat.render(80);

        chat.start_tool(
            "tool-1".into(),
            "bash".into(),
            r#"{"command":"echo hi"}"#.into(),
        );
        let after = chat.render(80);

        assert_eq!(
            after, before,
            "tool output should not drag a scrolled viewport during an active response"
        );
    }

    #[test]
    fn viewport_clamps_to_full_oldest_page_instead_of_blank() {
        let mut chat = chat_with_streaming_history();
        let height = 10;
        chat.set_viewport_height(height);
        chat.scroll_up(10_000);
        let before = chat.render(80);

        assert_eq!(
            before.len(),
            height,
            "oldest scrollback view should be full"
        );
        assert!(before.iter().any(|line| line.contains("history line 0")));

        chat.append_token("\nnew streamed line 1\nnew streamed line 2\nnew streamed line 3");
        let after = chat.render(80);

        assert_eq!(after.len(), height, "streaming should not shrink to blank");
        assert_eq!(after, before, "oldest full page should remain anchored");
    }

    // ── Integration: server JSON → chat rendering (issue #511) ───────
    //
    // These tests simulate the exact flow in app.rs:
    // 1. Deserialize ToolExecutionEnd JSON from the server
    // 2. Extract result text (same logic as app.rs handle_event)
    // 3. Feed to Chat via start_tool + complete_tool
    // 4. Render and verify content appears inside the bg box

    /// Use the shared extraction function from client.rs.
    fn extract_result_text(result: &serde_json::Value) -> String {
        crate::infrastructure::client::extract_result_text(result)
    }

    /// Check that bg-colored lines (lines containing the bg ANSI code)
    /// include the expected text.
    fn bg_lines_contain(lines: &[String], bg_code: &str, expected: &str) -> bool {
        lines
            .iter()
            .filter(|l| l.contains(bg_code))
            .any(|l| strip_ansi(l).contains(expected))
    }

    #[test]
    fn integration_read_tool_shows_content_in_box() {
        // Server sends this JSON for a read tool result.
        let end_json = serde_json::json!({
            "content": [{"type": "text", "text": "fn main() {\n    println!(\"hello\");\n}"}]
        });
        let result_text = extract_result_text(&end_json);

        let mut chat = Chat::new();
        chat.start_tool(
            "c-1".into(),
            "read".into(),
            r#"{"path":"src/main.rs"}"#.into(),
        );
        chat.complete_tool("c-1", &result_text, false, None);

        let lines = chat.render(80);
        let plain = render_plain(&mut chat, 80);

        // The read header should be in the box.
        assert!(
            bg_lines_contain(&lines, theme::BG_SUCCESS, "read"),
            "bg box should contain 'read' header: {}",
            plain
        );
        assert!(
            bg_lines_contain(&lines, theme::BG_SUCCESS, "src/main.rs"),
            "bg box should contain path: {}",
            plain
        );
        // The file content should also be in the bg box.
        assert!(
            bg_lines_contain(&lines, theme::BG_SUCCESS, "fn main()"),
            "bg box should contain file content 'fn main()': {}",
            plain
        );
    }

    #[test]
    fn integration_read_tool_content_has_background() {
        let end_json = serde_json::json!({
            "content": [{"type": "text", "text": "line 1\nline 2\nline 3"}]
        });
        let result_text = extract_result_text(&end_json);

        let mut chat = Chat::new();
        chat.start_tool("c-1".into(), "read".into(), r#"{"path":"test.txt"}"#.into());
        chat.complete_tool("c-1", &result_text, false, None);

        let lines = chat.render(80);

        // Count how many lines have the success bg — should be more than just
        // the header (header + 3 content lines = at least 4).
        let bg_count = lines
            .iter()
            .filter(|l| l.contains(theme::BG_SUCCESS))
            .count();
        assert!(
            bg_count >= 4,
            "should have at least 4 bg lines (header + 3 content), got {}",
            bg_count
        );
    }

    #[test]
    fn integration_edit_tool_shows_diff_in_box() {
        let end_json = serde_json::json!({
            "content": [{"type": "text", "text": "Applied edit\n+added line\n-removed line\n context"}]
        });
        let result_text = extract_result_text(&end_json);

        let mut chat = Chat::new();
        chat.start_tool(
            "c-1".into(),
            "edit".into(),
            r#"{"path":"src/main.rs"}"#.into(),
        );
        chat.complete_tool("c-1", &result_text, false, None);

        let lines = chat.render(80);
        let plain = render_plain(&mut chat, 80);

        assert!(
            bg_lines_contain(&lines, theme::BG_SUCCESS, "edit"),
            "bg box should contain 'edit' header: {}",
            plain
        );
        // Diff content should be inside the box.
        assert!(
            bg_lines_contain(&lines, theme::BG_SUCCESS, "+added"),
            "bg box should contain diff '+added': {}",
            plain
        );
        assert!(
            bg_lines_contain(&lines, theme::BG_SUCCESS, "-removed"),
            "bg box should contain diff '-removed': {}",
            plain
        );
    }

    #[test]
    fn integration_bash_tool_shows_output_in_box() {
        let end_json = serde_json::json!({
            "content": [{"type": "text", "text": "file1.txt\nfile2.txt\nfile3.txt"}]
        });
        let result_text = extract_result_text(&end_json);

        let mut chat = Chat::new();
        chat.start_tool("c-1".into(), "bash".into(), r#"{"command":"ls"}"#.into());
        chat.complete_tool("c-1", &result_text, false, None);

        let lines = chat.render(80);
        let plain = render_plain(&mut chat, 80);

        assert!(
            bg_lines_contain(&lines, theme::BG_SUCCESS, "$ ls"),
            "bg box should contain '$ ls' header: {}",
            plain
        );
        assert!(
            bg_lines_contain(&lines, theme::BG_SUCCESS, "file1.txt"),
            "bg box should contain bash output: {}",
            plain
        );
    }

    #[test]
    fn integration_write_tool_shows_content_in_box() {
        let end_json = serde_json::json!({
            "content": [{"type": "text", "text": "Written successfully"}]
        });
        let result_text = extract_result_text(&end_json);

        let mut chat = Chat::new();
        chat.start_tool(
            "c-1".into(),
            "write".into(),
            r#"{"path":"out.txt","content":"hello world\nsecond line"}"#.into(),
        );
        chat.complete_tool("c-1", &result_text, false, None);

        let lines = chat.render(80);
        let plain = render_plain(&mut chat, 80);

        assert!(
            bg_lines_contain(&lines, theme::BG_SUCCESS, "write"),
            "bg box should contain 'write' header: {}",
            plain
        );
        assert!(
            bg_lines_contain(&lines, theme::BG_SUCCESS, "out.txt"),
            "bg box should contain path: {}",
            plain
        );
        // Write shows the args content, not the result.
        assert!(
            bg_lines_contain(&lines, theme::BG_SUCCESS, "hello world"),
            "bg box should contain written content: {}",
            plain
        );
    }

    #[test]
    fn integration_pending_tool_has_pending_bg() {
        // Before tool completes, should show pending bg.
        let mut chat = Chat::new();
        chat.start_tool("c-1".into(), "read".into(), r#"{"path":"test.txt"}"#.into());

        let lines = chat.render(80);
        let bg_count = lines
            .iter()
            .filter(|l| l.contains(theme::BG_PENDING))
            .count();
        assert!(
            bg_count >= 1,
            "pending tool should have pending bg, got {} bg lines",
            bg_count
        );
    }

    #[test]
    fn integration_error_tool_has_error_bg() {
        let end_json = serde_json::json!({
            "content": [{"type": "text", "text": "command not found: xyz"}]
        });
        let result_text = extract_result_text(&end_json);

        let mut chat = Chat::new();
        chat.start_tool("c-1".into(), "bash".into(), r#"{"command":"xyz"}"#.into());
        chat.complete_tool("c-1", &result_text, true, None);

        let lines = chat.render(80);
        let bg_count = lines.iter().filter(|l| l.contains(theme::BG_ERROR)).count();
        assert!(
            bg_count >= 1,
            "error tool should have error bg, got {} bg lines",
            bg_count
        );
        assert!(
            bg_lines_contain(&lines, theme::BG_ERROR, "command not found"),
            "error bg box should contain error text"
        );
    }

    // ── Workflow tool rendering ────────────────────────────────────

    #[test]
    fn workflow_tool_renders_action_and_step() {
        let mut chat = Chat::new();
        chat.start_tool(
            "wf-1".into(),
            "workflow".into(),
            r#"{"action":"check","step":3}"#.into(),
        );
        chat.complete_tool("wf-1", "Step 3 checked.", false, Some(5));
        let lines = chat.render(80);
        assert!(
            bg_lines_contain(&lines, theme::BG_SUCCESS, "workflow"),
            "should contain 'workflow' in success bg"
        );
        assert!(
            bg_lines_contain(&lines, theme::BG_SUCCESS, "check"),
            "should contain action 'check'"
        );
    }

    #[test]
    fn workflow_tool_renders_select_template() {
        let mut chat = Chat::new();
        chat.start_tool(
            "wf-2".into(),
            "workflow".into(),
            r#"{"action":"select_template","template":"fix"}"#.into(),
        );
        chat.complete_tool("wf-2", "Selected workflow template 'fix'.", false, Some(10));
        let lines = chat.render(80);
        assert!(
            bg_lines_contain(&lines, theme::BG_SUCCESS, "fix"),
            "should contain template name 'fix'"
        );
    }

    #[test]
    fn workflow_tool_renders_set_issue() {
        let mut chat = Chat::new();
        chat.start_tool(
            "wf-3".into(),
            "workflow".into(),
            r#"{"action":"set_issue","issueNumber":42,"issueTitle":"Auth bug"}"#.into(),
        );
        chat.complete_tool("wf-3", "Active issue set: #42 — Auth bug", false, Some(3));
        let lines = chat.render(80);
        assert!(
            bg_lines_contain(&lines, theme::BG_SUCCESS, "#42"),
            "should contain issue number"
        );
    }
}
