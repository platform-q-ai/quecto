//! Chat display component — renders conversation history.
//!
//! Displays user messages, assistant responses (with streaming), and
//! tool execution results in a scrollable vertical layout.

use crate::component::Component;
use crate::components::markdown::Markdown;
use crate::theme;
#[cfg(test)]
use crate::utils::visible_width;
use crate::utils::{truncate_to_width, wrap_text};

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
    ToolStart {
        tool_call_id: String,
        tool_name: String,
        args: String,
    },
    ToolEnd {
        tool_call_id: String,
        tool_name: String,
        result: String,
        is_error: bool,
        duration_ms: Option<u64>,
    },
    Status {
        text: String,
    },
}

/// Chat display component.
pub struct Chat {
    entries: Vec<ChatEntry>,
    /// Scroll offset from the bottom (0 = at bottom, showing most recent).
    scroll_offset: usize,
}

impl Chat {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
            scroll_offset: 0,
        }
    }

    pub fn add_entry(&mut self, entry: ChatEntry) {
        self.entries.push(entry);
        // Auto-scroll to bottom on new content.
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
        // No active streaming message — create one.
        self.entries.push(ChatEntry::Assistant {
            text: token.to_string(),
            streaming: true,
        });
        self.scroll_offset = 0;
    }

    /// Finalize the current streaming message.
    pub fn finalize_assistant(&mut self) {
        if let Some(ChatEntry::Assistant { streaming, .. }) = self.entries.last_mut() {
            *streaming = false;
        }
    }

    /// Update a tool execution with its result.
    pub fn complete_tool(
        &mut self,
        tool_call_id: &str,
        result: &str,
        is_error: bool,
        duration_ms: Option<u64>,
    ) {
        // Find the ToolStart entry and replace or add a ToolEnd.
        let tool_name = self
            .entries
            .iter()
            .rev()
            .find_map(|e| match e {
                ChatEntry::ToolStart {
                    tool_call_id: id,
                    tool_name,
                    ..
                } if id == tool_call_id => Some(tool_name.clone()),
                _ => None,
            })
            .unwrap_or_default();

        self.entries.push(ChatEntry::ToolEnd {
            tool_call_id: tool_call_id.to_string(),
            tool_name,
            result: result.to_string(),
            is_error,
            duration_ms,
        });
        self.scroll_offset = 0;
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.scroll_offset = 0;
    }

    pub fn scroll_up(&mut self, amount: usize) {
        self.scroll_offset = self.scroll_offset.saturating_add(amount);
        // Clamp to entry count to prevent unbounded growth.
        self.scroll_offset = self.scroll_offset.min(self.entries.len().saturating_mul(5));
    }

    pub fn scroll_down(&mut self, amount: usize) {
        self.scroll_offset = self.scroll_offset.saturating_sub(amount);
    }

    pub fn entry_count(&self) -> usize {
        self.entries.len()
    }
}

