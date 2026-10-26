//! A follower's policy (epic #2705 B11) over a scripted primary, an
//! in-memory replica and recorded pauses: batched catch-up, fork refusal,
//! admin rows taken whole, the feed followed, and no loop that spins.
use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};

use super::*;
use crate::application::project_board::ports::TokenError;
use crate::application::project_board::ports::feed::{BoardRevision, StreamRead};
use crate::application::project_board::ports::replication::{
    AdminToken, ReplicaBatch, ReplicaClock, ReplicaPosition, ReplicaRevision,
};
use crate::domain::project_board::value_objects::slug::Slug;
use crate::domain::project_board::value_objects::timestamp::Timestamp;

/// Calls past which a follower is a runaway.
const RUNAWAY: usize = 10_000;

fn chain(revision: u64) -> String {
    format!("c{revision}")
}

fn revision(revision: u64) -> ReplicaRevision {
    ReplicaRevision {
        head: format!("1:{revision}"),
        root: "r".into(),
        name: "Ada".into(),
        email: "ada@example.com".into(),
        revision,
        parent: revision.checked_sub(1).filter(|parent| *parent > 0),
        message: "board: edit".into(),
        at: 0,
        git_commit: None,
        git_meta: None,
        principal: None,
        chain: chain(revision),
    }
}

fn token(name: &str) -> AdminToken {
    AdminToken {
        name: Slug::parse(name).unwrap(),
        created: Timestamp::parse("2026-10-26T00:00:00Z").unwrap(),
        kind: crate::application::project_board::ports::TokenKind::Device,
        digest: "0".repeat(64),
    }
}

/// A primary of `tip` revisions (more as the test adds them), its tokens
/// and feed scripted; each batch asked counted.
struct Primary {
    tip: Mutex<u64>,
    tokens: Mutex<Vec<AdminToken>>,
    batches: AtomicUsize,
    unreachable: bool,
    feed: Mutex<VecDeque<Vec<StreamRead>>>,
    /// The tip it grows to as its feed is next opened.
    grows_to: Mutex<Option<u64>>,
}

impl Primary {
    fn of(tip: u64) -> Arc<Self> {
        Arc::new(Self::plain(tip))
    }

    fn plain(tip: u64) -> Self {
        Self {
            tip: Mutex::new(tip),
            tokens: Mutex::new(vec![token("phone")]),
            batches: AtomicUsize::new(0),
            unreachable: false,
            feed: Mutex::new(VecDeque::new()),
            grows_to: Mutex::new(None),
        }
    }
}

struct Played(VecDeque<StreamRead>);

impl BoardStream for Played {
    fn read(&mut self) -> StreamRead {
        self.0.pop_front().unwrap_or(StreamRead::Ended)
    }

    fn retry(&self) -> Option<Duration> {
        None
    }
}

impl ReplicationSource for Primary {
    fn batch(
        &self,
        after: u64,
        _objects_after: u64,
        limit: BatchLimit,
    ) -> Result<ReplicaBatch, OpenRefused> {
        let asked = self.batches.fetch_add(1, Ordering::SeqCst);
        assert!(asked < RUNAWAY, "the follower asked {RUNAWAY} batches");
        if self.unreachable {
            return Err(OpenRefused::Unreachable("no primary".into()));
        }
        let tip = *self.tip.lock().unwrap();
        let last = tip.min(after + limit.get() as u64);
        Ok(ReplicaBatch {
            after,
            chain_after: (1..=tip).contains(&after).then(|| chain(after)),
            objects: vec![],
            revisions: (after + 1..=last).map(revision).collect(),
            epochs: vec![],
            meta: None,
            tokens: self.tokens.lock().unwrap().clone(),
            tip,
            more: last < tip,
        })
    }

    fn open(&self, _after: Option<u64>) -> Result<Box<dyn BoardStream>, OpenRefused> {
        if let Some(tip) = self.grows_to.lock().unwrap().take() {
            *self.tip.lock().unwrap() = tip;
        }
        match self.feed.lock().unwrap().pop_front() {
            Some(reads) => Ok(Box::new(Played(reads.into()))),
            None => Err(OpenRefused::Refused("the feed is done".into())),
        }
    }

    fn primary(&self) -> String {
        "http://100.64.0.1:7979".into()
    }
}

/// A replica in memory: its revisions' chains, each applied in order.
#[derive(Default)]
struct Replica {
    chains: Mutex<Vec<String>>,
    /// Its diverged mark, kept as a store keeps it across restarts.
    mark: Mutex<Option<String>>,
}

