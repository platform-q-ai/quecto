//! Synchronize a client transcript (#1857): a client's ledger position and
//! the typed reply that reconciles it — a reset to the newest window when
//! the client's epoch or revision no longer describes the retained
//! transcript, else the delta of committed messages after its revision.
//!
//! The values are domain messages and ledger positions; the transport
//! encodes them and decides how many delta messages one frame carries.
use super::history::HistoryPage;
use crate::domain::message::Message;

/// Where a client stands: the epoch it synchronised in and the last
/// revision it holds, plus how many newest messages a reset carries (the
/// protocol's page size, decided by the transport).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncRequest {
    pub epoch: u64,
    pub since_rev: u64,
    pub reset_window: usize,
}

/// A client told to resynchronise: the newest window of the live
/// transcript at the ledger's current position. A reset is always caught
/// up; the client continues from `rev` in `epoch`.
#[derive(Debug, Clone)]
pub struct TranscriptReset {
    pub epoch: u64,
    pub rev: u64,
    pub page: HistoryPage,
}

/// The committed messages after the client's revision, in commit order.
///
/// `next_rev` is present exactly when the frame could not carry every
/// committed message: the revision of the first message left out, which
/// the client names as its position on its next request.
#[derive(Debug, Clone)]
pub struct TranscriptDelta {
    pub epoch: u64,
    pub rev: u64,
    pub messages: Vec<Message>,
    pub next_rev: Option<u64>,
}

impl TranscriptDelta {
    /// Whether the client holds every committed message once it applies
    /// this delta.
    pub fn caught_up(&self) -> bool {
        self.next_rev.is_none()
    }
}

/// What brings a client up to date: a reset is always caught up at its
/// ledger position; a delta is caught up unless it was cut.
#[derive(Debug, Clone)]
pub enum TranscriptSync {
    Reset(TranscriptReset),
    Delta(TranscriptDelta),
}

#[cfg(test)]
#[path = "sync_tests.rs"]
mod tests;