impl Component for Chat {
    fn render(&mut self, width: usize) -> Vec<String> {
        let mut all_lines: Vec<String> = Vec::new();

        for entry in &self.entries {
            match entry {
                ChatEntry::User { text } => {
                    all_lines.push(String::new()); // spacer
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
                        continue; // Don't render empty streaming placeholder.
                    }
                    all_lines.push(String::new()); // spacer
                    // Render assistant content as markdown.
                    let mut md = Markdown::new(text, 0);
                    let md_lines = md.render(width);
                    if md_lines.is_empty() {
                        // Fallback: wrap raw text if markdown produced nothing.
                        let wrapped = wrap_text(text, width);
                        for line in &wrapped {
                            all_lines.push(truncate_to_width(line, width, None));
                        }
                    } else {
                        all_lines.extend(md_lines);
                    }
                    if *streaming {
                        // Show cursor indicator for streaming.
                        all_lines.last_mut().map(|l| l.push_str(&theme::dim("▌")));
                    }
                }
                ChatEntry::ToolStart {
                    tool_name, args, ..
                } => {
                    if is_subagent_tool(tool_name) {
                        // Subagent-style: ◈ spawn agent_label — task
                        let args_summary = summarize_subagent_args(tool_name, args);
                        let line = format!(
                            "  {} {} {}",
                            theme::magenta(SUBAGENT_ICON_RUNNING),
                            theme::magenta(tool_name),
                            theme::dim(&args_summary)
                        );
                        all_lines.push(truncate_to_width(&line, width, None));
                    } else {
                        // Regular Pi-style: ⠋ tool_name args_summary
                        let args_summary = summarize_tool_args(args);
                        let line = format!(
                            "  {} {} {}",
                            theme::spinner("⠋"),
                            theme::tool_name(tool_name),
                            theme::dim(&args_summary)
                        );
                        all_lines.push(truncate_to_width(&line, width, None));
                    }
                }
                ChatEntry::ToolEnd {
                    tool_name,
                    result,
                    is_error,
                    duration_ms,
                    ..
                } => {
                    let is_sub = is_subagent_tool(tool_name);
                    let dur = duration_ms
                        .map(|ms| theme::dim(&format!("  {}ms", ms)))
                        .unwrap_or_default();

                    // Header: icon + name + duration.
                    let (icon, name) = if is_sub {
                        let icon = if *is_error {
                            theme::error("✗")
                        } else {
                            theme::success(SUBAGENT_ICON_DONE)
                        };
                        (icon, theme::magenta(tool_name))
                    } else {
                        let icon = if *is_error {
                            theme::error("✗")
                        } else {
                            theme::success("✓")
                        };
                        (icon, theme::tool_name(tool_name))
                    };
                    all_lines.push(truncate_to_width(
                        &format!("  {} {}{}", icon, name, dur),
                        width,
                        None,
                    ));

                    // Result preview (subagent uses deeper indentation).
                    if !result.is_empty() {
                        let result_color: fn(&str) -> String =
                            if *is_error { theme::error } else { theme::dim };
                        let first_line = result.lines().next().unwrap_or("");
                        let total = result.lines().count();
                        let (indent, max_chars) = if is_sub {
                            (SUBAGENT_INDENT, 50)
                        } else {
                            ("    ", 60)
                        };
                        let preview = if total > 1 {
                            let trunc: String = first_line.chars().take(max_chars).collect();
                            format!("{}{}  (+{} lines)", indent, trunc, total - 1)
                        } else {
                            let max_single = if is_sub { 70 } else { 80 };
                            let trunc: String = first_line.chars().take(max_single).collect();
                            format!("{}{}", indent, trunc)
                        };
                        all_lines.push(truncate_to_width(&result_color(&preview), width, None));
                    }
                }
                ChatEntry::Status { text } => {
                    all_lines.push(String::new()); // spacer
                    for status_line in text.lines() {
                        all_lines.push(truncate_to_width(&theme::dim(status_line), width, None));
                    }
                }
            }
        }

        // Apply scroll offset (scroll_offset lines from bottom are hidden).
        if self.scroll_offset > 0 && all_lines.len() > self.scroll_offset {
            let end = all_lines.len() - self.scroll_offset;
            all_lines.truncate(end);
        }

        all_lines
    }

    fn invalidate(&mut self) {}
}

/// Check if a tool name is a subagent-related tool (spawn or agent_cmd).
fn is_subagent_tool(name: &str) -> bool {
    matches!(name, "spawn" | "agent_cmd")
}