impl ReplicaStore for Replica {
    fn position(&self) -> Result<ReplicaPosition, ReplicaError> {
        let chains = self.chains.lock().unwrap();
        Ok(ReplicaPosition {
            revision: chains.len() as u64,
            chain: chains.last().cloned(),
            objects: 0,
        })
    }

    fn apply(&self, batch: &ReplicaBatch) -> Result<ReplicaPosition, ReplicaError> {
        {
            let mut chains = self.chains.lock().unwrap();
            for revision in &batch.revisions {
                assert_eq!(revision.revision, chains.len() as u64 + 1, "in order");
                chains.push(revision.chain.clone());
            }
        }
        self.position()
    }

    fn diverged(&self) -> Result<Option<String>, ReplicaError> {
        Ok(self.mark.lock().unwrap().clone())
    }

    fn mark_diverged(&self, why: &str) -> Result<(), ReplicaError> {
        *self.mark.lock().unwrap() = Some(why.to_string());
        Ok(())
    }
}

#[derive(Default)]
struct Admin(Mutex<Vec<Vec<AdminToken>>>);

impl AdminRows for Admin {
    fn export(&self) -> Result<Vec<AdminToken>, TokenError> {
        Ok(self.0.lock().unwrap().last().cloned().unwrap_or_default())
    }

    fn replace(&self, rows: &[AdminToken]) -> Result<(), TokenError> {
        self.0.lock().unwrap().push(rows.to_vec());
        Ok(())
    }
}

#[derive(Default)]
struct Recorded(Mutex<Vec<Duration>>);

impl WatchPauses for Recorded {
    fn pause(&self, how_long: Duration) {
        let mut paused = self.0.lock().unwrap();
        assert!(
            paused.len() < RUNAWAY,
            "the follower paused {RUNAWAY} times"
        );
        paused.push(how_long);
    }
}

const FAST: ReplicaTiming = ReplicaTiming {
    watch: WatchTiming {
        pause: Duration::from_millis(1),
        min_backoff: Duration::from_millis(1),
        max_backoff: Duration::from_millis(8),
        busy_for: Duration::from_millis(100),
        busy_tries: 4,
        attempts: 2,
    },
    restart: Duration::from_millis(7),
    max_restart: Duration::from_millis(28),
};

struct Follower {
    primary: Arc<Primary>,
    replica: Arc<Replica>,
    admin: Arc<Admin>,
    pauses: Arc<Recorded>,
    follower: ReplicateBoard,
}

fn following(primary: Arc<Primary>, replica: Replica) -> Follower {
    let (replica, admin, pauses) = (
        Arc::new(replica),
        Arc::new(Admin::default()),
        Arc::new(Recorded::default()),
    );
    let follower = ReplicateBoard::new(
        Arc::clone(&primary) as _,
        Arc::clone(&replica) as _,
        Arc::clone(&admin) as _,
        Arc::clone(&pauses) as _,
        FAST,
        Arc::new(FakeClock::default()) as _,
    );
    Follower {
        primary,
        replica,
        admin,
        pauses,
        follower,
    }
}

#[test]
fn a_follower_of_1500_revisions_catches_up_in_at_most_3_batches() {
    let f = following(Primary::of(1_500), Replica::default());
    let caught = f.follower.catch_up().expect("caught up");
    assert!(caught.batches <= 3, "{caught:?}");
    assert_eq!(caught.applied, 1_500);
    assert_eq!(
        f.primary.batches.load(Ordering::SeqCst),
        caught.batches as usize
    );
    let status = f.follower.status();
    assert_eq!(
        (status.applied, status.primary_head),
        (1_500, Some(1_500)),
        "the lag shows in status"
    );
    assert_eq!(status.primary, "http://100.64.0.1:7979");
}

#[test]
fn a_follower_whose_last_chain_differs_is_refused_and_marked_diverged() {
    let forked = Replica {
        chains: Mutex::new(vec![chain(1), "forked".into()]),
        ..Replica::default()
    };
    let f = following(Primary::of(5), forked);
    let refused = f.follower.catch_up();
    assert!(
        matches!(&refused, Err(ReplicaError::Diverged(why)) if why.contains("revision 2")),
        "{refused:?}"
    );
    assert_eq!(
        f.replica.chains.lock().unwrap().len(),
        2,
        "nothing rewritten"
    );
    assert_eq!(f.follower.status().state.as_str(), "diverged");
    // A diverged follower stops for good: no pause, no batch more.
    f.follower.run(&|| true);
    assert_eq!(f.primary.batches.load(Ordering::SeqCst), 1);
    assert!(f.pauses.0.lock().unwrap().is_empty());
}

