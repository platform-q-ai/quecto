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
        // No artificial clamp here — render() handles the actual clamping
        // against the rendered line count. The old heuristic of entries*5
        // was far too low for long responses (#500).
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
        // Clamp to actual rendered line count and write back to prevent
        // unbounded growth (#500).
        if self.scroll_offset > 0 && !all_lines.is_empty() {
            let max_scroll = all_lines.len();
            let effective = self.scroll_offset.min(max_scroll);
            self.scroll_offset = effective; // Write back to prevent unbounded growth.
            let end = all_lines.len().saturating_sub(effective);
            if end > 0 {
                all_lines.truncate(end);
            } else {
                // Scrolled to the very top — show just the first line.
                all_lines.truncate(1);
            }
        }

        all_lines
    }

    fn invalidate(&mut self) {}
}

/// Extract the most useful field from tool args JSON for display.
/// Check if a tool name is a subagent-related tool (spawn or agent_cmd).
fn is_subagent_tool(name: &str) -> bool {
    matches!(name, "spawn" | "agent_cmd")
}

/// Summarize subagent tool arguments for display.
///
/// For `spawn`: shows "agent_label — task_summary"
/// For `agent_cmd`: shows "action → agentId: message_preview"
/// Sanitize a string by stripping control characters (terminal escape injection prevention).
fn sanitize(s: &str) -> String {
    s.chars().filter(|c| !c.is_control()).collect()
}

/// Truncate a string to max_chars, appending "..." if truncated.
fn truncate_with_ellipsis(s: &str, max_chars: usize) -> String {
    let char_count = s.chars().count();
    if char_count > max_chars {
        let truncated: String = s.chars().take(max_chars).collect();
        format!("{}...", truncated)
    } else {
        s.to_string()
    }
}

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
            let agent = sanitize(v.get("agent").and_then(|v| v.as_str()).unwrap_or("?"));
            let task = sanitize(v.get("task").and_then(|v| v.as_str()).unwrap_or(""));
            if task.is_empty() {
                agent
            } else {
                format!("{} — {}", agent, truncate_with_ellipsis(&task, 50))
            }
        }
        "agent_cmd" => {
            let action = sanitize(v.get("action").and_then(|v| v.as_str()).unwrap_or("?"));
            let agent_id = sanitize(v.get("agentId").and_then(|v| v.as_str()).unwrap_or("?"));
            let message = sanitize(v.get("message").and_then(|v| v.as_str()).unwrap_or(""));
            if message.is_empty() {
                format!("{} → {}", action, agent_id)
            } else {
                format!(
                    "{} → {}: {}",
                    action,
                    agent_id,
                    truncate_with_ellipsis(&message, 40)
                )
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

    // ── Scroll clamp tests (issue #500) ───────────────────────────────

    #[test]
    fn scroll_up_reaches_top_of_long_output() {
        let mut chat = Chat::new();
        // Add a very long assistant message (200+ lines when rendered).
        let long_text = (0..200)
            .map(|i| format!("Line {}", i))
            .collect::<Vec<_>>()
            .join("\n");
        chat.add_entry(ChatEntry::Assistant {
            text: long_text,
            streaming: false,
        });

        let full_lines = chat.render(80);
        let total_rendered = full_lines.len();
        assert!(total_rendered > 10, "should render many lines");

        // Scroll up partway (half the content).
        chat.scroll_up(total_rendered / 2);
        let scrolled = chat.render(80);

        // Scrolled view should show fewer lines (bottom half hidden).
        assert!(
            scrolled.len() < total_rendered,
            "scrolled view should be shorter: {} vs {}",
            scrolled.len(),
            total_rendered
        );
        // And should still show content from the top half.
        assert!(
            scrolled.len() > 1,
            "should have more than 1 line visible after half-scroll"
        );
    }

    #[test]
    fn scroll_offset_not_artificially_clamped() {
        let mut chat = Chat::new();
        // Add content that renders to many lines (1 entry, 100 lines).
        let long_text = (0..100)
            .map(|i| format!("Line {}", i))
            .collect::<Vec<_>>()
            .join("\n");
        chat.add_entry(ChatEntry::Assistant {
            text: long_text,
            streaming: false,
        });

        // With the old bug: entries.len()=1, clamp = 1*5 = 5.
        // Scrolling up 50 should NOT be clamped to 5.
        chat.scroll_up(50);

        // The scroll_offset should actually be 50, not clamped to 5.
        assert!(
            chat.scroll_offset >= 50,
            "scroll_offset should be at least 50, got {}",
            chat.scroll_offset
        );
    }

    #[test]
    fn scroll_down_from_scrolled_position() {
        let mut chat = Chat::new();
        // Use many separate user messages to guarantee many rendered lines.
        for i in 0..30 {
            chat.add_entry(ChatEntry::User {
                text: format!("Message number {}", i),
            });
        }

        let full = chat.render(80);
        assert!(
            full.len() > 30,
            "should have many rendered lines: {}",
            full.len()
        );

        // Scroll up partway.
        chat.scroll_up(15);
        let scrolled_up = chat.render(80);
        assert!(
            scrolled_up.len() < full.len(),
            "scrolled up should be shorter: {} vs {}",
            scrolled_up.len(),
            full.len()
        );

        // Scroll back down 10.
        chat.scroll_down(10);
        let after_down = chat.render(80);

        // After scrolling down, we should see more lines.
        assert!(
            after_down.len() > scrolled_up.len(),
            "scrolling down should show more lines: up={} down={}",
            scrolled_up.len(),
            after_down.len()
        );
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
        assert!(
            plain.contains('◆'),
            "should contain subagent success icon: {}",
            plain
        );
    }

    #[test]
    fn subagent_result_has_deeper_indentation() {
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
            args: r#"{"agent":"very-long-agent-name","task":"A very long task description"}"#
                .to_string(),
        });
        chat.complete_tool(
            "c-1",
            "Agent spawned successfully with a long result",
            false,
            Some(42),
        );
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
