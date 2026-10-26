//! Contract for replication's ports (epic #2705 B11), on the adapters a
//! board server composes: [`ReplicationLog`] and [`ReplicaStore`] on its
//! SQLite board, [`AdminRows`] on its tokens file. A follower catches up
//! in batches to a copy byte-identical to its primary's, refuses a fork,
//! applies a batch whole or not at all, and takes the admin rows whole.
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use quecto::application::project_board::ports::feed::{
    BatchLimit, BoardStream, OpenRefused, WatchPauses,
};
use quecto::application::project_board::ports::replication::{
    AdminRows, ReplicaBatch, ReplicaError, ReplicaStore, ReplicationLog, ReplicationSource,
};
use quecto::application::project_board::ports::{
    BoardArtifact, BoardOpStore, BoardStoreError, BoardTokens, BoardWrite, OpWrite, Transacted,
    WriteId,
};
use quecto::application::project_board::use_cases::{
    ReplicaTiming, ReplicateBoard, ServeReplication,
};
use quecto::domain::project_board::entities::claim::Identity;
use quecto::domain::project_board::value_objects::slug::Slug;
use quecto::domain::project_board::value_objects::timestamp::Timestamp;
use quecto::infrastructure::persistence::sqlite::project_board::SqliteBoardOps;
use quecto::infrastructure::test_support::board_artifacts::plan;

use super::board_feed::wrote;
use super::replication_log::{Server, chains, dump, server};

/// A primary reached in process: its batches counted.
struct Local {
    served: ServeReplication,
    batches: AtomicUsize,
}

impl ReplicationSource for Local {
    fn batch(
        &self,
        after: u64,
        objects_after: u64,
        limit: BatchLimit,
    ) -> Result<ReplicaBatch, OpenRefused> {
        let asked = self.batches.fetch_add(1, Ordering::SeqCst);
        assert!(asked < 1_000, "a runaway follower");
        (self.served.batch(after, objects_after, limit))
            .map_err(|error| OpenRefused::Refused(format!("{error:?}")))
    }

    fn open(&self, _: Option<u64>) -> Result<Box<dyn BoardStream>, OpenRefused> {
        Err(OpenRefused::Refused("no feed in process".into()))
    }

    fn primary(&self) -> String {
        "unix:///primary/board.sock".into()
    }
}

struct NoPause;

impl WatchPauses for NoPause {
    fn pause(&self, _: Duration) {}
}

/// `follower` following `primary` in process.
fn following(primary: &Server, follower: &Server) -> (Arc<Local>, ReplicateBoard) {
    let local = Arc::new(Local {
        served: ServeReplication::new(
            Arc::clone(&primary.ops) as Arc<dyn ReplicationLog>,
            Arc::clone(&primary.tokens) as Arc<dyn AdminRows>,
        ),
        batches: AtomicUsize::new(0),
    });
    let replicate = ReplicateBoard::new(
        Arc::clone(&local) as _,
        Arc::clone(&follower.ops) as Arc<dyn ReplicaStore>,
        Arc::clone(&follower.tokens) as Arc<dyn AdminRows>,
        Arc::new(NoPause),
        ReplicaTiming::PRODUCTION,
        Arc::new(quecto::infrastructure::time::replica_clock::MonotonicReplicaClock::default()),
    );
    (local, replicate)
}

/// One write of `edits` plans as `committer`, bootstrapping first.
fn written_as(ops: &SqliteBoardOps, committer: Identity, task: &str, edits: usize) {
    let write = OpWrite {
        write_id: WriteId::parse("00000000-0000-4000-8000-0000000000ff").unwrap(),
        request: "cli\nforked".into(),
        committer,
        principal: "token:laptop".into(),
    };
    let made = ops.transact(&write, &mut |store| {
        store.bootstrap()?;
        let mut head = store.read()?.head;
        for _ in 0..edits {
            match store.write(&head, &[BoardArtifact::Plan(plan(task, 1))], "board: plan")? {
                BoardWrite::Written(next) => head = next,
                other => return Err(BoardStoreError::Unavailable(format!("{other:?}"))),
            }
        }
        Ok("done".to_string())
    });
    assert_eq!(made, Ok(Transacted::Answered("done".into())));
}