#[test]
fn a_follower_ahead_of_its_primary_is_diverged() {
    let ahead = Replica {
        chains: Mutex::new((1..=6).map(chain).collect()),
        ..Replica::default()
    };
    let f = following(Primary::of(5), ahead);
    assert!(matches!(
        f.follower.catch_up(),
        Err(ReplicaError::Diverged(_))
    ));
}

#[test]
fn each_batch_takes_the_admin_rows_whole() {
    let f = following(Primary::of(3), Replica::default());
    f.follower.catch_up().unwrap();
    f.primary.tokens.lock().unwrap().clear();
    f.follower.catch_up().unwrap();
    let replaced = f.admin.0.lock().unwrap();
    assert_eq!(replaced.first(), Some(&vec![token("phone")]));
    assert_eq!(
        replaced.last(),
        Some(&vec![]),
        "a revoked token is revoked here"
    );
}

#[test]
fn a_batch_that_says_more_but_makes_no_progress_is_refused() {
    struct Stuck;
    impl ReplicationSource for Stuck {
        fn batch(&self, after: u64, _: u64, _: BatchLimit) -> Result<ReplicaBatch, OpenRefused> {
            Ok(ReplicaBatch {
                after,
                chain_after: None,
                objects: vec![],
                revisions: vec![],
                epochs: vec![],
                meta: None,
                tokens: vec![],
                tip: 9,
                more: true,
            })
        }
        fn open(&self, _: Option<u64>) -> Result<Box<dyn BoardStream>, OpenRefused> {
            Err(OpenRefused::Refused("no".into()))
        }
        fn primary(&self) -> String {
            "unix:///p".into()
        }
    }
    let follower = ReplicateBoard::new(
        Arc::new(Stuck),
        Arc::new(Replica::default()),
        Arc::new(Admin::default()),
        Arc::new(Recorded::default()),
        FAST,
        Arc::new(FakeClock::default()) as _,
    );
    assert!(matches!(follower.catch_up(), Err(ReplicaError::Invalid(_))));
}

#[test]
fn an_unreachable_primary_stalls_the_follower_which_pauses_before_each_try() {
    let primary = Arc::new(Primary {
        unreachable: true,
        ..Primary::plain(1)
    });
    let f = following(primary, Replica::default());
    let rounds = AtomicUsize::new(0);
    f.follower
        .run(&|| rounds.fetch_add(1, Ordering::SeqCst) < 16);
    let status = f.follower.status();
    assert_eq!(status.state.as_str(), "stalled");
    assert!(status.state.reason().unwrap().contains("no primary"));
    let paused = f.pauses.0.lock().unwrap();
    let batches = f.primary.batches.load(Ordering::SeqCst);
    assert!(
        batches >= 1 && paused.len() >= batches,
        "{batches} tries, {paused:?}"
    );
    // Stalled again and again, it backs off, doubling to its ceiling.
    let ms = |millis| Duration::from_millis(millis);
    assert_eq!(paused[..4], [ms(7), ms(14), ms(28), ms(28)], "{paused:?}");
}

/// A clock the test sets.
#[derive(Default)]
struct FakeClock(Mutex<Duration>);

impl FakeClock {
    fn set(&self, at: Duration) {
        *self.0.lock().unwrap() = at;
    }
}

impl ReplicaClock for FakeClock {
    fn now(&self) -> Duration {
        *self.0.lock().unwrap()
    }
}

/// A primary that is up or down as the test says, with no revision.
struct Flaky(std::sync::atomic::AtomicBool);

impl ReplicationSource for Flaky {
    fn batch(&self, after: u64, _: u64, _: BatchLimit) -> Result<ReplicaBatch, OpenRefused> {
        match self.0.load(Ordering::SeqCst) {
            true => Ok(ReplicaBatch {
                after,
                chain_after: None,
                objects: vec![],
                revisions: vec![],
                epochs: vec![],
                meta: None,
                tokens: vec![],
                tip: 0,
                more: false,
            }),
            false => Err(OpenRefused::Unreachable("no primary".into())),
        }
    }

