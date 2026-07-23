//! Chat display component — renders conversation history.

use crate::interface::component::Component;
use crate::interface::components::markdown::Markdown;
use crate::interface::theme;
use crate::interface::utils::{truncate_to_width, visible_width, wrap_text};

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

    /// Immutable view of entries (recovery / tests).
    pub fn entries(&self) -> &[ChatEntry] {
        &self.entries
    }

    pub fn entry_count(&self) -> usize {
        self.entries.len()
    }

    pub fn replace_range(&mut self, start: usize, end: usize, entries: Vec<ChatEntry>) {
        let start = start.min(self.entries.len());
        let end = end.max(start).min(self.entries.len());
        self.entries.splice(start..end, entries);
        self.render_cache = (0..self.entries.len()).map(|_| None).collect();
        self.combined_offsets.clear();
        self.combined_width = None;
        self.scroll_offset = 0;
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

    /// Prepend durable history ABOVE existing live content (#828).
    pub fn prepend_history(&mut self, mut entries: Vec<ChatEntry>) {
        if entries.is_empty() {
            return;
        }
        merge_tool_history_boundary(&mut entries, &mut self.entries);
        let n = entries.len();
        self.entries.splice(0..0, entries);
        self.render_cache
            .splice(0..0, std::iter::repeat_with(|| None).take(n));
        self.combined_width = None;
    }

    /// Replace a durable history prefix while preserving live content (#1050).
    pub fn replace_history_prefix(&mut self, old_len: usize, mut entries: Vec<ChatEntry>) {
        let remove = old_len.min(self.entries.len());
        if remove == 0 {
            merge_tool_history_boundary(&mut entries, &mut self.entries);
        }
        let n = entries.len();
        self.entries.splice(0..remove, entries);
        if self.render_cache.len() >= remove {
            self.render_cache
                .splice(0..remove, std::iter::repeat_with(|| None).take(n));
        } else {
            self.render_cache.clear();
            self.render_cache.resize(self.entries.len(), None);
        }
        self.combined_width = None;
    }

    pub fn set_viewport_height(&mut self, height: usize) {
        self.viewport_height = Some(height);
    }

    /// Current scrollback offset (0 == pinned to the latest output). Exposed so
    /// tests can assert that scroll keys actually moved the viewport.
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
                let line_count = lines.len();
                // A dims change rebuilds every entry; materializing all rendered
                // lines at once would transiently recreate the full-transcript
                // copy #981 removed. With a viewport, keep only the line count
                // here and let `gather_lines` render the visible window on
                // demand. Suffix rebuilds (streaming tail, completed tool) stay
                // single-render.
                let lines = if dims_changed && self.viewport_height.is_some() {
                    None
                } else {
                    Some(CachedLineSlice { start: 0, lines })
                };
                self.render_cache[idx] = Some(CachedEntryRender {
                    width,
                    tool_expanded,
                    line_count,
                    lines,
                });
                #[cfg(any(test, feature = "test-harness"))]
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
            self.combined_offsets.push(prev + cached.line_count);
            #[cfg(any(test, feature = "test-harness"))]
            {
                self.combined_extends += cached.line_count;
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
            let retain_margin = height.saturating_mul(RENDER_CACHE_RETAIN_VIEWPORTS);
            let lines = self.gather_lines(start, end, retain_margin, width, tool_expanded);
            self.evict_rendered_lines_outside(
                start.saturating_sub(retain_margin),
                end.saturating_add(retain_margin),
            );
            return lines;
        }

        // Apply scroll offset for callers that render the full chat without a
        // known viewport. Clamp to at least one line to avoid blanking out.
        if self.scroll_offset > 0 && full_line_count > 0 {
            let max_scroll = full_line_count.saturating_sub(1);
            let effective = self.scroll_offset.min(max_scroll);
            self.scroll_offset = effective;
            let end = full_line_count.saturating_sub(effective).max(1);
            return self.gather_lines(0, end, 0, width, tool_expanded);
        }

        self.gather_lines(0, full_line_count, 0, width, tool_expanded)
    }

    fn invalidate(&mut self) {}
}

