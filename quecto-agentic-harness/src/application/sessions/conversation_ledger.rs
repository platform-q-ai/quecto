//! The live-conversation read model of the active session (#1856, #1858):
//! the published (possibly pruned or collapsed) transcript, a bounded
//! id-addressable ledger of full message copies, the epoch/revision
//! frontier clients synchronise against, and the retention store consulted
//! when a collapsed message's full copy is no longer retained.
//!
//! Every reader — the idle dispatch loop, the busy reader task, the
//! connect-time snapshot — sees one epoch-consistent view behind one lock;
//! the loop publishes into it at every turn boundary. Nothing here knows a
//! wire shape: readers receive domain messages and typed advances.
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use crate::application::sessions::ports::ContextSpillStore;
use crate::domain::conversation_view::position_by_id;
use crate::domain::ids::MessageId;
use crate::domain::message::{Message, ThinkingBlock};

/// Bounded ledger budgets. Eviction is oldest-first and triggers on either
/// content bytes or entry count so long-running sessions and floods of tiny
/// messages cannot grow memory unbounded (#1060 review r4).
const LEDGER_MAX_BYTES: usize = 16 * 1024 * 1024;
pub const LEDGER_MAX_ENTRIES: usize = 8192;
/// Fixed per-entry overhead so a zero/tiny-content message still consumes
/// budget: it covers the id `String` stored twice (map key + order deque,
/// ~2×UUID) plus the owned Message/ToolCall struct footprint.
const LEDGER_ENTRY_OVERHEAD: usize = 256;

/// The ledger position after a publication: which epoch and revision the
/// transcript is at, and whether this publication changed it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LedgerAdvance {
    pub epoch: u64,
    pub rev: u64,
    pub changed: bool,
}

impl LedgerAdvance {
    fn unchanged(epoch: u64, rev: u64) -> Self {
        Self {
            epoch,
            rev,
            changed: false,
        }
    }
}

/// Approximate owned in-memory size of a ledger entry for byte-budgeting.
fn message_bytes(m: &Message) -> usize {
    LEDGER_ENTRY_OVERHEAD
        + m.content.len()
        + m.tool_calls
            .iter()
            .map(|tc| tc.arguments.len() + tc.name.len() + tc.id.len())
            .sum::<usize>()
        + m.tool_call_id.as_ref().map_or(0, |s| s.len())
        + m.tool_name.as_ref().map_or(0, |s| s.len())
        + m.thinking_blocks
            .iter()
            .map(thinking_block_bytes)
            .sum::<usize>()
}

fn thinking_block_bytes(tb: &ThinkingBlock) -> usize {
    match tb {
        ThinkingBlock::Normal {
            thinking,
            signature,
        } => thinking.len() + signature.len(),
        ThinkingBlock::Redacted { data } => data.len(),
    }
}

/// Live transcript plus the bounded full-copy ledger and its sync frontier.
#[derive(Default)]
pub struct ConversationLedger {
    /// Live conversation as last published (may be pruned/collapsed).
    messages: Vec<Message>,
    /// id → full message, bounded by byte + entry caps, authoritative for
    /// message recovery.
    ledger: HashMap<String, Message>,
    /// Insertion order (front = oldest) driving eviction.
    ledger_order: VecDeque<String>,
    /// Running byte total of `ledger` (content + per-entry overhead).
    ledger_bytes: usize,
    /// Retention store used as a best-effort backstop when a live message
    /// only has a collapsed recall stub and its full ledger copy has already
    /// been evicted. Held here so the busy reader can resolve refs while the
    /// dispatch loop is blocked (#1093).
    spill_store: Option<Arc<dyn ContextSpillStore>>,
    /// Monotonic identity/version of the published history. Deferred spill
    /// reads capture this value and may only complete while it is
    /// unchanged, so an old session/rewind lookup cannot cross a lifecycle
    /// replacement while the lock is released for I/O.
    generation: u64,
    epoch: u64,
    rev: u64,
    frontier: VecDeque<(u64, String)>,
}

impl ConversationLedger {
    /// The transcript as last published, in conversation order.
    pub fn live_messages(&self) -> &[Message] {
        &self.messages
    }

    pub fn epoch(&self) -> u64 {
        self.epoch
    }

