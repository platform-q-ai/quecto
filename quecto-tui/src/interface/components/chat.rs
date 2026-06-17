//! Chat display component — renders conversation history.
//!
//! Displays user messages, assistant responses (with streaming), and
//! tool execution results in a scrollable vertical layout.
//!
//! Tool rendering uses Quecto-style background-colored boxes with tool-specific
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
                    let tool_lines = render_tool_execution(ToolRenderArgs {
                        tool_name,
                        args_json: parsed_args,
                        result: result.as_deref(),
                        is_error: *is_error,
                        duration_ms: *duration_ms,
                        expanded: tool_expanded,
                        width,
                    });
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

#[path = "chat_render.rs"]
mod chat_render;
use chat_render::*;
#[cfg(test)]
#[path = "chat_integration_tests.rs"]
mod integration_tests;
#[cfg(test)]
#[path = "chat_tests.rs"]
mod tests;