impl Chat {
    /// Clone the global line window `[start, end)` out of the per-entry caches.
    /// Only the visible window is copied; rendered lines evicted from distant
    /// history are rebuilt on demand while their line counts remain cached.
    fn gather_lines(
        &mut self,
        start: usize,
        end: usize,
        retain_margin: usize,
        width: usize,
        tool_expanded: bool,
    ) -> Vec<String> {
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
            let lo = start.saturating_sub(entry_start);
            let hi = (end - entry_start).min(entry_end - entry_start);
            self.ensure_rendered_line_slice(idx, lo..hi, retain_margin, width, tool_expanded);
            let slice = self.render_cache[idx]
                .as_ref()
                .and_then(|cached| cached.lines.as_ref())
                .expect("covered entry must have rendered lines");
            let slice_lo = lo.saturating_sub(slice.start);
            let slice_hi = hi.saturating_sub(slice.start).min(slice.lines.len());
            out.extend_from_slice(&slice.lines[slice_lo..slice_hi]);
        }
        out
    }

    /// Guarantee the cached slice for entry `idx` covers the visible span.
    /// Coverage is checked against the visible span only, but a re-render
    /// stores `margin` extra lines on each side, so subsequent scroll steps
    /// consume the margin before another full `render_entry` — one amortized
    /// re-render per `margin` lines of scrolling.
    fn ensure_rendered_line_slice(
        &mut self,
        idx: usize,
        span: std::ops::Range<usize>,
        margin: usize,
        width: usize,
        tool_expanded: bool,
    ) {
        let needs_render = !matches!(
            &self.render_cache[idx],
            Some(c)
                if c.width == width
                    && c.tool_expanded == tool_expanded
                    && c.lines.as_ref().is_some_and(|slice| {
                        span.start >= slice.start && span.end <= slice.start + slice.lines.len()
                    })
        );
        if !needs_render {
            return;
        }

        let mut lines = Self::render_entry(&self.entries[idx], width, tool_expanded);
        let line_count = lines.len();
        let bounded_start = span.start.saturating_sub(margin).min(line_count);
        let bounded_end = span
            .end
            .saturating_add(margin)
            .min(line_count)
            .max(bounded_start);
        let lines = lines.drain(bounded_start..bounded_end).collect();
        self.render_cache[idx] = Some(CachedEntryRender {
            width,
            tool_expanded,
            line_count,
            lines: Some(CachedLineSlice {
                start: bounded_start,
                lines,
            }),
        });
        #[cfg(any(test, feature = "test-harness"))]
        {
            self.entry_builds += 1;
        }
    }

    fn evict_rendered_lines_outside(&mut self, retain_start: usize, retain_end: usize) {
        let entry_count = self.combined_offsets.len().saturating_sub(1);
        for idx in 0..entry_count {
            let entry_start = self.combined_offsets[idx];
            let entry_end = self.combined_offsets[idx + 1];
            let Some(cached) = self.render_cache[idx].as_mut() else {
                continue;
            };
            if entry_end <= retain_start || entry_start >= retain_end {
                cached.lines = None;
                continue;
            }

            let Some(slice) = cached.lines.as_mut() else {
                continue;
            };
            let keep_start = retain_start
                .saturating_sub(entry_start)
                .min(cached.line_count);
            let keep_end = retain_end
                .saturating_sub(entry_start)
                .min(cached.line_count)
                .max(keep_start);
            let slice_end = slice.start + slice.lines.len();
            let overlap_start = slice.start.max(keep_start);
            let overlap_end = slice_end.min(keep_end);
            if overlap_start >= overlap_end {
                cached.lines = None;
                continue;
            }

            let drain_prefix = overlap_start - slice.start;
            let drain_suffix_from = overlap_end - slice.start;
            slice.lines.drain(drain_suffix_from..);
            slice.lines.drain(..drain_prefix);
            slice.start = overlap_start;
        }
    }
}

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
#[path = "chat_tests.rs"]
mod tests;
