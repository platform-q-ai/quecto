//! SPIKE (board service): the project board store's contract, run against
//! the SQLite store, two clients of one database file.
use super::project_board_store::{
    Clients, check_bootstrap, check_bootstrap_race, check_carried_set_aside, check_clear_set_aside,
    check_committed_as_writer, check_compare_and_swap, check_invalid_writes, check_lock_races,
    check_quarantine, check_refused, check_unchanged_write, check_write_after_lost_and_refused,
    check_write_and_read, check_write_before_bootstrap, check_writer,
};
use quecto::domain::project_board::entities::claim::Identity;
use quecto::infrastructure::persistence::board_service::files::{Head, REGULAR};
use quecto::infrastructure::persistence::board_service::sqlite::{
    FileChange, RevisionMeta, SqliteBoard,
};
use quecto::infrastructure::persistence::board_service::store::SqliteBoardStore;
use std::sync::Arc;

pub struct Fixture {
    _dir: tempfile::TempDir,
    pub board: Arc<SqliteBoard>,
    pub alice: SqliteBoardStore,
}

fn identity(name: &str) -> Identity {
    Identity {
        name: name.into(),
        email: format!("{}@example.com", name.to_lowercase()),
    }
}

pub fn fixture() -> (Fixture, Clients) {
    let dir = tempfile::tempdir().unwrap();
    let board = Arc::new(SqliteBoard::open(&dir.path().join("board.sqlite")).unwrap());
    // Bob opens the same file in his own connection, as another process.
    let bob_board = Arc::new(SqliteBoard::open(&dir.path().join("board.sqlite")).unwrap());
    let alice = SqliteBoardStore::new(Arc::clone(&board), identity("Ada"));
    let bob = SqliteBoardStore::new(bob_board, identity("Bob"));
    let clients: Clients = (Box::new(alice.clone()), Box::new(bob));
    (
        Fixture {
            _dir: dir,
            board,
            alice,
        },
        clients,
    )
}

/// Puts `bytes` at `path` as a raw revision, as a hand edit would.
pub fn put_raw(board: &SqliteBoard, path: &str, bytes: &[u8]) {
    let head = board.head().unwrap().expect("a board");
    let next = Head {
        epoch: head.epoch,
        revision: head.revision + 1,
    };
    board
        .append_raw(
            next,
            &[FileChange {
                path: path.into(),
                mode: REGULAR.into(),
                bytes: Some(bytes.to_vec()),
            }],
            &RevisionMeta::by(identity("Mallory"), "hand edit", "raw"),
        )
        .unwrap();
}

#[test]
fn the_sqlite_store_bootstraps_once() {
    check_bootstrap(fixture().1);
}

#[test]
fn the_sqlite_store_counts_no_lock_race() {
    check_lock_races(fixture().1);
}

#[test]
fn the_sqlite_store_writes_and_reads_back() {
    check_write_and_read(fixture().1);
}

#[test]
fn the_sqlite_store_writes_only_at_the_head() {
    check_compare_and_swap(fixture().1);
}

#[test]
fn the_sqlite_store_moves_the_head_on_an_unchanged_write() {
    check_unchanged_write(fixture().1);
}

#[test]
fn the_sqlite_store_refuses_invalid_writes() {
    check_invalid_writes(fixture().1);
}

#[test]
fn the_sqlite_store_refuses_without_moving() {
    let (fixture, clients) = fixture();
    check_refused(clients, || {
        fixture.board.refuse_next_write("a hook said no")
    });
}

#[test]
fn the_sqlite_store_clears_a_file_set_aside() {
    let (fixture, clients) = fixture();
    check_clear_set_aside(clients, || {
        put_raw(&fixture.board, "tasks/b.md", b"not a task\n")
    });
}

#[test]
fn the_sqlite_store_sets_unreadable_files_aside() {
    let (fixture, clients) = fixture();
    check_quarantine(clients, || {
        put_raw(&fixture.board, "tasks/b.md", b"not a task\n")
    });
}

#[test]
fn the_sqlite_store_writes_again_after_a_lost_or_refused_write() {
    let (fixture, clients) = fixture();
    check_write_after_lost_and_refused(clients, || {
        fixture.board.refuse_next_write("a hook said no")
    });
}

#[test]
fn the_sqlite_store_takes_no_write_before_it_is_bootstrapped() {
    check_write_before_bootstrap(fixture().1);
}

#[test]
fn the_sqlite_store_is_bootstrapped_once_by_racing_clients() {
    check_bootstrap_race(fixture().1);
}

#[test]
fn the_sqlite_store_writes_as_a_valid_identity() {
    check_writer(fixture().1);
}

#[test]
fn the_sqlite_store_commits_as_its_writer() {
    let (fixture, clients) = fixture();
    check_committed_as_writer(clients, |head| {
        fixture.alice.committed_by(head).expect("a revision")
    });
}

#[test]
fn the_sqlite_store_carries_a_task_file_set_aside_and_drops_a_stray() {
    let (fixture, clients) = fixture();
    check_carried_set_aside(
        clients,
        || put_raw(&fixture.board, "tasks/b.md", b"not a task\n"),
        || put_raw(&fixture.board, "tasks/notes.txt", b"notes\n"),
        "tasks/notes.txt",
    );
}

/// Two clients, the board to spoil, and what keeps them alive.
pub fn kinds_fixture() -> (Fixture, Arc<SqliteBoard>, Clients) {
    let (fixture, clients) = fixture();
    let board = Arc::clone(&fixture.board);
    (fixture, board, clients)
}