    fn open(&self, _: Option<u64>) -> Result<Box<dyn BoardStream>, OpenRefused> {
        Err(OpenRefused::Unreachable("no primary".into()))
    }

    fn primary(&self) -> String {
        "unix:///p".into()
    }
}

/// B11 review 4: with production timing, a follower serves devices from
/// its first catch-up since start; once stalled, for [`STALE_AFTER`] by
/// the clock (not by its pauses), then no longer; caught up again, it is
/// timed afresh, and a brief stall after it still serves.
#[test]
fn a_stalled_follower_serves_devices_for_ten_minutes_by_the_clock() {
    let (clock, primary) = (
        Arc::new(FakeClock::default()),
        Arc::new(Flaky(std::sync::atomic::AtomicBool::new(true))),
    );
    let follower = ReplicateBoard::new(
        Arc::clone(&primary) as _,
        Arc::new(Replica::default()),
        Arc::new(Admin::default()),
        Arc::new(Recorded::default()),
        ReplicaTiming::PRODUCTION,
        Arc::clone(&clock) as _,
    );
    let serves = follower.devices_reader();
    assert!(!serves(), "not before its first catch-up");
    follower.catch_up().unwrap();
    assert!(serves());
    let stall = || {
        primary.0.store(false, Ordering::SeqCst);
        let rounds = AtomicUsize::new(0);
        follower.run(&|| rounds.fetch_add(1, Ordering::SeqCst) < 1);
        assert_eq!(follower.status().state.as_str(), "stalled");
    };
    clock.set(Duration::from_secs(100));
    stall();
    clock.set(Duration::from_secs(100) + STALE_AFTER - Duration::from_secs(1));
    assert!(serves(), "stalled 9 min 59 s");
    clock.set(Duration::from_secs(100) + STALE_AFTER);
    assert!(!serves(), "stalled 10 min");
    // The primary returns: caught up, timed afresh; a brief stall serves.
    primary.0.store(true, Ordering::SeqCst);
    follower.catch_up().unwrap();
    assert!(serves());
    stall();
    clock.set(Duration::from_secs(100) + STALE_AFTER + Duration::from_secs(30));
    assert!(serves(), "stalled 30 s since it last caught up");
}

/// Pauses that move the fake clock on by each pause.
struct Ticking(Arc<FakeClock>);

impl WatchPauses for Ticking {
    fn pause(&self, how_long: Duration) {
        self.0.set(self.0.now() + how_long);
    }
}

/// B11 review 5: run, stalled round after round with the clock moving by
/// its pauses, a follower is timed from its FIRST stall: it serves devices
/// for 10 min from then and none after, though each round stalls afresh.
#[test]
fn a_follower_stalled_round_after_round_is_timed_from_its_first_stall() {
    let clock = Arc::new(FakeClock::default());
    let primary = Arc::new(Flaky(std::sync::atomic::AtomicBool::new(true)));
    let follower = ReplicateBoard::new(
        Arc::clone(&primary) as _,
        Arc::new(Replica::default()),
        Arc::new(Admin::default()),
        Arc::new(Ticking(Arc::clone(&clock))),
        ReplicaTiming::PRODUCTION,
        Arc::clone(&clock) as _,
    );
    let serves = follower.devices_reader();
    follower.catch_up().unwrap();
    primary.0.store(false, Ordering::SeqCst);
    let start = Duration::from_secs(100);
    clock.set(start);
    // Seen at each check, and judged once run returns (a panic in `running`
    // would panic again as the follower's end is marked).
    let seen = Mutex::new(Vec::new());
    follower.run(&|| {
        let stalled = clock.now() - start;
        seen.lock().unwrap().push((stalled, serves()));
        stalled < STALE_AFTER * 3
    });
    let seen = seen.into_inner().unwrap();
    for &(stalled, served) in &seen {
        assert_eq!(served, stalled < STALE_AFTER, "stalled {stalled:?}");
    }
    let refused = seen.iter().filter(|(_, served)| !served).count();
    assert!(refused >= 2, "refused round after round: {seen:?}");
}

