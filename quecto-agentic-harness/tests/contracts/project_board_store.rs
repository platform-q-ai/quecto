//! Contract for [`ProjectBoardStore`] (#2476): reading, a compare-and-swap
//! write and the bootstrap, as every store must answer them. The checks
//! take two clients of one fresh board, so the git branch store (S1d) runs
//! the same checks over two clones of one remote; here they run over the
//! in-memory store.
use quecto::application::project_board::ports::{
    BoardBootstrap, BoardHead, BoardStoreError, BoardWrite, KeepCopy, ProjectBoardStore,
    SetAsideClear,
};
use quecto::domain::project_board::entities::claim::Identity;
use quecto::domain::project_board::entities::task::{Task, TaskFields};
use quecto::domain::project_board::value_objects::slug::Slug;
use quecto::domain::project_board::value_objects::timestamp::Timestamp;
use quecto::domain::project_board::value_objects::vocabulary::{TaskKind, TaskStatus};
use quecto::infrastructure::test_support::memory_board_store::MemoryBoardStore;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Barrier;

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
        accepted_plan: None,
        claim: None,
        prs: vec![],
        github_issue: None,
        resolution: None,
        superseded_by: vec![],
        references: vec![],
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
/// contract (git trees sort `a-b.md` before `a.md`).
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
    let moved = written(alice.write_tasks(&head, &first, "board: add a, a-b"));
    assert_ne!(moved, head);
    assert_eq!(
        (bob.read().unwrap().head, tasks(&*bob)),
        (moved.clone(), set(&first))
    );
    written(bob.write_tasks(&moved, &[task("a", "Renamed")], "board: rename a"));
    assert_eq!(
        tasks(&*alice),
        set(&[task("a", "Renamed"), task("a-b", "Second")])
    );
}

/// A write of tasks identical to the board's is still a change: it
/// answers `Written` and moves the head (a git commit always has a new
/// id), so a writer can always tell its write landed.
pub fn check_unchanged_write((alice, bob): Clients) {
    alice.bootstrap().unwrap();
    let head = alice.read().unwrap().head;
    let first = written(alice.write_tasks(&head, &[task("a", "One")], "board: add a"));
    let again = written(bob.write_tasks(&first, &[task("a", "One")], "board: add a again"));
    assert_ne!(again, first, "an unchanged write moves the head");
    assert_eq!(
        (alice.read().unwrap().head, tasks(&*alice)),
        (again, set(&[task("a", "One")]))
    );
}

/// A write from a head another client moved past is lost: the board does
/// not hold it, and keeps the winner's change.
pub fn check_compare_and_swap((alice, bob): Clients) {
    alice.bootstrap().unwrap();
    let stale = bob.read().unwrap().head;
    let won = written(alice.write_tasks(&stale, &[task("a", "Alice's")], "board: add a"));
    assert_eq!(
        bob.write_tasks(
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
        let refused = alice.write_tasks(&head, &tasks, message);
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
        alice.write_tasks(&head, &[task("a", "One")], "board: add a"),
        Ok(BoardWrite::Refused(_))
    ));
    assert_eq!(
        (bob.read().unwrap().head, tasks(&*bob)),
        (head, BTreeMap::new())
    );
}

/// A task file the store cannot read (`spoil` breaks `tasks/b.md`, as
/// a commit to the board does) is set aside under its id, never read as a
/// task, and never written over. Spoiling the file moves the head, so a
/// write from the head before it is lost.
pub fn check_quarantine((alice, bob): Clients, spoil: impl FnOnce()) {
    alice.bootstrap().unwrap();
    let head = alice.read().unwrap().head;
    let before = written(alice.write_tasks(
        &head,
        &[task("a", "One"), task("b", "Two")],
        "board: add a, b",
    ));
    spoil();
    let board = bob.read().unwrap();
    assert_ne!(board.head, before, "spoiling a file moves the head");
    assert_eq!(tasks(&*bob), set(&[task("a", "One")]));
    let ids: Vec<_> = board
        .quarantined
        .iter()
        .map(|file| file.id().cloned())
        .collect();
    assert_eq!(ids, [Some(Slug::parse("b").unwrap())]);
    assert_eq!(
        alice.write_tasks(&before, &[task("a", "Stale")], "board: rename a"),
        Ok(BoardWrite::Lost),
        "a write from before the file was spoiled"
    );
    let refused = bob.write_tasks(&board.head, &[task("b", "Mended")], "board: write b");
    assert!(matches!(refused, Ok(BoardWrite::Refused(_))), "{refused:?}");
    assert_eq!(alice.read().unwrap().head, board.head);
}

