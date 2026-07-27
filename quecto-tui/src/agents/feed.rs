use crate::agents::ledger::LedgerTranscript;
// Feed freshness is wall-clock staleness measured against real elapsed time, so
// it deliberately uses `std::time::Instant` rather than the pausable
// `tokio::time::Instant` used by roster lifecycle timers.
use std::time::Instant;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FeedAuthority {
    WarmSync,
    SyncedAuthoritative,
}

/// Pure synchronization/authority state for a direct child feed.
pub(crate) struct FeedSyncState {
    pub(crate) epoch: u64,
    pub(crate) rev: u64,
    pub(crate) last_fresh_at: Option<Instant>,
    pub(crate) supports_sync: bool,
    pub(crate) pending_rev: Option<u64>,
    pub(crate) transcript: LedgerTranscript,
    pub(crate) authority: FeedAuthority,
}

impl FeedSyncState {
    pub(crate) fn new(authority: FeedAuthority) -> Self {
        Self {
            epoch: 0,
            rev: 0,
            last_fresh_at: None,
            supports_sync: false,
            pending_rev: None,
            transcript: LedgerTranscript::default(),
            authority,
        }
    }
}
