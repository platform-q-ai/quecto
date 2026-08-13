//! Chat display component — renders conversation history.

use crate::components::component::Component;
use crate::components::markdown::Markdown;
use crate::components::theme;
use crate::components::utils::{truncate_to_width, visible_width, wrap_text};

/// Number of output lines shown for bash in collapsed mode (tail).
const BASH_PREVIEW_LINES: usize = 5;
/// Number of content lines shown for read/write in collapsed mode (head).
const FILE_PREVIEW_LINES: usize = 10;
/// Extra viewport-sized context retained around the visible chat window.
const RENDER_CACHE_RETAIN_VIEWPORTS: usize = 2;

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
    /// Ladder-demoted history message stub (#1061).
    Stub {
        id: String,
        is_user: bool,
        text: String,
        content_len: Option<usize>,
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

#[derive(Debug, Clone, PartialEq, Eq)]
struct CachedLineSlice {
    /// Entry-local line index corresponding to `lines[0]`.
    start: usize,
    lines: Vec<String>,
}

#[derive(Debug, Clone)]
struct CachedEntryRender {
    width: usize,
    tool_expanded: bool,
    line_count: usize,
    lines: Option<CachedLineSlice>,
}

/// Max transcript entries retained per chat session (#1196).
pub const CHAT_RETAINED_ENTRY_CAP: usize = 1024;

/// Chat display component.
#[derive(Debug)]
pub struct Chat {
    entries: Vec<ChatEntry>,
    render_cache: Vec<Option<CachedEntryRender>>,
    /// Cumulative line offsets: `combined_offsets[i]` is the global line index at
    /// which entry `i` starts within the concatenated history, with a trailing
    /// total. Length is `covered_entries + 1`. Rebuilt incrementally so an
    /// unchanged history is never rescanned per frame (#757). Per-entry line
    /// counts remain cached here, while rendered line vectors outside the
    /// viewport window are evicted and rebuilt on demand (#981).
    combined_offsets: Vec<usize>,
    /// Width the offset table was built for; a width change invalidates it.
    combined_width: Option<usize>,
    /// Tool-expand state the offset table was built for.
    combined_tool_expanded: bool,
    /// Test/harness-only counters proving the incremental cache avoids
    /// redundant work.
    #[cfg(any(test, feature = "test-harness"))]
    pub entry_builds: usize,
    #[cfg(any(test, feature = "test-harness"))]
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
    /// Entries removed ahead of the current window by retention since the last
    /// owner reconciliation. Controllers use this to keep turn indices stable.
    retention_front_trimmed: usize,
    retention_front_inserted: usize,
}

impl Default for Chat {
    fn default() -> Self {
        Self::new()
    }
}

fn merge_tool_history_boundary(prefix: &mut [ChatEntry], existing: &mut Vec<ChatEntry>) {
    let matching = matches!((prefix.last(), existing.first()), (Some(ChatEntry::ToolExecution { tool_call_id, result: None, .. }), Some(ChatEntry::ToolExecution { tool_call_id: existing_id, result: Some(_), .. })) if tool_call_id == existing_id);
    if !matching {
        return;
    }
    let Some(ChatEntry::ToolExecution {
        result: Some(result),
        is_error,
        ..
    }) = existing.first_mut()
    else {
        return;
    };
    let (result, is_error) = (std::mem::take(result), *is_error);
    if let Some(ChatEntry::ToolExecution {
        result: target,
        is_error: target_error,
        ..
    }) = prefix.last_mut()
    {
        *target = Some(result);
        *target_error = is_error;
    }
    existing.remove(0);
}

impl Chat {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
            render_cache: Vec::new(),
            combined_offsets: Vec::new(),
            combined_width: None,
            combined_tool_expanded: false,
            #[cfg(any(test, feature = "test-harness"))]
            entry_builds: 0,
            #[cfg(any(test, feature = "test-harness"))]
            combined_extends: 0,
            scroll_offset: 0,
            last_render_width: None,
            last_render_line_count: 0,
            viewport_height: None,
            tool_expanded: false,
            retention_front_trimmed: 0,
            retention_front_inserted: 0,
        }
    }

    pub fn add_entry(&mut self, entry: ChatEntry) {
        self.entries.push(entry);
        self.render_cache.push(None);
        self.enforce_retention_tail();
    }

    pub fn add_entry_follow_tail(&mut self, entry: ChatEntry) {
        self.add_entry(entry);
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
        self.enforce_retention_tail();
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

    /// Immutable view of entries (recovery / tests).
    pub fn entries(&self) -> &[ChatEntry] {
        &self.entries
    }

    pub fn entry_count(&self) -> usize {
        self.entries.len()
    }

    pub fn take_retention_front_delta(&mut self) -> (usize, usize) {
        (
            std::mem::take(&mut self.retention_front_trimmed),
            std::mem::take(&mut self.retention_front_inserted),
        )
    }

    pub fn replace_range(&mut self, start: usize, end: usize, entries: Vec<ChatEntry>) {
        let start = start.min(self.entries.len());
        let end = end.max(start).min(self.entries.len());
        self.entries.splice(start..end, entries);
        self.render_cache = (0..self.entries.len()).map(|_| None).collect();
        self.combined_offsets.clear();
        self.combined_width = None;
        self.enforce_retention_tail();
    }

    /// Start a tool execution — creates a ToolExecution entry.
    pub fn start_tool(&mut self, tool_call_id: String, tool_name: String, args: String) {
        self.finalize_assistant();
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
        self.enforce_retention_tail();
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
        self.retention_front_trimmed = 0;
        self.retention_front_inserted = 0;
    }

    /// Prepend durable history ABOVE existing live content (#828).
    pub fn prepend_history(&mut self, mut entries: Vec<ChatEntry>) {
        if entries.is_empty() {
            return;
        }
        merge_tool_history_boundary(&mut entries, &mut self.entries);
        let n = entries.len();
        let suffix_len = self.entries.len();
        self.entries.splice(0..0, entries);
        self.render_cache
            .splice(0..0, std::iter::repeat_with(|| None).take(n));
        self.combined_width = None;
        self.enforce_retention_after_prefix_mutation(n, suffix_len);
    }

    /// Replace a durable history prefix while preserving live content (#1050).
    pub fn replace_history_prefix(&mut self, old_len: usize, mut entries: Vec<ChatEntry>) {
        let remove = old_len.min(self.entries.len());
        if remove == 0 {
            merge_tool_history_boundary(&mut entries, &mut self.entries);
        }
        let n = entries.len();
        let suffix_len = self.entries.len().saturating_sub(remove);
        self.entries.splice(0..remove, entries);
        if self.render_cache.len() >= remove {
            self.render_cache
                .splice(0..remove, std::iter::repeat_with(|| None).take(n));
        } else {
            self.render_cache.clear();
            self.render_cache.resize(self.entries.len(), None);
        }
        self.combined_width = None;
        self.enforce_retention_after_prefix_mutation(n, suffix_len);
    }

    fn enforce_retention_tail(&mut self) {
        super::chat_retention::trim_tail(self);
    }

    fn enforce_retention_after_prefix_mutation(&mut self, prefix_len: usize, suffix_len: usize) {
        super::chat_retention::trim_after_prefix_mutation(self, prefix_len, suffix_len);
    }

    pub(super) fn trim_front_for_retention(&mut self, count: usize) {
        if count == 0 {
            return;
        }
        self.retention_front_trimmed = self.retention_front_trimmed.saturating_add(count);
        self.entries.drain(0..count);
        if self.render_cache.len() >= count {
            self.render_cache.drain(0..count);
        } else {
            self.render_cache.clear();
            self.render_cache.resize(self.entries.len(), None);
        }
        self.invalidate_after_retention_trim();
    }

    pub(super) fn replace_entries_after_retention(&mut self, entries: Vec<ChatEntry>) {
        self.entries = entries;
        self.render_cache.clear();
        self.render_cache.resize(self.entries.len(), None);
        self.invalidate_after_retention_trim();
    }

    pub(super) fn record_retention_front_trimmed(&mut self, count: usize) {
        self.retention_front_trimmed = self.retention_front_trimmed.saturating_add(count);
    }

    pub(super) fn record_retention_front_inserted(&mut self, count: usize) {
        self.retention_front_inserted = self.retention_front_inserted.saturating_add(count);
    }

    pub(super) fn invalidate_after_retention_trim(&mut self) {
        self.combined_offsets.clear();
        self.combined_width = None;
        self.scroll_offset = 0;
        self.last_render_line_count = 0;
    }

    pub fn set_viewport_height(&mut self, height: usize) {
        self.viewport_height = Some(height);
    }

    pub fn scroll_offset(&self) -> usize {
        self.scroll_offset
    }

    /// Whether the viewport has reached the oldest currently loaded line.
    /// Paging may fetch another prefix only at this boundary.
    pub fn is_at_oldest_loaded_history(&self) -> bool {
        let Some(height) = self.viewport_height else {
            return false;
        };
        self.scroll_offset >= self.last_render_line_count.saturating_sub(height)
    }

    pub fn scroll_up(&mut self, amount: usize) {
        self.scroll_offset = self.scroll_offset.saturating_add(amount);
    }

    pub fn scroll_down(&mut self, amount: usize) {
        self.scroll_offset = self.scroll_offset.saturating_sub(amount);
    }

    /// Number of retained rendered lines (tests/harness only — production never reads this).
    #[cfg(any(test, feature = "test-harness"))]
    pub fn cached_rendered_line_count(&self) -> usize {
        // Exhaustive destructuring (no `..`): adding a field to `Chat` fails to
        // compile here, forcing it to be classified as raw transcript state
        // (exempt from the bound) or rendered-line storage (counted), so a
        // second rendered-transcript copy cannot sneak past the bounded-cache
        // tests (#981).
        let Self {
            entries: _,          // raw transcript — intentionally unbounded
            render_cache,        // rendered lines — counted below
            combined_offsets: _, // per-entry line counts, O(entries) usize
            combined_width: _,
            combined_tool_expanded: _,
            entry_builds: _,
            combined_extends: _,
            scroll_offset: _,
            last_render_width: _,
            last_render_line_count: _,
            viewport_height: _,
            tool_expanded: _,
            retention_front_trimmed: _,
            retention_front_inserted: _,
        } = self;
        render_cache
            .iter()
            .flatten()
            .filter_map(|cached| cached.lines.as_ref())
            .map(|slice| slice.lines.len())
            .sum()
    }

    /// Maximum rendered lines the cache may retain after a viewport render
    /// (tests/harness only): the visible window plus the retention margin on
    /// each side. Panics without a viewport height, where no bound applies.
    #[cfg(any(test, feature = "test-harness"))]
    pub fn rendered_line_retention_bound(&self) -> usize {
        let height = self
            .viewport_height
            .expect("retention bound requires a viewport height");
        height * (2 * RENDER_CACHE_RETAIN_VIEWPORTS + 1)
    }

    /// Text of the last `Status` entry, if the last entry is one (tests/harness).
    #[cfg(any(test, feature = "test-harness"))]
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
                        let indicator = theme::dim("▌");
                        let indicator_width = visible_width(&indicator);
                        let text_width = width.saturating_sub(indicator_width);
                        *l = truncate_to_width(l, text_width, None);
                        l.push_str(&indicator);
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
            ChatEntry::Stub { text, is_user, .. } => {
                // A demoted stub renders exactly like its underlying role; scroll
                // auto-recall swaps it for the full body in place (#1061).
                let proxy = if *is_user {
                    ChatEntry::User { text: text.clone() }
                } else {
                    ChatEntry::Assistant {
                        text: text.clone(),
                        streaming: false,
                    }
                };
                return Self::render_entry(&proxy, width, tool_expanded);
            }
        }
        lines
    }
}

include!("chat_view.rs");

#[path = "chat_render.rs"]
mod chat_render;
use chat_render::*;
#[cfg(test)]
#[path = "chat_cache_tests.rs"]
mod cache_tests;
#[cfg(test)]
#[path = "chat_bg_tests.rs"]
mod chat_bg_tests;
#[cfg(test)]
#[path = "chat_file_preview_tests.rs"]
mod chat_file_preview_tests;
#[cfg(test)]
#[path = "chat_render_tests.rs"]
mod chat_render_tests;
#[path = "chat_stub.rs"]
mod chat_stub;
#[cfg(test)]
#[path = "chat_stub_tests.rs"]
mod chat_stub_tests;
#[cfg(test)]
#[path = "chat_integration_tests.rs"]
mod integration_tests;
#[cfg(test)]
#[path = "chat_retention_tests.rs"]
mod retention_tests;
#[cfg(test)]
#[path = "chat_tests.rs"]
mod tests;
