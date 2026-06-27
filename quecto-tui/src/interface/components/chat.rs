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

#[derive(Debug, Clone)]
struct CachedEntryRender {
    width: usize,
    tool_expanded: bool,
    lines: Vec<String>,
}

/// Chat display component.
#[derive(Debug)]
pub struct Chat {
    entries: Vec<ChatEntry>,
    render_cache: Vec<Option<CachedEntryRender>>,
    /// Cumulative line offsets: `combined_offsets[i]` is the global line index at
    /// which entry `i` starts within the concatenated history, with a trailing
    /// total. Length is `covered_entries + 1`. Rebuilt incrementally so an
    /// unchanged history is never rescanned per frame (#757); the rendered lines
    /// themselves live solely in `render_cache` (no second persistent copy), and
    /// each frame clones only the visible window via `gather_lines`.
    combined_offsets: Vec<usize>,
    /// Width the offset table was built for; a width change invalidates it.
    combined_width: Option<usize>,
    /// Tool-expand state the offset table was built for.
    combined_tool_expanded: bool,
    /// Test-only counters proving the incremental cache avoids redundant work.
    #[cfg(test)]
    pub entry_builds: usize,
    #[cfg(test)]
    pub combined_extends: usize,
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
            render_cache: Vec::new(),
            combined_offsets: Vec::new(),
            combined_width: None,
            combined_tool_expanded: false,
            #[cfg(test)]
            entry_builds: 0,
            #[cfg(test)]
            combined_extends: 0,
            scroll_offset: 0,
            last_render_width: None,
            last_render_line_count: 0,
            viewport_height: None,
            tool_expanded: false,
        }
    }

    pub fn add_entry(&mut self, entry: ChatEntry) {
        self.entries.push(entry);
        self.render_cache.push(None);
        self.scroll_offset = 0;
    }

    /// Append streaming token to the last assistant message, or create one.
    pub fn append_token(&mut self, token: &str) {
        if let Some(ChatEntry::Assistant { text, streaming }) = self.entries.last_mut() {
            if *streaming {
                text.push_str(token);
                if let Some(cache) = self.render_cache.last_mut() {
                    *cache = None;
                }
                return;
            }
        }
        self.entries.push(ChatEntry::Assistant {
            text: token.to_string(),
            streaming: true,
        });
        self.render_cache.push(None);
    }

    /// Finalize the current streaming message.
    pub fn finalize_assistant(&mut self) {
        if let Some(ChatEntry::Assistant { streaming, .. }) = self.entries.last_mut() {
            *streaming = false;
            if let Some(cache) = self.render_cache.last_mut() {
                *cache = None;
            }
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
        self.render_cache.push(None);
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
        for (idx, entry) in self.entries.iter_mut().enumerate().rev() {
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
                    if let Some(cache) = self.render_cache.get_mut(idx) {
                        *cache = None;
                    }
                    break;
                }
            }
        }
    }

    /// Toggle expand/collapse on all tool entries.
    pub fn toggle_tool_expand(&mut self) {
        self.tool_expanded = !self.tool_expanded;
        self.render_cache.fill(None);
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.render_cache.clear();
        self.scroll_offset = 0;
    }

    /// Prepend history entries ABOVE the existing (live) content (#828). The
    /// connect-on-select backfill for a busy sub-agent arrives AFTER live tokens
    /// have already streamed in; reconciling it as a prefix preserves the live
    /// content instead of a wholesale `clear()`+replace that would drop it.
    pub fn prepend_history(&mut self, entries: Vec<ChatEntry>) {
        if entries.is_empty() {
            return;
        }
        let n = entries.len();
        self.entries.splice(0..0, entries);
        self.render_cache
            .splice(0..0, std::iter::repeat_with(|| None).take(n));
        // Indices shifted, so the incremental offset table must rebuild fully.
        self.combined_width = None;
        self.scroll_offset = 0;
    }

    pub fn set_viewport_height(&mut self, height: usize) {
        self.viewport_height = Some(height);
    }

    /// Current scrollback offset (0 == pinned to the latest output). Exposed so
    /// tests can assert that scroll keys actually moved the viewport.
    pub fn scroll_offset(&self) -> usize {
        self.scroll_offset
    }

    pub fn scroll_up(&mut self, amount: usize) {
        self.scroll_offset = self.scroll_offset.saturating_add(amount);
    }

    pub fn scroll_down(&mut self, amount: usize) {
        self.scroll_offset = self.scroll_offset.saturating_sub(amount);
    }

    /// Number of chat entries (tests only — production never reads this).
    #[cfg(test)]
    pub fn entry_count(&self) -> usize {
        self.entries.len()
    }

    /// Text of the last `Status` entry, if the last entry is one (tests only).
    #[cfg(test)]
    pub fn last_status_text(&self) -> Option<&str> {
        match self.entries.last() {
            Some(ChatEntry::Status { text }) => Some(text.as_str()),
            _ => None,
        }
    }
}

