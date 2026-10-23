//! SPIKE (board service): the project board store's contract, run against
//! the service's client adapter: two clients of one board server (over its
//! Unix socket), the server's store the SQLite one.
use super::project_board_sqlite_store::put_raw;
use super::project_board_store::{
    Clients, check_bootstrap, check_bootstrap_race, check_carried_set_aside, check_clear_set_aside,
    check_committed_as_writer, check_compare_and_swap, check_invalid_writes, check_lock_races,
    check_quarantine, check_refused, check_unchanged_write, check_write_after_lost_and_refused,
    check_write_and_read, check_write_before_bootstrap, check_writer,
};
use quecto::application::project_board::ports::BoardHead;
use quecto::domain::project_board::entities::claim::Identity;
use quecto::infrastructure::persistence::board_service::client::ServiceBoardStore;
use quecto::infrastructure::persistence::board_service::files::Head;
use quecto::infrastructure::persistence::board_service::http;
use quecto::infrastructure::persistence::board_service::server::{Role, Server};
use quecto::infrastructure::persistence::board_service::sqlite::SqliteBoard;
use std::sync::Arc;

pub struct Fixture {
    _dir: tempfile::TempDir,
    board: Arc<SqliteBoard>,
}

fn identity(name: &str) -> Identity {
    Identity {
        name: name.into(),
        email: format!("{}@example.com", name.to_lowercase()),
    }
}

fn fixture() -> (Fixture, Clients) {
    let dir = tempfile::tempdir().unwrap();
    let board = Arc::new(SqliteBoard::open(&dir.path().join("board.sqlite")).unwrap());
    let server = Server::new(Arc::clone(&board), Role::Primary);
    let socket = dir.path().join("board.sock");
    let listener = std::os::unix::net::UnixListener::bind(&socket).unwrap();
    let handler = server.handler();
    std::thread::spawn(move || http::serve_unix(listener, handler));
    let spec = format!("unix:{}", socket.display());
    let alice = ServiceBoardStore::new(&spec, identity("Ada")).unwrap();
    let bob = ServiceBoardStore::new(&spec, identity("Bob")).unwrap();
    let clients: Clients = (Box::new(alice), Box::new(bob));
    (Fixture { _dir: dir, board }, clients)
}

fn committed_by(board: &SqliteBoard, head: &BoardHead) -> Identity {
    let head = Head::parse(&head.0).expect("a head");
    board.writer_of(head.revision).unwrap().expect("a revision")
}

#[test]
fn the_service_bootstraps_once() {
    check_bootstrap(fixture().1);
}

#[test]
fn the_service_counts_no_lock_race() {
    check_lock_races(fixture().1);
}

#[test]
fn the_service_writes_and_reads_back() {
    check_write_and_read(fixture().1);
}

#[test]
fn the_service_writes_only_at_the_head() {
    check_compare_and_swap(fixture().1);
}

#[test]
fn the_service_moves_the_head_on_an_unchanged_write() {
    check_unchanged_write(fixture().1);
}

#[test]
fn the_service_refuses_invalid_writes() {
    check_invalid_writes(fixture().1);
}

#[test]
fn the_service_refuses_without_moving() {
    let (fixture, clients) = fixture();
    check_refused(clients, || {
        fixture.board.refuse_next_write("a hook said no")
    });
}

#[test]
fn the_service_clears_a_file_set_aside() {
    let (fixture, clients) = fixture();
    check_clear_set_aside(clients, || {
        put_raw(&fixture.board, "tasks/b.md", b"not a task\n")
    });
}

#[test]
fn the_service_sets_unreadable_files_aside() {
    let (fixture, clients) = fixture();
    check_quarantine(clients, || {
        put_raw(&fixture.board, "tasks/b.md", b"not a task\n")
    });
}

#[test]
fn the_service_writes_again_after_a_lost_or_refused_write() {
    let (fixture, clients) = fixture();
    check_write_after_lost_and_refused(clients, || {
        fixture.board.refuse_next_write("a hook said no")
    });
}

#[test]
fn the_service_takes_no_write_before_it_is_bootstrapped() {
    check_write_before_bootstrap(fixture().1);
}

#[test]
fn the_service_is_bootstrapped_once_by_racing_clients() {
    check_bootstrap_race(fixture().1);
}

#[test]
fn the_service_writes_as_a_valid_identity() {
    check_writer(fixture().1);
}

#[test]
fn the_service_commits_as_its_writer() {
    let (fixture, clients) = fixture();
    check_committed_as_writer(clients, |head| committed_by(&fixture.board, head));
}

#[test]
fn the_service_carries_a_task_file_set_aside_and_drops_a_stray() {
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