#[test]
fn a_follower_of_1500_revisions_catches_up_in_3_batches_to_a_byte_identical_copy() {
    let (primary, follower) = (server(), server());
    wrote(&primary.ops, 1, true, 1_499);
    let (local, replicate) = following(&primary, &follower);
    let caught = replicate.catch_up().expect("caught up");
    assert!(caught.batches <= 3, "{caught:?}");
    assert_eq!(
        local.batches.load(Ordering::SeqCst),
        caught.batches as usize
    );
    assert_eq!(caught.applied, 1_500);
    let (primary_rows, follower_rows) = (dump(&primary.file), dump(&follower.file));
    assert!(primary_rows.len() > 1_500, "{} rows", primary_rows.len());
    assert!(primary_rows == follower_rows, "the copy differs");
    let snapshot = |ops: &SqliteBoardOps| format!("{:?}", ops.reads().read().unwrap());
    assert_eq!(snapshot(&follower.ops), snapshot(&primary.ops));
    assert!(chains(&follower.file).iter().all(Option::is_some));
    // Caught up, it asks once more and takes nothing.
    assert_eq!(replicate.catch_up().unwrap().batches, 1);
    assert!(dump(&follower.file) == primary_rows);
}

#[test]
fn a_follower_catches_up_again_after_more_writes() {
    let (primary, follower) = (server(), server());
    wrote(&primary.ops, 1, true, 3);
    let (_, replicate) = following(&primary, &follower);
    replicate.catch_up().unwrap();
    wrote(&primary.ops, 2, false, 2);
    assert_eq!(replicate.catch_up().unwrap().applied, 6);
    assert!(dump(&follower.file) == dump(&primary.file));
    let status = replicate.status();
    assert_eq!((status.applied, status.primary_head), (6, Some(6)));
}

#[test]
fn a_follower_whose_last_chain_differs_is_refused_and_marked_diverged() {
    let (primary, follower) = (server(), server());
    wrote(&primary.ops, 1, true, 2);
    let other = Identity {
        name: "Elsewhere".into(),
        email: "elsewhere@example.com".into(),
    };
    written_as(&follower.ops, other, "t", 2);
    let before = dump(&follower.file);
    let (_, replicate) = following(&primary, &follower);
    let refused = replicate.catch_up();
    assert!(
        matches!(&refused, Err(ReplicaError::Diverged(_))),
        "{refused:?}"
    );
    assert_eq!(replicate.status().state.as_str(), "diverged");
    assert!(dump(&follower.file) == before, "a fork is never rewritten");
    // The mark is kept beside the board: a restart is diverged too.
    let why = follower.ops.diverged().unwrap();
    assert!(why.is_some_and(|why| why.contains("chain differs")));
    let (local, restarted) = following(&primary, &follower);
    assert_eq!(restarted.status().state.as_str(), "diverged");
    assert!(matches!(
        restarted.catch_up(),
        Err(ReplicaError::Diverged(_))
    ));
    assert_eq!(local.batches.load(Ordering::SeqCst), 0, "it asks nothing");
}

#[test]
fn a_revoked_token_is_replicated() {
    let (primary, follower) = (server(), server());
    wrote(&primary.ops, 1, true, 0);
    let at = Timestamp::parse("2026-10-26T09:00:00Z").unwrap();
    let name = |text: &str| Slug::parse(text).unwrap();
    let phone = primary.tokens.add(&name("phone"), &at).unwrap();
    let laptop = primary.tokens.add(&name("laptop"), &at).unwrap();
    let (_, replicate) = following(&primary, &follower);
    replicate.catch_up().unwrap();
    assert_eq!(
        follower.tokens.verify(&laptop.secret),
        Ok(Some(name("laptop")))
    );
    assert_eq!(
        follower.tokens.verify(&phone.secret),
        Ok(Some(name("phone")))
    );
    primary.tokens.revoke(&name("laptop")).unwrap();
    replicate.catch_up().unwrap();
    assert_eq!(follower.tokens.verify(&laptop.secret), Ok(None));
    assert_eq!(
        follower.tokens.verify(&phone.secret),
        Ok(Some(name("phone")))
    );
    assert_eq!(follower.tokens.list(), primary.tokens.list());
    assert_eq!(follower.tokens.export(), primary.tokens.export());
}
