//! A follower of a primary board server (epic #2705 B11): it catches up in
//! batches from its place, then follows the primary's change feed, pulling
//! the next batch whenever the feed tells a revision it does not hold (and
//! before each reopen of the feed, so the admin rows and epochs, which make
//! no revision, are copied at least once a connection's lifetime). The feed
//! is followed by the watch client's own rules (`WatchBoard`): its
//! reconnects and busy backoff; past them the follower is stalled, pauses,
//! and starts again. Every loop pauses before it goes round again, so none
//! can spin. A fork (its chain at its last revision differs from the
//! primary's there) is refused: the follower marks itself diverged and
//! stops, never rewriting what it holds.
use std::cell::RefCell;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use super::watch_board::{WatchBoard, WatchTiming};
use crate::application::project_board::ports::feed::{
    BatchLimit, BoardStream, BoardStreams, FeedEvent, OpenRefused, WatchFlow, WatchPauses,
};
use crate::application::project_board::ports::replication::{
    AdminRows, ReplicaError, ReplicaState, ReplicaStatus, ReplicaStore, ReplicationSource,
};

/// A follower's times: the feed's watch, and the pause before it starts
/// again once stalled: `restart`, doubling while it stays stalled, to at
/// most `max_restart`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReplicaTiming {
    pub watch: WatchTiming,
    pub restart: Duration,
    pub max_restart: Duration,
}

impl ReplicaTiming {
    pub const PRODUCTION: Self = Self {
        watch: WatchTiming::PRODUCTION,
        restart: Duration::from_secs(30),
        max_restart: Duration::from_secs(600),
    };
}

/// A catch-up that reached the primary's tip: the batches it took and the
/// revision it applied last.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CaughtUp {
    pub batches: u32,
    pub applied: u64,
}

/// The most batches one catch-up takes: past any board (4,096 revisions a
/// batch of 500 at most, objects 8 MiB a batch) many times over.
const MAX_BATCHES: u32 = 100_000;

/// A follower of its primary.
pub struct ReplicateBoard {
    source: Arc<dyn ReplicationSource>,
    store: Arc<dyn ReplicaStore>,
    admin: Arc<dyn AdminRows>,
    pauses: Arc<dyn WatchPauses>,
    timing: ReplicaTiming,
    status: Arc<Mutex<ReplicaStatus>>,
}

impl ReplicateBoard {
    pub fn new(
        source: Arc<dyn ReplicationSource>,
        store: Arc<dyn ReplicaStore>,
        admin: Arc<dyn AdminRows>,
        pauses: Arc<dyn WatchPauses>,
        timing: ReplicaTiming,
    ) -> Self {
        assert!(
            timing.restart > Duration::ZERO && timing.restart <= timing.max_restart,
            "a restart never spins, and its backoff has a ceiling"
        );
        // A follower found diverged stays so across restarts.
        let state = match store.diverged() {
            Ok(Some(why)) => ReplicaState::Diverged(why),
            Ok(None) => ReplicaState::CatchingUp,
            Err(error) => ReplicaState::Stalled(error.to_string()),
        };
        let status = ReplicaStatus {
            primary: source.primary(),
            applied: store.position().map_or(0, |at| at.revision),
            primary_head: None,
            state,
        };
        Self {
            source,
            store,
            admin,
            pauses,
            timing,
            status: Arc::new(Mutex::new(status)),
        }
    }

    /// The follower's status now.
    pub fn status(&self) -> ReplicaStatus {
        self.status
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// A reader of the follower's status, for its server's `/v1/status`.
    pub fn status_reader(&self) -> Arc<dyn Fn() -> ReplicaStatus + Send + Sync> {
        let status = Arc::clone(&self.status);
        Arc::new(move || {
            status
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone()
        })
    }

    /// Batches from the follower's place until the primary has no more.
    ///
    /// # Errors
    /// A fork ([`ReplicaError::Diverged`], marked in status: the follower
    /// must not go on), or why a batch was not had or applied.
    pub fn catch_up(&self) -> Result<CaughtUp, ReplicaError> {
        if let ReplicaState::Diverged(why) = self.status().state {
            return Err(ReplicaError::Diverged(why));
        }
        let mut batches = 0;
        loop {
            let at = self.store.position()?;
            let batch = self
                .source
                .batch(at.revision, at.objects, BatchLimit::MOST)
                .map_err(|refused| ReplicaError::Unavailable(refused_why(refused)))?;
            batches += 1;
            match (batch.after == at.revision, batch.chain_after == at.chain) {
                (true, true) => {}
                (_, _) => {
                    let why = format!(
                        "this follower's revision {} is not the primary's (its chain differs): \
                         it holds a history the primary does not, and will not rewrite it",
                        at.revision
                    );
                    return Err(self.diverged(why));
                }
            }
            let now = match self.store.apply(&batch) {
                Ok(now) => now,
                Err(ReplicaError::Diverged(why)) => return Err(self.diverged(why)),
                Err(error) => return Err(error),
            };
            self.admin
                .replace(&batch.tokens)
                .map_err(|error| ReplicaError::Unavailable(error.to_string()))?;
            self.record(now.revision, batch.tip);
            match (batch.more, now != at, batches < MAX_BATCHES) {
                (false, _, _) => {
                    return Ok(CaughtUp {
                        batches,
                        applied: now.revision,
                    });
                }
                (true, true, true) => {}
                (true, false, _) => {
                    return Err(ReplicaError::Invalid(
                        "the primary's batch made no progress".into(),
                    ));
                }
                (true, true, false) => {
                    return Err(ReplicaError::Invalid(format!(
                        "the primary still had more after {MAX_BATCHES} batches"
                    )));
                }
            }
        }
    }

    /// Follows the primary until `running` says stop, or the follower
    /// diverges (it then stops for good, marked so).
    pub fn run(&self, running: &dyn Fn() -> bool) {
        /// Marks the follower stalled if it stops (or its thread unwinds)
        /// while its server still runs: its status never says it follows
        /// when nothing does.
        struct Ending<'a>(&'a ReplicateBoard, &'a dyn Fn() -> bool);
        impl Drop for Ending<'_> {
            fn drop(&mut self) {
                if (self.1)() {
                    let why = "the follower stopped following its primary".to_string();
                    self.0.record_state(ReplicaState::Stalled(why));
                }
            }
        }
        let _ending = Ending(self, running);
        let mut restart = self.timing.restart;
        while running() {
            let (halted, caught) = match self.catch_up() {
                Ok(caught) => (self.follow(caught.applied, running), true),
                Err(error) => (Some(error), false),
            };
            match halted {
                Some(ReplicaError::Diverged(_)) => return,
                Some(error) => self.record_state(ReplicaState::Stalled(error.to_string())),
                None => {}
            }
            // Stalled again and again (a primary down, a batch too large),
            // it backs off, doubling to its ceiling; caught up, it starts over.
            restart = match caught {
                true => self.timing.restart,
                false => restart,
            };
            if running() {
                self.pauses.pause(restart);
                restart = restart.saturating_mul(2).min(self.timing.max_restart);
            }
        }
    }

