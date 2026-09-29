//! Two board stores share one board file (#2269, owner decision D2): each
//! one's `BEGIN IMMEDIATE` transaction excludes the other with the same
//! contended message after the same 500 ms busy timeout, and a board the
//! Python board wrote opens in Rust without any change to its schema.
//!
//! Until #2283 the other store was Python's `swarm_store.Store`, run in a
//! `python3` child. With the Python board deleted, the other store is a
//! second Rust store on its own thread, and Python stands in the files it
//! wrote: `schema_after_create.json` (the schema Python's `create` wrote)
//! and `legacy_python_board.sqlite` (a mid-run board the last Python board
//! wrote, S18).
use std::path::Path;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use quecto::infrastructure::persistence::swarm_board::store::{BoardStore, StoreRefusal};
use serial_test::serial;

const SCHEMA_FIXTURE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/swarm_board/schema_after_create.json"
);
const LEGACY_BOARD: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/swarm_board/legacy_python_board.sqlite"
);
const CONTENDED: &str = "coordination store unavailable or contended: database is locked";

type Master = Vec<(String, String, String, Option<String>)>;

fn master(board: &Path) -> Master {
    let connection = rusqlite::Connection::open(board).expect("the board opens");
    let mut statement = connection
        .prepare("SELECT type,name,tbl_name,sql FROM sqlite_master ORDER BY name")
        .expect("sqlite_master is readable");
    statement
        .query_map([], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })
        .expect("sqlite_master rows")
        .collect::<Result<_, _>>()
        .expect("sqlite_master rows read")
}

/// Holds a transaction of a store of its own on `board`, on a thread, until
/// told to let go: the answer is the holder's release and whether its
/// transaction committed.
fn hold(
    board: &Path,
) -> (
    mpsc::Sender<()>,
    std::thread::JoinHandle<Result<(), StoreRefusal>>,
) {
    let (held, taken) = mpsc::channel();
    let (release, released) = mpsc::channel::<()>();
    let path = board.to_path_buf();
    let holder = std::thread::spawn(move || {
        BoardStore::new(&path).transaction(false, |_| {
            held.send(()).expect("the test waits for the hold");
            let _told = released.recv_timeout(Duration::from_secs(60));
            Ok(())
        })
    });
    taken
        .recv_timeout(Duration::from_secs(60))
        .expect("the holder takes the lock");
    (release, holder)
}

/// A store's empty transaction on `board`, timed.
fn try_once(board: &Path) -> (Result<(), StoreRefusal>, Duration) {
    let started = Instant::now();
    let answer = BoardStore::new(board).transaction(false, |_| Ok(()));
    (answer, started.elapsed())
}

#[test]
#[serial]
fn two_stores_transactions_exclude_each_other_on_one_file() {
    let dir = tempfile::tempdir().expect("temp dir");
    let board = dir.path().join("swarm.sqlite");
    BoardStore::new(&board)
        .transaction(true, |_| Ok(()))
        .expect("a store creates the board");

    // Each store in turn holds the write lock; the other waits the busy
    // timeout, then refuses with the contended text.
    for turn in ["first", "second"] {
        let (release, holder) = hold(&board);
        let (refused, waited) = try_once(&board);
        release.send(()).expect("the holder is told to let go");
        holder
            .join()
            .expect("the holder ends")
            .expect("the holder commits");
        assert_eq!(refused, Err(StoreRefusal(CONTENDED.into())), "{turn}");
        assert!(
            waited >= Duration::from_millis(450),
            "{turn}: the store waited the busy timeout: {waited:?}"
        );
        assert!(
            waited < Duration::from_secs(5),
            "{turn}: then gave up: {waited:?}"
        );
    }

    // Released, each takes its turn.
    assert_eq!(try_once(&board).0, Ok(()));
    assert_eq!(try_once(&board).0, Ok(()));
}

/// A store's create writes the schema Python's create wrote.
#[test]
#[serial]
fn a_created_board_holds_the_schema_python_wrote() {
    let dir = tempfile::tempdir().expect("temp dir");
    let board = dir.path().join("swarm.sqlite");
    BoardStore::new(&board)
        .transaction(true, |_| Ok(()))
        .expect("a store creates the board");
    let fixture: Master = serde_json::from_str(
        &std::fs::read_to_string(SCHEMA_FIXTURE).expect("the schema fixture is readable"),
    )
    .expect("the schema fixture is [type, name, tbl_name, sql] rows");
    assert_eq!(master(&board), fixture, "Rust writes Python's schema");
}

/// The board the last Python board wrote opens in Rust with no schema
/// change: its schema holds Python's create schema, a transaction and a
/// create are no-ops to it, and its journal mode is still the default.
#[test]
#[serial]
fn a_python_created_board_opens_in_rust_with_no_schema_change() {
    let dir = tempfile::tempdir().expect("temp dir");
    let board = dir.path().join("swarm.sqlite");
    std::fs::copy(LEGACY_BOARD, &board).expect("copy the legacy board");
    let before = master(&board);
    let fixture: Master = serde_json::from_str(
        &std::fs::read_to_string(SCHEMA_FIXTURE).expect("the schema fixture is readable"),
    )
    .expect("the schema fixture is [type, name, tbl_name, sql] rows");
    for row in &fixture {
        assert!(before.contains(row), "Python's schema holds {row:?}");
    }
    let store = BoardStore::new(&board);
    store
        .transaction(false, |_| Ok(()))
        .expect("Rust opens Python's board");
    store
        .transaction(true, |_| Ok(()))
        .expect("and a Rust create is a no-op");
    assert_eq!(master(&board), before);
    let journal: String = rusqlite::Connection::open(&board)
        .expect("the board opens")
        .query_row("PRAGMA journal_mode", [], |row| row.get(0))
        .expect("journal mode reads");
    assert_eq!(journal, "delete");
}
