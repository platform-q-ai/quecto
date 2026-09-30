//! #2338: a pure read runs in a deferred, read-only transaction. Under the
//! board's rollback journal it takes only a shared lock while it reads, so
//! a writer holding the write lock (between `BEGIN IMMEDIATE` and its
//! commit) neither refuses it nor makes it wait, and any write it attempts
//! is refused by the store.
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use rusqlite::Connection;
use serde_json::json;

use super::SqliteBoardRepository;
use crate::application::swarm::dto::BoardLocation;
use crate::application::swarm::ports::{BoardCallMeter, BoardRepository};
use crate::application::swarm::use_cases::ReadEventCursor;
use crate::domain::swarm::RefusalKind;
use crate::infrastructure::persistence::swarm_board::meter::SqliteBoardCallMeter;
use crate::infrastructure::persistence::swarm_board::store::BoardStore;

/// A created board and its plain repository.
fn created() -> (tempfile::TempDir, BoardStore, SqliteBoardRepository) {
    let dir = tempfile::TempDir::new().unwrap();
    let location = BoardLocation {
        database: dir.path().join("swarm.sqlite"),
        checkout: dir.path().to_path_buf(),
    };
    let store = BoardStore::new(&location.database);
    store.transaction(true, |_| Ok(())).unwrap();
    (dir, store, SqliteBoardRepository::new(&location))
}

/// A second connection holding the write lock (`BEGIN IMMEDIATE`) on the
/// board until the returned sender is dropped.
fn writer_holding(store: &BoardStore) -> (mpsc::Sender<()>, thread::JoinHandle<()>) {
    let path = store.path().to_path_buf();
    let (held, taken) = mpsc::channel();
    let (release, released) = mpsc::channel::<()>();
    let thread = thread::spawn(move || {
        let connection = Connection::open(path).unwrap();
        connection.execute_batch("BEGIN IMMEDIATE").unwrap();
        held.send(()).unwrap();
        let _told = released.recv_timeout(Duration::from_secs(30));
        connection.execute_batch("COMMIT").unwrap();
    });
    taken.recv_timeout(Duration::from_secs(30)).unwrap();
    (release, thread)
}

#[test]
fn the_event_cursor_is_read_while_a_writer_holds_the_write_lock() {
    let (_dir, store, repository) = created();
    let (release, writer) = writer_holding(&store);
    let started = Instant::now();
    let cursor = ReadEventCursor::new(Arc::new(repository)).execute();
    let waited = started.elapsed();
    drop(release);
    writer.join().unwrap();
    assert_eq!(cursor.unwrap(), 0, "an empty board's cursor");
    assert!(waited < Duration::from_millis(250), "{waited:?}");
}

#[test]
fn a_read_refuses_any_write() {
    let (_dir, store, repository) = created();
    let refused =
        repository.read(&mut |transaction| transaction.event("member", 1.0, "noted", &json!({})));
    let refusal = refused.expect_err("a read transaction writes nothing");
    assert_eq!(refusal.kind(), RefusalKind::Store, "{refusal}");
    let events: i64 = Connection::open(store.path())
        .unwrap()
        .query_row("SELECT count(*) FROM events", [], |row| row.get(0))
        .unwrap();
    assert_eq!(events, 0, "nothing was written");
}

#[test]
fn a_read_of_a_missing_board_is_refused_as_missing() {
    let (dir, _store, repository) = created();
    std::fs::remove_file(dir.path().join("swarm.sqlite")).unwrap();
    let refusal = repository
        .read(&mut |transaction| transaction.event_generation().map(|_| ()))
        .expect_err("a lost board is never recreated");
    assert_eq!(refusal.kind(), RefusalKind::StoreMissing, "{refusal}");
    assert!(!dir.path().join("swarm.sqlite").exists());
}

/// A metered read is measured as one transaction that was never busy,
/// even while a writer holds the write lock.
#[test]
fn a_metered_read_is_one_transaction_never_busy_behind_a_writer() {
    let (_dir, store, repository) = created();
    let call = SqliteBoardCallMeter::new(repository).open();
    let (release, writer) = writer_holding(&store);
    let read = call.read(&mut |transaction| transaction.event_generation().map(|_| ()));
    drop(release);
    writer.join().unwrap();
    read.unwrap();
    let measure = call.measure().expect("a transaction began");
    assert_eq!(measure.transactions, 1, "{measure:?}");
    assert!(!measure.busy, "{measure:?}");
}
