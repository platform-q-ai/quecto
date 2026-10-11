//! Contract for [`ProjectBoardStore`] (#2476): reading, a compare-and-swap
//! write and the bootstrap, as every store must answer them. The checks
//! take two clients of one fresh board, so the git branch store (S1d) runs
//! the same checks over two clones of one remote; here they run over the
//! in-memory store.
use quecto::application::project_board::memory_store::MemoryBoardStore;
use quecto::application::project_board::ports::{
    BoardBootstrap, BoardHead, BoardStoreError, BoardWrite, ProjectBoardStore,
};
use quecto::domain::project_board::entities::task::{Task, TaskFields};
use quecto::domain::project_board::value_objects::slug::Slug;
use quecto::domain::project_board::value_objects::timestamp::Timestamp;
use quecto::domain::project_board::value_objects::vocabulary::{TaskKind, TaskStatus};
use std::collections::BTreeMap;

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

fn written(write: Result<BoardWrite, BoardStoreError>) -> BoardHead {
    match write.unwrap() {
        BoardWrite::Written(head) => head,
        other => panic!("not written: {other:?}"),
    }
}

/// A board's tasks as a set: the store's own order is not part of the
/// contract (git trees sort `a-b.json` before `a.json`).
fn tasks(store: &dyn ProjectBoardStore) -> BTreeMap<String, Task> {
    let board = store.read().unwrap();
    board
        .tasks
        .into_iter()
        .map(|task| (task.fields().id.as_str().to_owned(), task))
        .collect()
}

fn set(tasks: &[Task]) -> BTreeMap<String, Task> {
    tasks
        .iter()
        .map(|task| (task.fields().id.as_str().to_owned(), task.clone()))
        .collect()
}

/// No board is read, written or found before it is bootstrapped; then it
/// is created once, and every client finds it after.
pub fn check_bootstrap((alice, bob): Clients) {
    assert_eq!(alice.read(), Err(BoardStoreError::NoBoard));
    let Ok(BoardBootstrap::Created(head)) = alice.bootstrap() else {
        panic!("created")
    };
    assert_eq!(bob.bootstrap(), Ok(BoardBootstrap::Existing(head.clone())));
    assert_eq!(
        alice.bootstrap(),
        Ok(BoardBootstrap::Existing(head.clone()))
    );
    let board = bob.read().unwrap();
    assert_eq!(
        (board.head, board.tasks, board.quarantined),
        (head, vec![], vec![])
    );
}

/// A write at the head moves it, and every client reads what it wrote; a
/// task written again replaces the one of its id whole; ids that sort
/// differently as file names (`a`, `a-b`) are each their own task.
pub fn check_write_and_read((alice, bob): Clients) {
    alice.bootstrap().unwrap();
    let head = alice.read().unwrap().head;
    let first = [task("a", "First"), task("a-b", "Second")];
    let moved = written(alice.write(&head, &first, "board: add a, a-b"));
    assert_ne!(moved, head);
    assert_eq!(
        (bob.read().unwrap().head, tasks(&*bob)),
        (moved.clone(), set(&first))
    );
    written(bob.write(&moved, &[task("a", "Renamed")], "board: rename a"));
    assert_eq!(
        tasks(&*alice),
        set(&[task("a", "Renamed"), task("a-b", "Second")])
    );
}

/// A write from a head another client moved past is lost: the board does
/// not hold it, and keeps the winner's change.
pub fn check_compare_and_swap((alice, bob): Clients) {
    alice.bootstrap().unwrap();
    let stale = bob.read().unwrap().head;
    let won = written(alice.write(&stale, &[task("a", "Alice's")], "board: add a"));
    assert_eq!(
        bob.write(
            &stale,
            &[task("a", "Bob's"), task("b", "Bob's")],
            "board: add a, b"
        ),
        Ok(BoardWrite::Lost)
    );
    let board = bob.read().unwrap();
    assert_eq!(
        (board.head, tasks(&*bob)),
        (won, set(&[task("a", "Alice's")]))
    );
}

/// No tasks, one id twice and a message that is not one short line are
/// refused as invalid writes, before anything moves.
pub fn check_invalid_writes((alice, _bob): Clients) {
    alice.bootstrap().unwrap();
    let head = alice.read().unwrap().head;
    for (tasks, message) in [
        (vec![], "board: nothing"),
        (vec![task("a", "One"), task("a", "Two")], "board: twice"),
        (vec![task("a", "One")], "two\nlines"),
        (vec![task("a", "One")], ""),
    ] {
        let refused = alice.write(&head, &tasks, message);
        assert!(
            matches!(refused, Err(BoardStoreError::InvalidWrite(_))),
            "{message:?}: {refused:?}"
        );
    }
    assert_eq!(alice.read().unwrap().head, head);
}

/// A write the store refuses (`refuse` arranges one) leaves the board
/// where it was.
pub fn check_refused((alice, bob): Clients, refuse: impl FnOnce()) {
    alice.bootstrap().unwrap();
    let head = alice.read().unwrap().head;
    refuse();
    assert!(matches!(
        alice.write(&head, &[task("a", "One")], "board: add a"),
        Ok(BoardWrite::Refused(_))
    ));
    assert_eq!(
        (bob.read().unwrap().head, tasks(&*bob)),
        (head, BTreeMap::new())
    );
}

/// A task file the store cannot read (`spoil` breaks `tasks/b.json`) is set
/// aside under its id, never read as a task, and never written over.
pub fn check_quarantine((alice, bob): Clients, spoil: impl FnOnce()) {
    alice.bootstrap().unwrap();
    let head = alice.read().unwrap().head;
    written(alice.write(
        &head,
        &[task("a", "One"), task("b", "Two")],
        "board: add a, b",
    ));
    spoil();
    let board = bob.read().unwrap();
    assert_eq!(tasks(&*bob), set(&[task("a", "One")]));
    let ids: Vec<_> = board
        .quarantined
        .iter()
        .map(|file| file.id.clone())
        .collect();
    assert_eq!(ids, [Some(Slug::parse("b").unwrap())]);
    let refused = bob.write(&board.head, &[task("b", "Mended")], "board: write b");
    assert!(matches!(refused, Ok(BoardWrite::Refused(_))), "{refused:?}");
    assert_eq!(alice.read().unwrap().head, board.head);
}

fn memory() -> (MemoryBoardStore, Clients) {
    let store = MemoryBoardStore::default();
    let clients: Clients = (Box::new(store.client()), Box::new(store.client()));
    (store, clients)
}

#[test]
fn the_memory_store_bootstraps_once() {
    check_bootstrap(memory().1);
}

#[test]
fn the_memory_store_writes_and_reads_back() {
    check_write_and_read(memory().1);
}

#[test]
fn the_memory_store_writes_only_at_the_head() {
    check_compare_and_swap(memory().1);
}

#[test]
fn the_memory_store_refuses_invalid_writes() {
    check_invalid_writes(memory().1);
}

#[test]
fn the_memory_store_refuses_without_moving() {
    let (store, clients) = memory();
    check_refused(clients, || store.refuse_next_write("a hook said no"));
}

#[test]
fn the_memory_store_sets_unreadable_files_aside() {
    let (store, clients) = memory();
    check_quarantine(clients, || store.quarantine("b", "malformed"));
}