/// A file set aside (`spoil` breaks `tasks/b.md`) is cleared in one
/// change, only from the head that holds it: its id is free again, and the
/// rest of the board stands. A path that is not set aside, or a copy to
/// keep for a name listed once, is refused.
pub fn check_clear_set_aside((alice, bob): Clients, spoil: impl FnOnce()) {
    let clearing = |path: &str, object: &str, repeated: &[(&str, KeepCopy)]| SetAsideClear {
        path: path.into(),
        object: object.into(),
        repeated: repeated
            .iter()
            .map(|(path, keep)| ((*path).to_string(), keep.clone()))
            .collect(),
    };
    alice.bootstrap().unwrap();
    let head = alice.read().unwrap().head;
    let before = written(alice.write_tasks(
        &head,
        &[task("a", "One"), task("b", "Two")],
        "board: add a, b",
    ));
    spoil();
    let board = bob.read().unwrap();
    let (path, object) = (
        board.quarantined[0].path.clone(),
        board.quarantined[0].object.clone(),
    );
    assert_eq!(path, "tasks/b.md");
    let zeros = "0".repeat(object.len());
    let seen = BTreeSet::from([object.clone()]);
    for clear in [
        clearing("tasks/a.md", &object, &[]),
        clearing(&path, &zeros, &[]),
        clearing(
            &path,
            &object,
            &[(path.as_str(), KeepCopy::Drop(seen.clone()))],
        ),
    ] {
        let refused = bob.clear_set_aside(&board.head, &clear, "board: clear");
        assert!(
            matches!(&refused, Ok(BoardWrite::Refused(why)) if !why.is_empty()),
            "{clear:?}: {refused:?}"
        );
    }
    assert_eq!(
        alice.clear_set_aside(&before, &clearing(&path, &object, &[]), "board: clear b"),
        Ok(BoardWrite::Lost),
        "a clear from before the file was spoiled"
    );
    let invalid = bob.clear_set_aside(&board.head, &clearing(&path, &object, &[]), "two\nlines");
    assert!(
        matches!(invalid, Err(BoardStoreError::InvalidWrite(_))),
        "{invalid:?}"
    );
    let cleared = written(bob.clear_set_aside(
        &board.head,
        &clearing(&path, &object, &[]),
        "board: clear b",
    ));
    let after = alice.read().unwrap();
    assert_eq!(
        (after.head.clone(), after.quarantined, tasks(&*alice)),
        (cleared.clone(), vec![], set(&[task("a", "One")]))
    );
    written(alice.write_tasks(&cleared, &[task("b", "Mended")], "board: write b"));
    assert_eq!(tasks(&*bob), set(&[task("a", "One"), task("b", "Mended")]));
}

/// A client whose write was lost, or refused, writes again from a fresh
/// read: no stale local state holds it back.
pub fn check_write_after_lost_and_refused((alice, bob): Clients, refuse: impl FnOnce()) {
    alice.bootstrap().unwrap();
    let stale = bob.read().unwrap().head;
    written(alice.write_tasks(&stale, &[task("a", "Alice's")], "board: add a"));
    assert_eq!(
        bob.write_tasks(&stale, &[task("b", "Bob's")], "board: add b"),
        Ok(BoardWrite::Lost)
    );
    let fresh = bob.read().unwrap().head;
    let after_lost = written(bob.write_tasks(&fresh, &[task("b", "Bob's")], "board: add b"));
    refuse();
    assert!(matches!(
        bob.write_tasks(&after_lost, &[task("c", "Bob's")], "board: add c"),
        Ok(BoardWrite::Refused(_))
    ));
    let head = bob.read().unwrap().head;
    assert_eq!(head, after_lost, "a refusal leaves the board where it was");
    written(bob.write_tasks(&head, &[task("c", "Bob's")], "board: add c"));
    assert_eq!(
        tasks(&*alice),
        set(&[task("a", "Alice's"), task("b", "Bob's"), task("c", "Bob's")])
    );
}

/// A write to a board that is not bootstrapped answers `NoBoard`, and
/// leaves it unbootstrapped.
pub fn check_write_before_bootstrap((alice, bob): Clients) {
    let anywhere = BoardHead("0".repeat(40));
    assert_eq!(
        alice.write_tasks(&anywhere, &[task("a", "One")], "board: add a"),
        Err(BoardStoreError::NoBoard)
    );
    assert_eq!(bob.read(), Err(BoardStoreError::NoBoard));
}

/// Two clients bootstrapping at once: one creates the board, the other
/// finds it, and both name the same head.
pub fn check_bootstrap_race((alice, bob): Clients) {
    let start = Barrier::new(2);
    let (left, right) = std::thread::scope(|scope| {
        let left = scope.spawn(|| {
            start.wait();
            alice.bootstrap()
        });
        let right = scope.spawn(|| {
            start.wait();
            bob.bootstrap()
        });
        (left.join().unwrap(), right.join().unwrap())
    });
    match (left.unwrap(), right.unwrap()) {
        (BoardBootstrap::Created(made), BoardBootstrap::Existing(found))
        | (BoardBootstrap::Existing(found), BoardBootstrap::Created(made)) => {
            assert_eq!(made, found);
        }
        other => panic!("one creates, the other finds: {other:?}"),
    }
}

/// Each client writes as a valid committer identity.
pub fn check_writer((alice, bob): Clients) {
    for client in [&alice, &bob] {
        let writer = client.writer();
        assert_eq!(writer.validate("writer"), Ok(()), "{writer:?}");
    }
}

