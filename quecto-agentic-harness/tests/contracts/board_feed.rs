//! Contract for [`BoardFeed`] (epic #2705 B10), on the SQLite adapter a
//! board server serves: the tip and batches read the committed revisions
//! in order, a batch never past its limit; a subscription made before a
//! write is told every revision the write commits, and woken.
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use quecto::application::project_board::ports::feed::{
    BatchLimit, BoardFeed, BoardRevision, FeedDrained, FeedTip, FeedWake,
};
use quecto::application::project_board::ports::{
    BoardArtifact, BoardOpStore, BoardStoreError, BoardWrite, OpWrite, Transacted, WriteId,
};
use quecto::domain::project_board::entities::claim::Identity;
use quecto::infrastructure::persistence::sqlite::project_board::SqliteBoardOps;
use quecto::infrastructure::test_support::board_artifacts::plan;

pub(super) fn identity() -> Identity {
    Identity {
        name: "Server".into(),
        email: "server@example.com".into(),
    }
}

pub(super) fn revision(revision: u64) -> BoardRevision {
    BoardRevision { epoch: 1, revision }
}

/// A board file with no board yet.
pub(super) fn board() -> (tempfile::TempDir, SqliteBoardOps) {
    let dir = tempfile::tempdir().expect("a temp dir");
    let ops = SqliteBoardOps::open(&dir.path().join("board.sqlite3"), identity()).expect("a board");
    (dir, ops)
}

/// One write, `number` its id's last digits, that bootstraps the board
/// when `bootstrap` and then makes `edits` revisions more.
pub(super) fn wrote(ops: &SqliteBoardOps, number: u16, bootstrap: bool, edits: usize) {
    let write = OpWrite {
        write_id: WriteId::parse(&format!("00000000-0000-4000-8000-00000000{number:04x}"))
            .expect("a write id"),
        request: format!("cli\n{number}"),
        committer: identity(),
        principal: "local".into(),
    };
    let made = ops.transact(&write, &mut |store| {
        if bootstrap {
            store.bootstrap()?;
        }
        let mut head = store.read()?.head;
        for _ in 0..edits {
            match store.write(&head, &[BoardArtifact::Plan(plan("t", 1))], "board: plan")? {
                BoardWrite::Written(next) => head = next,
                other => return Err(BoardStoreError::Unavailable(format!("{other:?}"))),
            }
        }
        Ok("done".to_string())
    });
    assert_eq!(made, Ok(Transacted::Answered("done".into())));
}

/// A wake that counts its calls.
pub(super) fn counted() -> (Arc<AtomicUsize>, FeedWake) {
    let calls = Arc::new(AtomicUsize::new(0));
    let counting = Arc::clone(&calls);
    (
        calls,
        Arc::new(move || {
            counting.fetch_add(1, Ordering::SeqCst);
        }),
    )
}

#[test]
fn a_board_with_none_has_a_zero_tip_and_no_revisions() {
    let (_dir, ops) = board();
    assert_eq!(
        ops.tip(),
        Ok(FeedTip {
            epoch: 0,
            revision: 0
        })
    );
    let batch = ops.revisions(0, BatchLimit::MOST).expect("a batch");
    assert_eq!(batch.revisions, vec![]);
}

#[test]
fn batches_are_the_committed_revisions_in_order_within_their_limit() {
    let (_dir, ops) = board();
    wrote(&ops, 1, true, 6);
    assert_eq!(
        ops.tip(),
        Ok(FeedTip {
            epoch: 1,
            revision: 7
        })
    );
    let first = ops
        .revisions(0, BatchLimit::of(4).expect("4"))
        .expect("a batch");
    assert_eq!(first.revisions, (1..=4).map(revision).collect::<Vec<_>>());
    assert_eq!(
        first.tip,
        FeedTip {
            epoch: 1,
            revision: 7
        }
    );
    let rest = ops.revisions(4, BatchLimit::MOST).expect("a batch");
    assert_eq!(rest.revisions, (5..=7).map(revision).collect::<Vec<_>>());
    assert_eq!(BatchLimit::of(0), None);
    assert_eq!(BatchLimit::of(501), None);
}

#[test]
fn a_subscription_is_told_each_committed_revision_and_woken() {
    let (_dir, ops) = board();
    let (calls, wake) = counted();
    let mut subscription = ops.subscribe(wake);
    wrote(&ops, 1, true, 2);
    assert_eq!(calls.load(Ordering::SeqCst), 1, "one wake for the write");
    assert_eq!(
        subscription.drain(),
        FeedDrained::Revisions((1..=3).map(revision).collect())
    );
    wrote(&ops, 2, false, 1);
    assert_eq!(
        subscription.drain(),
        FeedDrained::Revisions(vec![revision(4)])
    );
}