impl Chat {
    fn render_entry(entry: &ChatEntry, width: usize, tool_expanded: bool) -> Vec<String> {
        let mut lines = Vec::new();
        match entry {
            ChatEntry::User { text } => {
                lines.push(String::new());
                let label = theme::bold(&theme::accent("> "));
                let wrapped = wrap_text(text, width.saturating_sub(2));
                for (i, line) in wrapped.iter().enumerate() {
                    if i == 0 {
                        lines.push(truncate_to_width(
                            &format!("{}{}", label, line),
                            width,
                            None,
                        ));
                    } else {
                        lines.push(truncate_to_width(&format!("  {}", line), width, None));
                    }
                }
            }
            ChatEntry::Assistant { text, streaming } => {
                if text.is_empty() && *streaming {
                    return lines;
                }
                lines.push(String::new());
                let mut md = Markdown::new(text, 0);
                let md_lines = md.render(width);
                if md_lines.is_empty() {
                    let wrapped = wrap_text(text, width);
                    for line in &wrapped {
                        lines.push(truncate_to_width(line, width, None));
                    }
                } else {
                    lines.extend(md_lines);
                }
                if *streaming {
                    if let Some(l) = lines.last_mut() {
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
                lines.push(String::new());
                lines.extend(render_tool_execution(ToolRenderArgs {
                    tool_name,
                    args_json: parsed_args,
                    result: result.as_deref(),
                    is_error: *is_error,
                    duration_ms: *duration_ms,
                    expanded: tool_expanded,
                    width,
                }));
            }
            ChatEntry::Status { text } => {
                lines.push(String::new());
                for status_line in text.lines() {
                    lines.push(truncate_to_width(&theme::dim(status_line), width, None));
                }
            }
        }
        lines
    }
}

impl Component for Chat {
    fn render(&mut self, width: usize) -> Vec<String> {
        let tool_expanded = self.tool_expanded;
        if self.render_cache.len() != self.entries.len() {
            self.render_cache.resize(self.entries.len(), None);
        }

        // Rebuild only the dirty suffix of the concatenated buffer. A width or
        // tool-expand change invalidates everything; otherwise the first entry
        // whose per-entry cache is stale (e.g. the streaming tail, or a tool
        // entry completed in the middle) marks where re-extension begins.
        let dims_changed =
            self.combined_width != Some(width) || self.combined_tool_expanded != tool_expanded;

        let mut first_dirty = self.entries.len();
        for idx in 0..self.entries.len() {
            let fresh = matches!(
                &self.render_cache[idx],
                Some(c) if c.width == width && c.tool_expanded == tool_expanded
            );
            if !fresh {
                first_dirty = idx;
                break;
            }
        }

        let covered = self.combined_offsets.len().saturating_sub(1);
        let rebuild_from = if dims_changed {
            0
        } else {
            first_dirty.min(covered)
        };

        self.combined_offsets.truncate(rebuild_from + 1);
        if self.combined_offsets.is_empty() {
            self.combined_offsets.push(0);
        }

        for idx in rebuild_from..self.entries.len() {
            let fresh = matches!(
                &self.render_cache[idx],
                Some(c) if c.width == width && c.tool_expanded == tool_expanded
            );
            if !fresh {
                let lines = Self::render_entry(&self.entries[idx], width, tool_expanded);
                self.render_cache[idx] = Some(CachedEntryRender {
                    width,
                    tool_expanded,
                    lines,
                });
                #[cfg(test)]
                {
                    self.entry_builds += 1;
                }
            }
            let cached = self.render_cache[idx]
                .as_ref()
                .expect("entry cache populated above");
            let prev = *self
                .combined_offsets
                .last()
                .expect("offset table always has a base entry");
            self.combined_offsets.push(prev + cached.lines.len());
            #[cfg(test)]
            {
                self.combined_extends += cached.lines.len();
            }
        }

        self.combined_width = Some(width);
        self.combined_tool_expanded = tool_expanded;

        // If the user is scrolled into history, keep the visible viewport anchored
        // while new streaming/tool lines are appended below it. A fixed
        // distance-from-bottom offset would otherwise shrink as content grows,
        // pulling the viewport back toward the live response on every render.
        let full_line_count = self.combined_offsets.last().copied().unwrap_or(0);
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
            let max_scroll = full_line_count.saturating_sub(height);
            let effective = self.scroll_offset.min(max_scroll);
            self.scroll_offset = effective;
            let end = full_line_count.saturating_sub(effective);
            let start = end.saturating_sub(height);
            return self.gather_lines(start, end);
        }

        // Apply scroll offset for callers that render the full chat without a
        // known viewport. Clamp to at least one line to avoid blanking out.
        if self.scroll_offset > 0 && full_line_count > 0 {
            let max_scroll = full_line_count.saturating_sub(1);
            let effective = self.scroll_offset.min(max_scroll);
            self.scroll_offset = effective;
            let end = full_line_count.saturating_sub(effective).max(1);
            return self.gather_lines(0, end);
        }

        self.gather_lines(0, full_line_count)
    }

    fn invalidate(&mut self) {}
}

impl Chat {
    /// Clone the global line window `[start, end)` out of the per-entry caches.
    /// Only the visible window is copied; the full rendered history is never
    /// cloned per frame and is stored exactly once (in `render_cache`) (#757).
    fn gather_lines(&self, start: usize, end: usize) -> Vec<String> {
        if end <= start {
            return Vec::new();
        }
        let mut out = Vec::with_capacity(end - start);
        let entry_count = self.combined_offsets.len().saturating_sub(1);
        for idx in 0..entry_count {
            let entry_start = self.combined_offsets[idx];
            let entry_end = self.combined_offsets[idx + 1];
            if entry_end <= start {
                continue;
            }
            if entry_start >= end {
                break;
            }
            let lines = &self.render_cache[idx]
                .as_ref()
                .expect("covered entry must be cached")
                .lines;
            let lo = start.saturating_sub(entry_start);
            let hi = (end - entry_start).min(lines.len());
            out.extend_from_slice(&lines[lo..hi]);
        }
        out
    }
}

#[path = "chat_render.rs"]
mod chat_render;
use chat_render::*;
#[cfg(test)]
#[path = "chat_render_tests.rs"]
mod chat_render_tests;
#[cfg(test)]
#[path = "chat_integration_tests.rs"]
mod integration_tests;
#[cfg(test)]
#[path = "chat_tests.rs"]
mod tests;
