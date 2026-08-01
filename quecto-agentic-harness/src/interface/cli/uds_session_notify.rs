//! Subagent notification enqueue/dedupe logic for [`AgentSession`] (#816,
//! #1082). Child module of `uds_session` so it can reach the session's
//! private queues.

use super::{AgentSession, PendingMessage};

/// Outcome of [`AgentSession::enqueue_subagent_notification`] (#1082 review):
/// callers must distinguish "this is a new note to announce" from "stale
/// duplicate" from "dropped for capacity" — a single bool conflated the
/// latter two.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotificationEnqueueOutcome {
    /// Newly queued (or replacing an earlier still-pending note for the same
    /// agent). The dedupe sequence has been recorded.
    Retained,
    /// Stale or repeated sequence; already delivered or superseded. Dropped.
    Duplicate,
    /// Both the pending queue and the overflow buffer are full. NOT retained
    /// and the dedupe sequence was NOT advanced — the identical sequence may
    /// be retried later.
    Dropped,
}

impl NotificationEnqueueOutcome {
    /// Whether the note was accepted for eventual delivery.
    pub fn is_retained(self) -> bool {
        matches!(self, Self::Retained)
    }
}

impl AgentSession {
    pub fn record_subagent_notification(&mut self, agent_id: String, sequence: u64) -> bool {
        if self
            .last_subagent_notification
            .get(&agent_id)
            .is_some_and(|last| sequence <= *last)
        {
            return false;
        }
        if self.last_subagent_notification.len() >= Self::MAX_DEDUPE_AGENTS
            && !self.last_subagent_notification.contains_key(&agent_id)
            && let Some(oldest) = self.last_subagent_notification.keys().next().cloned()
        {
            self.last_subagent_notification.remove(&oldest);
        }
        self.last_subagent_notification.insert(agent_id, sequence);
        true
    }

    /// Enqueue a subagent completion note for delivery at the parent's NEXT
    /// idle boundary (#816). The note is buffered as a
    /// [`PendingMessage::SubagentNotification`] so it is drained as a single
    /// `role:"user"` turn by `drain_pending_and_nudge` — never injected into
    /// an in-flight parent turn.
    ///
    /// Deduped per-agent via [`Self::record_subagent_notification`] (a stale or
    /// repeated `sequence` is ignored). Multiple still-pending completions for
    /// the SAME agent are coalesced into one note (the latest replaces the
    /// earlier) so a noisy child does not cost N extra LLM turns.
    ///
    /// Capacity (#1082 review rounds 2-3): when `pending` is full and the
    /// note is not coalescible it is retained in a bounded overflow buffer
    /// that [`Self::drain_pending`] appends — so a saturated queue delays,
    /// but does not lose, a note. Because coalescing guarantees at most ONE
    /// buffered note per agent, the overflow is sized at
    /// `MAX_DEDUPE_AGENTS` — the same bound as the dedupe map — so a drop
    /// requires more distinct concurrent subagents than the session even
    /// tracks. If that edge is ever hit the dedupe sequence is left
    /// untouched, so the identical sequence stays retryable (and stall
    /// alerts additionally have monitor-side retention).
    pub fn enqueue_subagent_notification(
        &mut self,
        agent_id: String,
        sequence: u64,
        content: String,
        is_completion: bool,
    ) -> NotificationEnqueueOutcome {
        // Dedupe against the monotonic per-agent sequence — the passive broadcast
        // path also records completions, so a repeated/stale sequence is dropped
        // and the note is injected exactly once. Peek-only here: the sequence is
        // recorded only once the note is durably retained below.
        if self
            .last_subagent_notification
            .get(&agent_id)
            .is_some_and(|last| sequence <= *last)
        {
            return NotificationEnqueueOutcome::Duplicate;
        }
        // Coalesce: if a still-pending note for this same agent has not yet been
        // drained, replace it in place (latest wins) instead of queuing a second
        // turn — a noisy child must not cost N extra LLM turns. The pending
        // note's own sequence guards staleness too (#1082 review round 2): if
        // the dedupe watermark for this agent was evicted at capacity, an older
        // sequence must not overwrite a newer pending note.
        if let Some(existing) = self
            .pending
            .iter_mut()
            .chain(self.overflow_notifications.iter_mut())
            .find_map(|m| match m {
                PendingMessage::SubagentNotification { agent_id: id, .. } if *id == agent_id => {
                    Some(m)
                }
                _ => None,
            })
        {
            if let PendingMessage::SubagentNotification {
                sequence: pending_seq,
                ..
            } = existing
                && sequence <= *pending_seq
            {
                return NotificationEnqueueOutcome::Duplicate;
            }
            *existing = PendingMessage::subagent_notification(
                agent_id.clone(),
                sequence,
                content,
                is_completion,
            );
            self.record_subagent_notification(agent_id, sequence);
            return NotificationEnqueueOutcome::Retained;
        }
        let note = PendingMessage::subagent_notification(
            agent_id.clone(),
            sequence,
            content,
            is_completion,
        );
        if self.pending.len() < Self::MAX_PENDING {
            self.pending.push_back(note);
        } else if self.overflow_notifications.len() < Self::MAX_DEDUPE_AGENTS {
            // Full pending queue: retain in the overflow buffer instead of
            // dropping — drained (appended) together with `pending`. Sized at
            // MAX_DEDUPE_AGENTS: coalescing caps the buffers at one note per
            // agent, so this bound cannot be exceeded by real traffic (#1082
            // review round 3 — ordinary completion/error notes have no
            // producer-side retry, so they must not drop here).
            self.overflow_notifications.push_back(note);
        } else {
            // Both buffers full: NOT retained, dedupe sequence NOT advanced,
            // so the identical sequence remains retryable (#1082 review).
            return NotificationEnqueueOutcome::Dropped;
        }
        self.record_subagent_notification(agent_id, sequence);
        NotificationEnqueueOutcome::Retained
    }
}
