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