/// B11 review 4: a restart clears no stall into serving: a follower
/// started again serves no device until it has caught up.
#[test]
fn a_restarted_follower_serves_no_device_until_it_catches_up() {
    let primary = Arc::new(Flaky(std::sync::atomic::AtomicBool::new(false)));
    let follower = ReplicateBoard::new(
        Arc::clone(&primary) as _,
        Arc::new(Replica::default()),
        Arc::new(Admin::default()),
        Arc::new(Recorded::default()),
        ReplicaTiming::PRODUCTION,
        Arc::new(FakeClock::default()),
    );
    let serves = follower.devices_reader();
    assert!(follower.catch_up().is_err());
    assert!(!serves(), "catching up with an old token set");
    primary.0.store(true, Ordering::SeqCst);
    follower.catch_up().unwrap();
    assert!(serves());
}

/// B11 review: a diverged follower is kept so across a restart: it asks
/// its primary nothing and stays diverged.
#[test]
fn a_diverged_follower_stays_diverged_across_a_restart() {
    let forked = Replica {
        chains: Mutex::new(vec![chain(1), "forked".into()]),
        ..Replica::default()
    };
    let f = following(Primary::of(5), forked);
    assert!(f.follower.catch_up().is_err());
    let why = f.replica.mark.lock().unwrap().clone();
    assert!(
        why.as_deref().is_some_and(|why| why.contains("revision 2")),
        "{why:?}"
    );
    // The server restarts: a new follower over the same store.
    let restarted = ReplicateBoard::new(
        Arc::clone(&f.primary) as _,
        Arc::clone(&f.replica) as _,
        Arc::clone(&f.admin) as _,
        Arc::clone(&f.pauses) as _,
        FAST,
        Arc::new(FakeClock::default()) as _,
    );
    assert_eq!(restarted.status().state.as_str(), "diverged");
    let asked = f.primary.batches.load(Ordering::SeqCst);
    restarted.run(&|| true);
    assert!(matches!(
        restarted.catch_up(),
        Err(ReplicaError::Diverged(_))
    ));
    assert_eq!(
        f.primary.batches.load(Ordering::SeqCst),
        asked,
        "nothing asked"
    );
    assert_eq!(restarted.status().state.as_str(), "diverged");
}

/// B11 review: a follower whose thread dies while its server runs is
/// shown stalled, never following.
#[test]
fn a_follower_that_dies_is_shown_stalled() {
    struct Dying;
    impl ReplicationSource for Dying {
        fn batch(&self, _: u64, _: u64, _: BatchLimit) -> Result<ReplicaBatch, OpenRefused> {
            panic!("the follower's thread dies");
        }
        fn open(&self, _: Option<u64>) -> Result<Box<dyn BoardStream>, OpenRefused> {
            Err(OpenRefused::Refused("no".into()))
        }
        fn primary(&self) -> String {
            "unix:///p".into()
        }
    }
    let follower = ReplicateBoard::new(
        Arc::new(Dying),
        Arc::new(Replica::default()),
        Arc::new(Admin::default()),
        Arc::new(Recorded::default()),
        FAST,
        Arc::new(FakeClock::default()) as _,
    );
    let died = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| follower.run(&|| true)));
    assert!(died.is_err());
    let status = follower.status();
    assert_eq!(status.state.as_str(), "stalled");
    assert!(status.state.reason().unwrap().contains("stopped following"));
}

#[test]
fn the_feed_pulls_each_revision_the_follower_does_not_hold() {
    let primary = Primary::of(2);
    let told = |revision| StreamRead::Event {
        id: revision,
        event: FeedEvent::Revision(BoardRevision { epoch: 1, revision }),
    };
    primary.feed.lock().unwrap().push_back(vec![
        StreamRead::Event {
            id: 2,
            event: FeedEvent::Epoch { epoch: 1 },
        },
        told(3),
    ]);
    // The primary makes revision 3 as the follower opens its feed.
    *primary.grows_to.lock().unwrap() = Some(3);
    let f = following(primary, Replica::default());
    let rounds = AtomicUsize::new(0);
    // It runs until its third batch: the catch-up (to 2), the feed's
    // revision 3, and one before the feed would be opened again.
    let running = || {
        let asked = f.primary.batches.load(Ordering::SeqCst);
        rounds.fetch_add(1, Ordering::SeqCst) < 40 && asked < 3
    };
    f.follower.run(&running);
    assert_eq!(f.replica.chains.lock().unwrap().len(), 3);
    let status = f.follower.status();
    assert_eq!((status.applied, status.primary_head), (3, Some(3)));
    assert_eq!(f.primary.batches.load(Ordering::SeqCst), 3);
    assert_eq!(status.state.as_str(), "following");
    assert!(
        f.pauses.0.lock().unwrap().is_empty(),
        "stopped before a pause"
    );
}