    /// The feed from `applied`, each revision it does not hold pulled in;
    /// why it ended, when it did not end by `running`.
    fn follow(&self, applied: u64, running: &dyn Fn() -> bool) -> Option<ReplicaError> {
        self.record_state(ReplicaState::Following);
        let halted = RefCell::new(None::<ReplicaError>);
        let applied = RefCell::new(applied);
        let pull = || match self.catch_up() {
            Ok(caught) => {
                *applied.borrow_mut() = caught.applied;
                true
            }
            Err(error) => {
                *halted.borrow_mut() = Some(error);
                false
            }
        };
        let opens = Arc::new(AtomicU64::new(0));
        let streams = SourceStreams(Arc::clone(&self.source), Arc::clone(&opens));
        let watch = WatchBoard::new(
            Box::new(streams),
            Arc::clone(&self.pauses),
            self.timing.watch,
        );
        // At most one pull between two opens (none before the first: the
        // catch-up was just made).
        let mut pulled_at = 0;
        let mut before_open = || {
            let opened = opens.load(Ordering::SeqCst);
            let fresh = match opened == pulled_at {
                true => true,
                false => {
                    pulled_at = opened;
                    pull()
                }
            };
            running() && halted.borrow().is_none() && fresh
        };
        let mut each = |event: FeedEvent| {
            let held = *applied.borrow();
            let wanted = match event {
                FeedEvent::Revision(told) => told.revision > held,
                FeedEvent::Epoch { .. } | FeedEvent::Reset { .. } => true,
            };
            match (wanted, running()) {
                (_, false) => WatchFlow::Stop,
                (false, true) => WatchFlow::Continue,
                (true, true) => match pull() {
                    true => WatchFlow::Continue,
                    false => WatchFlow::Stop,
                },
            }
        };
        let from = *applied.borrow();
        let watched = watch.watch(Some(from), &mut before_open, &mut each);
        let halted = halted.into_inner();
        match (halted, watched) {
            (Some(error), _) => Some(error),
            (None, Err(why)) => Some(ReplicaError::Unavailable(why)),
            (None, Ok(())) => None,
        }
    }

    /// Marks the follower diverged, and keeps the mark (so a restart is
    /// diverged too): why.
    fn diverged(&self, why: String) -> ReplicaError {
        let why = match self.store.mark_diverged(&why) {
            Ok(()) => why,
            Err(error) => format!("{why} (the mark was not kept: {error})"),
        };
        self.record_state(ReplicaState::Diverged(why.clone()));
        ReplicaError::Diverged(why)
    }

    fn record_state(&self, state: ReplicaState) {
        let mut status = self.status.lock().unwrap_or_else(PoisonError::into_inner);
        // A diverged follower stays so.
        if let ReplicaState::Diverged(_) = status.state {
            return;
        }
        status.state = state;
    }

    fn record(&self, applied: u64, primary_head: u64) {
        let mut status = self.status.lock().unwrap_or_else(PoisonError::into_inner);
        status.applied = applied;
        status.primary_head = Some(primary_head);
    }
}

/// The primary's feed, as the watch opens it, each open counted.
struct SourceStreams(Arc<dyn ReplicationSource>, Arc<AtomicU64>);

impl BoardStreams for SourceStreams {
    fn open(&self, after: Option<u64>) -> Result<Box<dyn BoardStream>, OpenRefused> {
        self.1.fetch_add(1, Ordering::SeqCst);
        self.0.open(after)
    }
}

fn refused_why(refused: OpenRefused) -> String {
    match refused {
        OpenRefused::Busy(why) | OpenRefused::Refused(why) | OpenRefused::Unreachable(why) => why,
    }
}

#[cfg(test)]
#[path = "replicate_board_tests.rs"]
mod tests;
