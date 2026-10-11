//! Contract for [`ProjectBoardStore`] (#2476): reading, a compare-and-swap
//! write and the bootstrap, as every store must answer them. The checks
//! take two clients of one fresh board, so the git branch store (S1d) runs
//! the same checks over two clones of one remote; here they run over the
//! in-memory store.
use quecto::application::project_board::memory_store::MemoryBoardStore;
use quecto::application::project_board::ports::{
    BoardStoreError, BoardWrite, Bootstrapped, ProjectBoardStore,
};
use quecto::domain::project_board::entities::task::{Task, TaskFields};
use quecto::domain::project_board::value_objects::slug::Slug;
use quecto::domain::project_board::value_objects::timestamp::Timestamp;
use quecto::domain::project_board::value_objects::vocabulary::{TaskKind, TaskStatus};

/// Two clients of one board that is not bootstrapped yet.
pub type Clients = (Box<dyn ProjectBoardStore>, Box<dyn ProjectBoardStore>);

pub fn task(id: &str, title: &str) -> Task {
    let at = Timestamp::parse("2026-10-10T08:00:00Z").unwrap();
    Task::new(TaskFields {
        id: Slug::parse(id).unwrap(),
        title: title.into(),
        kind: TaskKind::Task,
        description: String::new(),
        status: TaskStatus::Ready,
        parent: None,
        depends_on: vec![],
        plan: vec![],
        items: vec![],
        team: None,
        claim: None,
        runs: vec![],
        reviews: vec![],
        prs: vec![],
        created: at.clone(),
        updated: at,
    })
    .unwrap()
}

fn written(
    write: Result<BoardWrite, BoardStoreError>,
) -> quecto::application::project_board::ports::BoardHead {
    match write.unwrap() {
        BoardWrite::Written(head) => head,
        other => panic!("not written: {other:?}"),
    }
}

/// No board is read, written or found before it is bootstrapped; then it
/// is created once, and every client finds it after.
pub fn check_bootstrap((alice, bob): Clients) {
    assert_eq!(alice.read(), Err(BoardStoreError::NoBoard));
    let Ok(Bootstrapped::Created(head)) = alice.bootstrap() else {
        panic!("created")
    };
    assert_eq!(bob.bootstrap(), Ok(Bootstrapped::Existing(head.clone())));
    assert_eq!(alice.bootstrap(), Ok(Bootstrapped::Existing(head.clone())));
    let board = bob.read().unwrap();
    assert_eq!(
        (board.head, board.tasks, board.quarantined),
        (head, vec![], vec![])
    );
}

/// A write at the head moves it, and every client reads what it wrote; a
/// task written again replaces the one of its id whole.
pub fn check_write_and_read((alice, bob): Clients) {
    alice.bootstrap().unwrap();
    let head = alice.read().unwrap().head;
    let moved = written(alice.write(
        &head,
        &[task("a", "First"), task("b", "Second")],
        "board: add a, b",
    ));
    assert_ne!(moved, head);
    let board = bob.read().unwrap();
    assert_eq!(
        (board.head.clone(), board.tasks.clone()),
        (moved.clone(), vec![task("a", "First"), task("b", "Second")])
    );
    written(bob.write(&moved, &[task("a", "Renamed")], "board: rename a"));
    assert_eq!(
        alice.read().unwrap().tasks,
        vec![task("a", "Renamed"), task("b", "Second")]
    );
}

/// A write from a head another client moved past is lost, and the board
/// keeps the winner's change.
pub fn check_compare_and_swap((alice, bob): Clients) {
    alice.bootstrap().unwrap();
    let stale = bob.read().unwrap().head;
    let won = written(alice.write(&stale, &[task("a", "Alice's")], "board: add a"));
    assert_eq!(
        bob.write(&stale, &[task("a", "Bob's")], "board: add a"),
        Ok(BoardWrite::Lost)
    );
    let board = bob.read().unwrap();
    assert_eq!((board.head, board.tasks), (won, vec![task("a", "Alice's")]));
}

fn memory() -> Clients {
    let store = MemoryBoardStore::default();
    (Box::new(store.client()), Box::new(store.client()))
}

#[test]
fn the_memory_store_bootstraps_once() {
    check_bootstrap(memory());
}

#[test]
fn the_memory_store_writes_and_reads_back() {
    check_write_and_read(memory());
}

#[test]
fn the_memory_store_writes_only_at_the_head() {
    check_compare_and_swap(memory());
}
