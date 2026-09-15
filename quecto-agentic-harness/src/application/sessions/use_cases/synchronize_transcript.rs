//! Synchronize a client transcript (#1857): reconcile a client's ledger
//! position (its epoch and last revision) with the active session's read
//! model and select what brings it up to date — a reset to the newest
//! window when the epoch changed or the revision fell below the retained
//! frontier, else the delta of committed messages after the revision.
//!
//! One owner for the idle dispatch loop and the busy reader task: both
//! reconcile against the same read model under one read lock. The
//! transport decides how many delta messages one frame carries (its frame
//! predicate); the application decides what the cut means (`next_rev`).
//! The reset window is the shared paging rule (`history_paging`).
use crate::application::sessions::active_session::{ActiveSessionHandle, ActiveSessionState};
use crate::application::sessions::conversation_ledger::ConversationLedger;
use crate::application::sessions::dto::{
    SyncRequest, TranscriptDelta, TranscriptReset, TranscriptSync,
};
use crate::application::sessions::history_paging::newest_window;
use crate::domain::message::Message;

pub struct SynchronizeTranscript {
    state: ActiveSessionHandle,
}

impl SynchronizeTranscript {
    pub fn new(state: ActiveSessionHandle) -> Self {
        Self { state }
    }

    /// A client must resynchronise when its epoch is not the ledger's,
    /// when its revision lies below the oldest retained committed revision
    /// (an older delta can no longer be reconstructed), or when a retained
    /// revision no longer resolves to a message.
    fn needs_reset(ledger: &ConversationLedger, request: &SyncRequest) -> bool {
        request.epoch != ledger.epoch()
            || ledger
                .frontier()
                .next()
                .is_some_and(|(oldest, _)| request.since_rev < oldest)
            || ledger.frontier().any(|(_, id)| ledger.lookup(id).is_none())
    }

    /// Reconcile `request` against `state`, the caller's consistent view.
    ///
    /// A reset carries the newest `reset_window` messages of the live
    /// transcript as a history page. A delta walks the committed messages
    /// after `since_rev` in commit order and offers each to `carry`, the
    /// transport's frame predicate; the first refusal ends the delta and
    /// its revision becomes `next_rev`.
    fn reconcile(
        state: &ActiveSessionState,
        request: &SyncRequest,
        mut carry: impl FnMut(&Message) -> bool,
    ) -> TranscriptSync {
        let ledger = state.conversation();
        if Self::needs_reset(ledger, request) {
            return TranscriptSync::Reset(TranscriptReset {
                epoch: ledger.epoch(),
                rev: ledger.rev(),
                page: newest_window(ledger.live_messages(), "", request.reset_window),
            });
        }
        let mut messages = Vec::new();
        let mut next_rev = None;
        let committed = ledger
            .frontier()
            .filter(|(rev, _)| *rev > request.since_rev)
            .filter_map(|(rev, id)| ledger.lookup(id).map(|message| (rev, message)));
        for (rev, message) in committed {
            if !carry(message) {
                next_rev = Some(rev);
                break;
            }
            messages.push(message.clone());
        }
        TranscriptSync::Delta(TranscriptDelta {
            epoch: ledger.epoch(),
            rev: ledger.rev(),
            messages,
            next_rev,
        })
    }

    /// Reconcile `request` under the read model's lock (see
    /// [`Self::reconcile`] for what a reset and a delta carry).
    pub async fn execute(
        &self,
        request: &SyncRequest,
        carry: impl FnMut(&Message) -> bool,
    ) -> TranscriptSync {
        let state = self.state.read().await;
        Self::reconcile(&state, request, carry)
    }
}

impl std::fmt::Debug for SynchronizeTranscript {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SynchronizeTranscript")
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
#[path = "synchronize_transcript_tests.rs"]
mod tests;
