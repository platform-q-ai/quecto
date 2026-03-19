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
                    let args_summary = summarize_tool_args(args);
                    // Top border for running tool box.
                    let border_w = width.saturating_sub(2);
                    all_lines.push(truncate_to_width(
                        &format!("  {}", theme::dim(&"─".repeat(border_w))),
                        width,
                        None,
                    ));
                    // Pi-style: ⠋ tool_name args_summary
                    let line = format!(
                        "  {} {} {}",
                        theme::spinner("⠋"),
                        theme::tool_name(tool_name),
                        theme::dim(&args_summary)
                    );
                    all_lines.push(truncate_to_width(&line, width, None));
                }
                ChatEntry::ToolEnd {
                    tool_name,
                    result,
                    is_error,
                    duration_ms,
                    ..
                } => {
                    let border_w = width.saturating_sub(2);

                    // Top border for completed tool box.
                    all_lines.push(truncate_to_width(
                        &format!("  {}", theme::dim(&"─".repeat(border_w))),
                        width,
                        None,
                    ));

                    // Pi-style: ✓/✗ tool_name  duration
                    let icon = if *is_error {
                        theme::error("✗")
                    } else {
                        theme::success("✓")
                    };
                    let dur = duration_ms
                        .map(|ms| theme::dim(&format!("  {}ms", ms)))
                        .unwrap_or_default();
                    let name = theme::tool_name(tool_name);

                    all_lines.push(truncate_to_width(
                        &format!("  {} {}{}", icon, name, dur),
                        width,
                        None,
                    ));

                    // Show truncated result preview (collapsed view).
                    if !result.is_empty() {
                        let result_color: fn(&str) -> String =
                            if *is_error { theme::error } else { theme::dim };
                        let first_line = result.lines().next().unwrap_or("");
                        let total = result.lines().count();
                        let preview = if total > 1 {
                            let trunc: String = first_line.chars().take(60).collect();
                            format!("    {}  (+{} lines)", trunc, total - 1)
                        } else {
                            let trunc: String = first_line.chars().take(80).collect();
                            format!("    {}", trunc)
                        };
                        all_lines.push(truncate_to_width(&result_color(&preview), width, None));
                    }

                    // Bottom border for completed tool box.
                    all_lines.push(truncate_to_width(
                        &format!("  {}", theme::dim(&"─".repeat(border_w))),
                        width,
                        None,
                    ));
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
}