/// Summarize subagent tool arguments for display.
///
/// For `spawn`: shows "agent_label — task_summary"
/// For `agent_cmd`: shows "action → agent_id: message_preview"
fn summarize_subagent_args(tool_name: &str, args: &str) -> String {
    let trimmed = args.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let v: serde_json::Value = match serde_json::from_str(trimmed) {
        Ok(v) => v,
        Err(_) => return summarize_tool_args(args),
    };

    match tool_name {
        "spawn" => {
            let agent = v.get("agent").and_then(|v| v.as_str()).unwrap_or("?");
            let task = v.get("task").and_then(|v| v.as_str()).unwrap_or("");
            let task_preview: String = task.chars().take(50).collect();
            if task.chars().count() > 50 {
                format!("{} — {}...", agent, task_preview)
            } else if task.is_empty() {
                agent.to_string()
            } else {
                format!("{} — {}", agent, task_preview)
            }
        }
        "agent_cmd" => {
            let action = v.get("action").and_then(|v| v.as_str()).unwrap_or("?");
            let agent_id = v.get("agentId").and_then(|v| v.as_str()).unwrap_or("?");
            let message = v.get("message").and_then(|v| v.as_str()).unwrap_or("");
            if message.is_empty() {
                format!("{} → {}", action, agent_id)
            } else {
                let msg_preview: String = message.chars().take(40).collect();
                if message.chars().count() > 40 {
                    format!("{} → {}: {}...", action, agent_id, msg_preview)
                } else {
                    format!("{} → {}: {}", action, agent_id, msg_preview)
                }
            }
        }
        _ => summarize_tool_args(args),
    }
}

/// Subagent icon for running state.
const SUBAGENT_ICON_RUNNING: &str = "◈";
/// Subagent icon for completed state.
const SUBAGENT_ICON_DONE: &str = "◆";
/// Indentation for subagent tool content (deeper than regular 4-space indent).
const SUBAGENT_INDENT: &str = "      ";