/// Every write is committed as its client's `writer()`: `committed_by`
/// reads the committer of the board at a head.
pub fn check_committed_as_writer(
    (alice, bob): Clients,
    committed_by: impl Fn(&BoardHead) -> Identity,
) {
    alice.bootstrap().unwrap();
    let head = alice.read().unwrap().head;
    let by_alice = written(alice.write_tasks(&head, &[task("a", "One")], "board: add a"));
    assert_eq!(committed_by(&by_alice), alice.writer());
    let by_bob = written(bob.write_tasks(&by_alice, &[task("b", "Two")], "board: add b"));
    assert_eq!(committed_by(&by_bob), bob.writer());
}

/// A task file set aside (`spoil` breaks `tasks/b.md`) is carried
/// forward, a stray beside it (`add_stray`) is not: the store says which,
/// a link waits on the carried one (its pull requests unknown), the next
/// write drops the stray and keeps the carried file, and a clear (a write
/// too) drops a stray beside the file it clears.
pub fn check_carried_set_aside(
    (alice, _bob): Clients,
    spoil: impl FnOnce(),
    add_stray: impl Fn(),
    stray: &str,
) {
    use quecto::application::project_board::dto::{BoardOp, ProjectBoardError};
    use quecto::application::project_board::ports::ArtifactKey;
    use quecto::application::project_board::use_cases::ChangeBoardTask;
    use quecto::infrastructure::time::board_clock::SystemBoardClock;
    use std::sync::Arc;
    alice.bootstrap().unwrap();
    let head = alice.read().unwrap().head;
    written(alice.write_tasks(
        &head,
        &[task("a", "One"), task("b", "Two")],
        "board: add a, b",
    ));
    spoil();
    add_stray();
    let carried = |store: &dyn ProjectBoardStore| -> Vec<(String, bool)> {
        let board = store.read().unwrap();
        board
            .quarantined
            .into_iter()
            .map(|file| (file.path, file.carried))
            .collect()
    };
    let both = [("tasks/b.md".to_string(), true), (stray.to_string(), false)];
    assert_eq!(carried(&*alice), both);
    let alice: Arc<dyn ProjectBoardStore> = Arc::from(alice);
    let change =
        ChangeBoardTask::new(alice.clone(), Arc::new(SystemBoardClock::default())).unwrap();
    assert_eq!(
        change.execute(&Slug::parse("a").unwrap(), &BoardOp::AddPr(7)),
        Err(ProjectBoardError::Quarantined {
            key: ArtifactKey::Task(Slug::parse("b").unwrap()),
            path: "tasks/b.md".into(),
        })
    );
    let head = alice.read().unwrap().head;
    written(alice.write_tasks(&head, &[task("c", "Three")], "board: add c"));
    assert_eq!(carried(&*alice), [("tasks/b.md".to_string(), true)]);
    add_stray();
    assert_eq!(carried(&*alice), both);
    let board = alice.read().unwrap();
    let object = board.quarantined[0].object.clone();
    let clear = SetAsideClear {
        path: "tasks/b.md".into(),
        object,
        repeated: BTreeMap::new(),
    };
    written(alice.clear_set_aside(&board.head, &clear, "board: clear b"));
    assert_eq!(carried(&*alice), [], "the clear drops the stray beside b");
}

/// Two clients of one fresh in-memory board, writing as Ada and Bob.
fn memory() -> (MemoryBoardStore, Clients) {
    let store = MemoryBoardStore::default();
    let bob = Identity {
        name: "Bob".into(),
        email: "bob@example.com".into(),
    };
    let clients: Clients = (Box::new(store.client()), Box::new(store.client_as(bob)));
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
fn the_memory_store_moves_the_head_on_an_unchanged_write() {
    check_unchanged_write(memory().1);
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
fn the_memory_store_clears_a_file_set_aside() {
    let (store, clients) = memory();
    check_clear_set_aside(clients, || store.quarantine("b", "malformed"));
}

#[test]
fn the_memory_store_sets_unreadable_files_aside() {
    let (store, clients) = memory();
    check_quarantine(clients, || store.quarantine("b", "malformed"));
}

#[test]
fn the_memory_store_writes_again_after_a_lost_or_refused_write() {
    let (store, clients) = memory();
    check_write_after_lost_and_refused(clients, || store.refuse_next_write("a hook said no"));
}

#[test]
fn the_memory_store_takes_no_write_before_it_is_bootstrapped() {
    check_write_before_bootstrap(memory().1);
}

#[test]
fn the_memory_store_is_bootstrapped_once_by_racing_clients() {
    check_bootstrap_race(memory().1);
}

#[test]
fn the_memory_store_writes_as_a_valid_identity() {
    check_writer(memory().1);
}

#[test]
fn the_memory_store_commits_as_its_writer() {
    let (store, clients) = memory();
    check_committed_as_writer(clients, |head| {
        store.committed_by(head).expect("a commit at this head")
    });
}

#[test]
fn the_memory_store_carries_a_task_file_set_aside_and_drops_a_stray() {
    let (store, clients) = memory();
    check_carried_set_aside(
        clients,
        || store.quarantine("b", "malformed"),
        || store.set_aside_stray("tasks/notes.txt", "not a task file"),
        "tasks/notes.txt",
    );
}
