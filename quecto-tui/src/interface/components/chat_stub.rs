//! Demoted-history stub recall for [`Chat`] (#1061). Split from `chat.rs` to
//! keep that file within the source line-count gate. A history message the
//! context ladder collapsed arrives as a [`ChatEntry::Stub`] and is swapped for
//! its full body in place once auto-recall fetches it.

use super::{Chat, ChatEntry};

impl Chat {
    /// Stable ids of history entries still rendered as recallable stubs. Drives
    /// auto-recall-on-scroll: each id is fetched once and swapped for its full
    /// body when it arrives.
    pub fn stub_message_ids(&self) -> Vec<String> {
        self.entries
            .iter()
            .filter_map(|entry| match entry {
                ChatEntry::Stub { id, .. } => Some(id.clone()),
                _ => None,
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
