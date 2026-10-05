use crate::agents::ledger::LedgerTranscript;
// Feed freshness is wall-clock staleness measured against real elapsed time, so
// it deliberately uses `std::time::Instant` rather than the pausable
// `tokio::time::Instant` used by roster lifecycle timers.
use std::time::{Duration, Instant};

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
    pub(crate) stats_refresh: StatsRefresh,
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
            stats_refresh: StatsRefresh::Settled,
        }
    }
}

/// How long an unanswered session-stats request holds back the next one. A
/// child answers queued requests only once its prompt ends, so a long run
/// keeps a request outstanding; past this grace a fresh trigger may send
/// again, in case the reply was lost (a lagging broadcast receiver can drop
/// messages).
pub(crate) const STATS_REPLY_GRACE: Duration = Duration::from_secs(30);

/// Whether a child's own session-stats request is outstanding, so run ends
/// in quick succession queue at most one request plus one follow-up rather
/// than one each in the child's dispatch queue. Replies and run ends share
/// the child's ordered broadcast stream, so the follow-up is a safety net
/// for replies that answered an earlier request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StatsRefresh {
    /// No request outstanding and none owed.
    Settled,
    /// A request went out at `sent`; `stale` records a trigger seen since.
    Pending { sent: Instant, stale: bool },
    /// The stats are stale but the request could not be queued: it is sent
    /// at the next chance (a trigger, a reply, or any event from the child).
    Owed,
}

impl StatsRefresh {
    /// A request was sent at `now` (the connect-time request).
    pub(crate) fn sent_at(now: Instant) -> Self {
        Self::Pending {
            sent: now,
            stale: false,
        }
    }

    /// The child's stats went stale at `now`: whether to send a request now.
    pub(crate) fn should_request(&mut self, now: Instant) -> bool {
        let send = match *self {
            Self::Settled | Self::Owed => true,
            Self::Pending { sent, .. } => now.saturating_duration_since(sent) >= STATS_REPLY_GRACE,
        };
        *self = match (send, *self) {
            (true, _) => Self::sent_at(now),
            (false, Self::Pending { sent, .. }) => Self::Pending { sent, stale: true },
            (false, unchanged) => unchanged,
        };
        debug_assert!(
            !send || *self == Self::sent_at(now),
            "a send leaves one fresh request"
        );
        send
    }

    /// A stats reply arrived at `now`: whether to send a request for a
    /// trigger seen while one was outstanding, or for one that is owed.
    pub(crate) fn answered(&mut self, now: Instant) -> bool {
        let send = match *self {
            Self::Pending { stale, .. } => stale,
            Self::Owed => true,
            Self::Settled => false,
        };
        *self = match send {
            true => Self::sent_at(now),
            false => Self::Settled,
        };
        debug_assert!(
            !matches!(*self, Self::Pending { stale: true, .. }),
            "a reply never leaves a trigger waiting"
        );
        send
    }

    /// The request could not be queued: it is owed.
    pub(crate) fn not_sent(&mut self) {
        *self = Self::Owed;
    }

    /// Whether a request is owed: one that could not be queued.
    pub(crate) fn owes_request(&self) -> bool {
        *self == Self::Owed
    }
}

#[cfg(test)]
#[path = "feed_stats_refresh_tests.rs"]
mod feed_stats_refresh_tests;
