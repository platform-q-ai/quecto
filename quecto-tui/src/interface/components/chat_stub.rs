//! Demoted-history stub recall for [`Chat`] (#1061). Split from `chat.rs` to
//! keep that file within the source line-count gate. A history message the
//! context ladder collapsed arrives as a [`ChatEntry::Stub`] and is swapped for
//! its full body in place once auto-recall fetches it.

use super::{Chat, ChatEntry};

impl Chat {
    /// Stable ids of recallable stubs intersecting the current viewport. This
    /// keeps scroll-triggered recall lazy instead of fetching every loaded stub.
    pub fn visible_stub_message_ids(&self) -> Vec<String> {
        let Some(height) = self.viewport_height else {
            return Vec::new();
        };
        let full_lines = self.combined_offsets.last().copied().unwrap_or(0);
        let max_scroll = full_lines.saturating_sub(height);
        let effective_scroll = self.scroll_offset.min(max_scroll);
        let end = full_lines.saturating_sub(effective_scroll);
        let start = end.saturating_sub(height);
        self.entries
            .iter()
            .enumerate()
            .filter_map(|(idx, entry)| {
                let entry_start = *self.combined_offsets.get(idx)?;
                let entry_end = *self.combined_offsets.get(idx + 1)?;
                if entry_end <= start || entry_start >= end {
                    return None;
                }
                match entry {
                    ChatEntry::Stub { id, .. } => Some(id.clone()),
                    _ => None,
                }
            })
            .collect()
    }

    /// Swap a recalled stub for its full content in place, converting it to a
    /// plain `User`/`Assistant` entry so it is no longer a recall target. Returns
    /// whether a matching stub was found. Preserves position and role.
    pub fn recall_stub(&mut self, message_id: &str, full_text: &str) -> bool {
        for (idx, entry) in self.entries.iter_mut().enumerate() {
            if let ChatEntry::Stub { id, is_user, .. } = entry {
                if id == message_id {
                    *entry = if *is_user {
                        ChatEntry::User {
                            text: full_text.to_string(),
                        }
                    } else {
                        ChatEntry::Assistant {
                            text: full_text.to_string(),
                            streaming: false,
                        }
                    };
                    if let Some(cache) = self.render_cache.get_mut(idx) {
                        *cache = None;
                    }
                    // An expanded stub changes this entry's line count, so the
                    // incremental offset table must rebuild from here.
                    self.combined_width = None;
                    return true;
                }
            }
        }
        false
    }
}
