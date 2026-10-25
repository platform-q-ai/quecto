//! The SQLite notifier (epic #2705 B10): a committed write publishes its
//! revisions in order to bounded queues; a stalled one is reset.
use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

fn identity() -> Identity {
    Identity {
        name: "Server".into(),
        email: "server@example.com".into(),
    }
}

fn revision(revision: u64) -> BoardRevision {
    BoardRevision { epoch: 1, revision }
}

/// A wake that counts its calls.
fn counted() -> (Arc<AtomicUsize>, FeedWake) {
    let calls = Arc::new(AtomicUsize::new(0));
    let counting = Arc::clone(&calls);
    let wake: FeedWake = Arc::new(move || {
        counting.fetch_add(1, Ordering::SeqCst);
    });
    (calls, wake)
}

fn write(id: u8) -> OpWrite {
    OpWrite {
        write_id: WriteId::parse(&format!("00000000-0000-4000-8000-0000000000{id:02x}")).unwrap(),
        request: format!("cli\n{id}"),
        committer: identity(),
        principal: "local".into(),
    }
}

#[test]
fn a_publish_queues_and_wakes_each_live_subscriber() {
    let hub = FeedHub::default();
    let (calls, wake) = counted();
    let mut subscription = hub.subscribe(wake);
    let (gone_calls, gone_wake) = counted();
    drop(hub.subscribe(gone_wake));
    hub.publish(&[revision(1), revision(2)]);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        gone_calls.load(Ordering::SeqCst),
        0,
        "a dropped one is gone"
    );
    assert_eq!(
        subscription.drain(),
        FeedDrained::Revisions(vec![revision(1), revision(2)])
    );
    assert_eq!(subscription.drain(), FeedDrained::Revisions(vec![]));
}

/// Red test (plan B10): a subscriber that reads nothing while more than
/// its queue is published is reset, and its queue dropped: the publisher
/// never waits on it, and what it holds stays bounded.
#[test]
fn a_stalled_subscriber_is_reset_and_stays_reset() {
    let hub = FeedHub::default();
    let (_, wake) = counted();
    let mut stalled = hub.subscribe(wake);
    let (_, wake) = counted();
    let mut reading = hub.subscribe(wake);
    for at in 1..=QUEUE as u64 {
        hub.publish(&[revision(at)]);
        assert!(matches!(reading.drain(), FeedDrained::Revisions(got) if got.len() == 1));
    }
    hub.publish(&[revision(QUEUE as u64 + 1)]);
    assert_eq!(stalled.drain(), FeedDrained::Reset);
    hub.publish(&[revision(QUEUE as u64 + 2)]);
    assert_eq!(stalled.drain(), FeedDrained::Reset, "a reset is for good");
    assert_eq!(
        reading.drain(),
        FeedDrained::Revisions(vec![revision(QUEUE as u64 + 1), revision(QUEUE as u64 + 2)])
    );
}

/// A committed write publishes the revisions it made; a failed one, or
/// one replayed from its kept answer, publishes none.
#[test]
fn a_committed_write_publishes_its_revisions_and_no_other_does() {
    let dir = tempfile::tempdir().unwrap();
    let ops = SqliteBoardOps::open(&dir.path().join("board.sqlite3"), identity()).unwrap();
    let (calls, wake) = counted();
    let mut subscription = ops.subscribe(wake);
    let made = ops.transact(&write(1), &mut |store| {
        store.bootstrap().map(|_| "made".to_string())
    });
    assert_eq!(made, Ok(Transacted::Answered("made".into())));
    assert_eq!(
        subscription.drain(),
        FeedDrained::Revisions(vec![revision(1)])
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let replayed = ops.transact(&write(1), &mut |_| panic!("a kept answer is replayed"));
    assert_eq!(replayed, Ok(Transacted::Replayed("made".into())));
    let failed = ops.transact(&write(2), &mut |store| {
        store.bootstrap()?;
        Err(BoardStoreError::Unavailable("failed after writing".into()))
    });
    assert!(failed.is_err());
    assert_eq!(subscription.drain(), FeedDrained::Revisions(vec![]));
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "nothing more was published"
    );
}

#[test]
fn the_tip_and_batches_read_the_revisions_table() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("board.sqlite3");
    let ops = SqliteBoardOps::open(&path, identity()).unwrap();
    assert_eq!(
        ops.tip(),
        Ok(FeedTip {
            epoch: 0,
            revision: 0
        }),
        "no board"
    );
    let store = SqliteProjectBoardStore::open(&path, identity()).unwrap();
    store.bootstrap().unwrap();
    let c = store.connection.lock().unwrap();
    let root: String = c
        .query_row("SELECT root FROM revisions WHERE revision=1", [], |r| {
            r.get(0)
        })
        .unwrap();
    for _ in 0..4 {
        storage::ObjectWriter::new(&c)
            .advance(&root, &identity(), "board: edit", None)
            .unwrap();
    }
    drop(c);
    assert_eq!(
        ops.tip(),
        Ok(FeedTip {
            epoch: 1,
            revision: 5
        })
    );
    let batch = ops.revisions(1, BatchLimit::of(3).unwrap()).unwrap();
    assert_eq!(batch.revisions, vec![revision(2), revision(3), revision(4)]);
    assert_eq!(
        batch.tip,
        FeedTip {
            epoch: 1,
            revision: 5
        }
    );
    assert_eq!(
        ops.revisions(5, BatchLimit::MOST).unwrap().revisions,
        vec![]
    );
    assert_eq!(
        ops.revisions(u64::MAX, BatchLimit::MOST).unwrap().revisions,
        vec![],
        "a cursor past any i64 reads nothing"
    );
}

#[test]
fn a_malformed_head_is_a_failure_never_a_revision() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("board.sqlite3");
    let store = SqliteProjectBoardStore::open(&path, identity()).unwrap();
    store.bootstrap().unwrap();
    store
        .connection
        .lock()
        .unwrap()
        .execute("UPDATE revisions SET head='1:7' WHERE revision=1", [])
        .unwrap();
    let ops = SqliteBoardOps::open(&path, identity());
    let read = ops.and_then(|ops| ops.revisions(0, BatchLimit::MOST));
    assert!(
        matches!(read, Err(BoardStoreError::Unavailable(_))),
        "{read:?}"
    );
}