    pub fn rev(&self) -> u64 {
        self.rev
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// The committed revisions still in the sync window, oldest first, each
    /// with the id of the message it committed.
    pub fn frontier(&self) -> impl Iterator<Item = (u64, &str)> {
        self.frontier.iter().map(|(rev, id)| (*rev, id.as_str()))
    }

    pub fn spill_store(&self) -> Option<&Arc<dyn ContextSpillStore>> {
        self.spill_store.as_ref()
    }

    /// Fold one message into the ledger under the byte budget. `overwrite`
    /// replaces an existing (possibly collapsed) entry with a full copy;
    /// without it, an earlier full copy is kept (publish must not clobber
    /// it). Returns whether the id was new to the ledger.
    fn remember(&mut self, m: &Message, overwrite: bool) -> bool {
        let id = m.id().to_string();
        let already = self.ledger.contains_key(&id);
        if already && !overwrite {
            return false;
        }
        let new_sz = message_bytes(m);
        if already {
            let old_sz = self.ledger.get(&id).map(message_bytes).unwrap_or(0);
            self.ledger_bytes = self.ledger_bytes.saturating_sub(old_sz);
            self.ledger.insert(id, m.clone()); // keeps its original age in `ledger_order`
        } else {
            self.ledger.insert(id.clone(), m.clone());
            self.ledger_order.push_back(id);
        }
        self.ledger_bytes += new_sz;
        // Evict oldest-first while EITHER cap is exceeded — the byte budget
        // bounds large-payload content, the entry cap bounds a flood of
        // tiny/empty messages whose per-entry cost the byte total under-counts.
        while self.ledger_bytes > LEDGER_MAX_BYTES || self.ledger.len() > LEDGER_MAX_ENTRIES {
            let Some(old_id) = self.ledger_order.pop_front() else {
                break;
            };
            if let Some(old) = self.ledger.remove(&old_id) {
                self.ledger_bytes = self.ledger_bytes.saturating_sub(message_bytes(&old));
                if let Some(pos) = self.frontier.iter().position(|(_, fid)| fid == &old_id) {
                    self.frontier.remove(pos);
                }
            }
        }
        !already
    }

    fn advance_for_new_ids(&mut self, ids: Vec<String>) -> LedgerAdvance {
        if ids.is_empty() {
            return LedgerAdvance::unchanged(self.epoch, self.rev);
        }
        for id in ids {
            self.rev = self.rev.wrapping_add(1);
            self.frontier.push_back((self.rev, id));
        }
        LedgerAdvance {
            epoch: self.epoch,
            rev: self.rev,
            changed: true,
        }
    }

    /// Replace the live view and fold its messages into the ledger WITHOUT
    /// overwriting existing entries — a full copy recorded earlier must
    /// survive a later in-place collapse of the live message.
    pub fn publish(&mut self, messages: &[Message]) -> LedgerAdvance {
        let mut new_ids = Vec::new();
        for m in messages {
            if self.remember(m, false) {
                new_ids.push(m.id().to_string());
            }
        }
        self.messages = messages.to_vec();
        self.advance_for_new_ids(new_ids)
    }

    /// Record full copies of `messages` (a turn's appended messages), so a
    /// ref resolves to full content even after the ladder prunes or
    /// collapses the live entry.
    pub fn record_full(&mut self, messages: &[Message]) -> LedgerAdvance {
        let mut new_ids = Vec::new();
        for m in messages {
            if self.remember(m, true) {
                new_ids.push(m.id().to_string());
            }
        }
        self.advance_for_new_ids(new_ids)
    }

    /// Attach the retention store for best-effort recall of live collapsed
    /// messages whose full ledger copy is no longer available. A changed
    /// store handle invalidates any deferred read prepared against the
    /// previous one (`Arc::ptr_eq`, so ordinary per-turn refreshes that
    /// re-publish the same handle do not advance the generation).
    pub fn set_spill_store(&mut self, spill_store: Option<Arc<dyn ContextSpillStore>>) {
        let store_changed = match (&self.spill_store, &spill_store) {
            (Some(current), Some(next)) => !Arc::ptr_eq(current, next),
            (None, None) => false,
            _ => true,
        };
        if store_changed {
            self.generation = self.generation.wrapping_add(1);
        }
        self.spill_store = spill_store;
    }

    /// Replace the retention store without touching the generation: only
    /// valid right after [`Self::clear`], which already advanced it.
    pub(super) fn replace_spill_store_after_clear(
        &mut self,
        spill_store: Option<Arc<dyn ContextSpillStore>>,
    ) {
        self.spill_store = spill_store;
    }

    /// Look a message id up by its full copy: the ledger wins over the live
    /// conversation (which may hold only a collapsed stub). `None` when the
    /// ref is neither in the (bounded) ledger nor the live conversation.
    /// Shared by every resolver so the ledger-then-live precedence stays in
    /// one place.
    pub fn lookup(&self, message_id: &str) -> Option<&Message> {
        self.ledger.get(message_id).or_else(|| {
            position_by_id(&self.messages, &MessageId::from(message_id)).map(|i| &self.messages[i])
        })
    }

    /// The full ledger copy of a message, if retained (the live conversation
    /// may hold only a collapsed stub).
    pub fn full_copy(&self, message_id: &str) -> Option<&Message> {
        self.ledger.get(message_id)
    }

    /// Every retained message: the ledger's full copies in insertion order,
    /// then any live message the ledger no longer holds.
    pub fn retained_messages(&self) -> Vec<Message> {
        let mut messages: Vec<_> = self
            .ledger_order
            .iter()
            .filter_map(|id| self.ledger.get(id).cloned())
            .collect();
        for message in &self.messages {
            if messages
                .iter()
                .all(|existing| existing.id() != message.id())
            {
                messages.push(message.clone());
            }
        }
        messages
    }

    /// Drop the live view, the ledger and the frontier, and open a new
    /// epoch; refs from the cleared transcript stop resolving.
    pub fn clear(&mut self) -> LedgerAdvance {
        self.messages.clear();
        self.ledger.clear();
        self.ledger_order.clear();
        self.frontier.clear();
        self.ledger_bytes = 0;
        self.generation = self.generation.wrapping_add(1);
        self.epoch = self.epoch.wrapping_add(1);
        LedgerAdvance {
            epoch: self.epoch,
            rev: self.rev,
            changed: true,
        }
    }
}

#[cfg(test)]
#[path = "conversation_ledger_tests.rs"]
mod tests;