/// Extract the most useful field from tool args JSON for display.
fn summarize_tool_args(args: &str) -> String {
    let trimmed = args.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(trimmed) {
        for key in &["command", "path", "query", "url", "content", "oldText"] {
            if let Some(val) = v.get(key).and_then(|v| v.as_str()) {
                let clean: String = val
                    .chars()
                    .filter(|&c| c >= ' ' && c != '\u{007F}')
                    .take(60)
                    .collect();
                if val.chars().count() > 60 {
                    return format!("{}...", clean);
                }
                return clean;
            }
        }
    }
    let clean: String = trimmed
        .chars()
        .filter(|&c| c >= ' ' && c != '\u{007F}')
        .take(60)
        .collect();
    if trimmed.chars().count() > 60 {
        format!("{}...", clean)
    } else {
        clean
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_chat_renders_empty() {
        let mut chat = Chat::new();
        let lines = chat.render(80);
        assert!(lines.is_empty());
    }

    #[test]
    fn user_message_rendered() {
        let mut chat = Chat::new();
        chat.add_entry(ChatEntry::User {
            text: "Hello agent".to_string(),
        });
        let lines = chat.render(80);
        let joined = lines.join("\n");
        // Strip ANSI for content check.
        let plain: String = joined
            .chars()
            .filter(|c| !c.is_control() || *c == '\n')
            .collect();
        assert!(
            plain.contains("Hello agent"),
            "should contain user message: {}",
            plain
        );
    }

    #[test]
    fn streaming_tokens() {
        let mut chat = Chat::new();
        chat.append_token("Hello");
        chat.append_token(" world");
        let lines = chat.render(80);
        let joined = lines.join("\n");
        let plain: String = joined
            .chars()
            .filter(|c| !c.is_control() || *c == '\n')
            .collect();
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
        // The streaming cursor ▌ should not be present.
        assert!(
            !joined.contains('▌'),
            "finalized message should not have cursor"
        );
    }

    #[test]
    fn tool_execution_displayed() {
        let mut chat = Chat::new();
        chat.add_entry(ChatEntry::ToolStart {
            tool_call_id: "c-1".to_string(),
            tool_name: "bash".to_string(),
            args: "ls -la".to_string(),
        });
        chat.complete_tool("c-1", "file1.txt\nfile2.txt", false, Some(42));
        let lines = chat.render(80);
        let joined = lines.join("\n");
        let plain: String = joined
            .chars()
            .filter(|c| !c.is_control() || *c == '\n')
            .collect();
        assert!(plain.contains("bash"), "should contain tool name");
        assert!(plain.contains("file1.txt"), "should contain result");
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
                line,
                visible_width(line)
            );
        }
    }

    // ── Subagent rendering tests (issue #472) ─────────────────────────

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

    #[test]
    fn is_subagent_tool_detects_spawn() {
        assert!(is_subagent_tool("spawn"));
    }

    #[test]
    fn is_subagent_tool_detects_agent_cmd() {
        assert!(is_subagent_tool("agent_cmd"));
    }

    #[test]
    fn is_subagent_tool_rejects_regular_tools() {
        assert!(!is_subagent_tool("bash"));
        assert!(!is_subagent_tool("read"));
        assert!(!is_subagent_tool("edit"));
        assert!(!is_subagent_tool("write"));
        assert!(!is_subagent_tool("grep"));
    }

    #[test]
    fn spawn_tool_shows_agent_label() {
        let mut chat = Chat::new();
        chat.add_entry(ChatEntry::ToolStart {
            tool_call_id: "c-1".to_string(),
            tool_name: "spawn".to_string(),
            args: r#"{"agent":"reviewer","task":"Review PR"}"#.to_string(),
        });
        let lines = chat.render(80);
        let plain = strip_ansi(&lines.join("\n"));
        assert!(
            plain.contains("reviewer"),
            "should contain agent label: {}",
            plain
        );
    }

    #[test]
    fn spawn_tool_shows_subagent_icon() {
        let mut chat = Chat::new();
        chat.add_entry(ChatEntry::ToolStart {
            tool_call_id: "c-1".to_string(),
            tool_name: "spawn".to_string(),
            args: r#"{"agent":"reviewer","task":"Review PR"}"#.to_string(),
        });
        let lines = chat.render(80);
        let plain = strip_ansi(&lines.join("\n"));
        // Subagent tools use a distinct icon (◈ for running, ◆ for done).
        assert!(
            plain.contains('◈') || plain.contains('◆'),
            "should contain subagent icon: {}",
            plain
        );
    }

    #[test]
    fn agent_cmd_steer_shows_action_and_target() {
        let mut chat = Chat::new();
        chat.add_entry(ChatEntry::ToolStart {
            tool_call_id: "c-2".to_string(),
            tool_name: "agent_cmd".to_string(),
            args: r#"{"action":"steer","agentId":"reviewer","message":"focus on tests"}"#
                .to_string(),
        });
        let lines = chat.render(80);
        let plain = strip_ansi(&lines.join("\n"));
        assert!(plain.contains("steer"), "should contain action: {}", plain);
        assert!(
            plain.contains("reviewer"),
            "should contain agent id: {}",
            plain
        );
    }

    #[test]
    fn agent_cmd_follow_up_shows_action_and_target() {
        let mut chat = Chat::new();
        chat.add_entry(ChatEntry::ToolStart {
            tool_call_id: "c-3".to_string(),
            tool_name: "agent_cmd".to_string(),
            args: r#"{"action":"follow_up","agentId":"builder","message":"also fix lint"}"#
                .to_string(),
        });
        let lines = chat.render(80);
        let plain = strip_ansi(&lines.join("\n"));
        assert!(
            plain.contains("follow_up"),
            "should contain action: {}",
            plain
        );
        assert!(
            plain.contains("builder"),
            "should contain agent id: {}",
            plain
        );
    }

    #[test]
    fn agent_cmd_get_state_shows_action_and_target() {
        let mut chat = Chat::new();
        chat.add_entry(ChatEntry::ToolStart {
            tool_call_id: "c-4".to_string(),
            tool_name: "agent_cmd".to_string(),
            args: r#"{"action":"get_state","agentId":"reviewer"}"#.to_string(),
        });
        let lines = chat.render(80);
        let plain = strip_ansi(&lines.join("\n"));
        assert!(
            plain.contains("get_state"),
            "should contain action: {}",
            plain
        );
        assert!(
            plain.contains("reviewer"),
            "should contain agent id: {}",
            plain
        );
    }

    #[test]
    fn agent_cmd_abort_shows_action_and_target() {
        let mut chat = Chat::new();
        chat.add_entry(ChatEntry::ToolStart {
            tool_call_id: "c-5".to_string(),
            tool_name: "agent_cmd".to_string(),
            args: r#"{"action":"abort","agentId":"builder"}"#.to_string(),
        });
        let lines = chat.render(80);
        let plain = strip_ansi(&lines.join("\n"));
        assert!(plain.contains("abort"), "should contain action: {}", plain);
        assert!(
            plain.contains("builder"),
            "should contain agent id: {}",
            plain
        );
    }

    #[test]
    fn spawn_completion_shows_success_and_duration() {
        let mut chat = Chat::new();
        chat.add_entry(ChatEntry::ToolStart {
            tool_call_id: "c-1".to_string(),
            tool_name: "spawn".to_string(),
            args: r#"{"agent":"reviewer","task":"Review PR"}"#.to_string(),
        });
        chat.complete_tool("c-1", "Agent spawned successfully", false, Some(1500));
        let lines = chat.render(80);
        let plain = strip_ansi(&lines.join("\n"));
        assert!(
            plain.contains("1500ms"),
            "should contain duration: {}",
            plain
        );
        // Success icon should be the subagent variant (◆ not ✓).
        assert!(
            plain.contains('◆'),
            "should contain subagent success icon: {}",
            plain
        );
    }

    #[test]
    fn subagent_result_has_deeper_indentation() {
        // Compare indentation of regular tool vs subagent tool result.
        let mut chat_regular = Chat::new();
        chat_regular.add_entry(ChatEntry::ToolStart {
            tool_call_id: "c-1".to_string(),
            tool_name: "bash".to_string(),
            args: r#"{"command":"ls"}"#.to_string(),
        });
        chat_regular.complete_tool("c-1", "file1.txt", false, None);
        let regular_lines = chat_regular.render(80);

        let mut chat_subagent = Chat::new();
        chat_subagent.add_entry(ChatEntry::ToolStart {
            tool_call_id: "c-2".to_string(),
            tool_name: "agent_cmd".to_string(),
            args: r#"{"action":"get_state","agentId":"reviewer"}"#.to_string(),
        });
        chat_subagent.complete_tool("c-2", "Agent is idle", false, None);
        let subagent_lines = chat_subagent.render(80);

        // Find the result preview line in each.
        let regular_result = regular_lines
            .iter()
            .find(|l| strip_ansi(l).contains("file1.txt"))
            .map(|l| strip_ansi(l))
            .unwrap_or_default();
        let subagent_result = subagent_lines
            .iter()
            .find(|l| strip_ansi(l).contains("Agent is idle"))
            .map(|l| strip_ansi(l))
            .unwrap_or_default();

        let regular_indent = regular_result.len() - regular_result.trim_start().len();
        let subagent_indent = subagent_result.len() - subagent_result.trim_start().len();
        assert!(
            subagent_indent > regular_indent,
            "subagent should have deeper indentation: regular={} subagent={}",
            regular_indent,
            subagent_indent
        );
    }

    #[test]
    fn subagent_tool_lines_respect_width() {
        let mut chat = Chat::new();
        chat.add_entry(ChatEntry::ToolStart {
            tool_call_id: "c-1".to_string(),
            tool_name: "spawn".to_string(),
            args: r#"{"agent":"very-long-agent-name-that-might-overflow","task":"A very long task description that exceeds the terminal width"}"#.to_string(),
        });
        chat.complete_tool(
            "c-1",
            "Agent spawned successfully with a very long result message",
            false,
            Some(42),
        );

        chat.add_entry(ChatEntry::ToolStart {
            tool_call_id: "c-2".to_string(),
            tool_name: "agent_cmd".to_string(),
            args:
                r#"{"action":"steer","agentId":"long-agent","message":"A very long steer message"}"#
                    .to_string(),
        });
        chat.complete_tool("c-2", "Steered successfully", false, None);

        let lines = chat.render(40);
        for line in &lines {
            assert!(
                visible_width(line) <= 40,
                "line exceeds width: {} (width={})",
                line,
                visible_width(line)
            );
        }
    }
}
