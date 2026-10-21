use super::CreateBoardTask;
use crate::application::project_board::dto::{
    IdUser, NewBoardTask, NewBoardTaskStatus, ProjectBoardError,
};
use crate::application::project_board::ports::{
    ArtifactKey, BoardArtifact, BoardStoreError, ProjectBoardStore,
};
use crate::application::project_board::use_cases::MAX_WRITE_ATTEMPTS;
use crate::application::project_board::use_cases::fixtures_tests::{
    SetClock, at, board, fields, new_task, set_aside_task, slug,
};
use crate::domain::project_board::entities::discussion::{Discussion, DiscussionFields};
use crate::domain::project_board::entities::task::{Task, TaskFields};
use crate::domain::project_board::value_objects::vocabulary::TaskStatus;
use crate::infrastructure::test_support::board_artifacts;
use crate::infrastructure::test_support::memory_board_store::MemoryBoardStore;
use std::sync::Arc;

fn create(store: &MemoryBoardStore, clock: Arc<SetClock>) -> CreateBoardTask {
    CreateBoardTask::new(Arc::new(store.client()), clock).unwrap()
}

/// Another writer's task, written straight to the store.
fn other_writes(board: &MemoryBoardStore, id: &str) {
    let head = board.read().unwrap().head;
    let task = Task::new(TaskFields {
        created: at("09:00:00"),
        updated: at("09:00:00"),
        ..fields(id)
    })
    .unwrap();
    board.write_tasks(&head, &[task], "other").unwrap();
}

#[test]
fn a_task_is_created_and_stamped_now() {
    let store = board();
    let task = create(&store, SetClock::at("10:00:00"))
        .execute(&new_task("a"))
        .unwrap();
    assert_eq!(
        (&task.fields().created, &task.fields().updated),
        (&at("10:00:00"), &at("10:00:00"))
    );
    assert_eq!(store.read().unwrap().tasks, vec![task]);
}

/// A new task holds only its own fields: it is created a draft or ready,
/// with no plan, team, claim, runs, reviews, pull requests or items,
/// which `NewBoardTask` cannot carry (a plan is its own artifact).
#[test]
fn a_new_task_holds_only_its_own_fields() {
    let store = board();
    let draft = create(&store, SetClock::at("10:00:00"))
        .execute(&NewBoardTask {
            status: NewBoardTaskStatus::Draft,
            description: "Why and how.".into(),
            ..new_task("a")
        })
        .unwrap();
    assert_eq!(
        draft.into_fields(),
        TaskFields {
            status: TaskStatus::Draft,
            description: "Why and how.".into(),
            created: at("10:00:00"),
            updated: at("10:00:00"),
            ..fields("a")
        }
    );
}

/// A parent or dependency must already be on the board, so no two tasks
/// can ever be created depending on each other.
#[test]
fn a_parent_or_dependency_not_on_the_board_is_refused() {
    let store = board();
    let a_on_b = NewBoardTask {
        depends_on: vec![slug("b")],
        ..new_task("a")
    };
    let refused = create(&store, SetClock::at("10:00:00")).execute(&a_on_b);
    assert_eq!(refused, Err(ProjectBoardError::UnknownReference(slug("b"))));
    let orphan = create(&store, SetClock::at("10:00:00")).execute(&NewBoardTask {
        parent: Some(slug("p")),
        ..new_task("a")
    });
    assert_eq!(orphan, Err(ProjectBoardError::UnknownReference(slug("p"))));
    create(&store, SetClock::at("10:00:00"))
        .execute(&new_task("b"))
        .unwrap();
    create(&store, SetClock::at("10:01:00"))
        .execute(&a_on_b)
        .unwrap();
}

#[test]
fn an_id_held_or_set_aside_is_refused() {
    let store = board();
    create(&store, SetClock::at("10:00:00"))
        .execute(&new_task("a"))
        .unwrap();
    let again = create(&store, SetClock::at("10:01:00")).execute(&new_task("a"));
    assert_eq!(again, Err(ProjectBoardError::Exists(slug("a"))));
    store.quarantine("q", "malformed");
    let over = create(&store, SetClock::at("10:01:00")).execute(&new_task("q"));
    assert_eq!(over, Err(set_aside_task("q")));
}

/// Another writer moves the board between the read and the write: the
/// create is built again against the fresh board and lands.
#[test]
fn a_lost_write_is_retried_against_the_fresh_board() {
    let store = board();
    store.before_next_write(|board| other_writes(board, "b"));
    create(&store, SetClock::at("10:00:00"))
        .execute(&new_task("a"))
        .unwrap();
    let ids: Vec<_> = store
        .read()
        .unwrap()
        .tasks
        .iter()
        .map(|task| task.fields().id.as_str().to_owned())
        .collect();
    assert_eq!(ids, ["a", "b"]);
}

#[test]
fn a_retry_that_finds_the_id_taken_is_refused() {
    let store = board();
    store.before_next_write(|board| other_writes(board, "a"));
    let refused = create(&store, SetClock::at("10:00:00")).execute(&new_task("a"));
    assert_eq!(refused, Err(ProjectBoardError::Exists(slug("a"))));
}

/// The other writer moves the board before `interferences` attempts; each
/// of them loses, and a later one lands while attempts remain.
fn interfered(
    interferences: usize,
) -> (
    Result<Task, ProjectBoardError>,
    MemoryBoardStore,
    Vec<usize>,
) {
    fn move_it(board: &MemoryBoardStore, left: usize) {
        other_writes(board, &format!("other-{left}"));
        if left > 1 {
            board.before_next_write(move |board| move_it(board, left - 1));
        }
    }
    let store = board();
    store.before_next_write(move |board| move_it(board, interferences));
    let clock = SetClock::at("10:00:00");
    let created = create(&store, clock.clone()).execute(&new_task("a"));
    let pauses = clock.pauses.lock().unwrap().clone();
    (created, store, pauses)
}

#[test]
fn a_board_moved_before_every_attempt_is_reported_contended() {
    let (created, store, pauses) = interfered(MAX_WRITE_ATTEMPTS);
    assert_eq!(
        created,
        Err(ProjectBoardError::Contended {
            attempts: MAX_WRITE_ATTEMPTS
        })
    );
    assert_eq!(
        store.writes(),
        MAX_WRITE_ATTEMPTS,
        "only the other writer's writes landed"
    );
    assert_eq!(
        pauses,
        (2..=MAX_WRITE_ATTEMPTS).collect::<Vec<_>>(),
        "a pause before each retry"
    );
}

#[test]
fn a_board_moved_before_all_but_the_last_attempt_takes_the_write() {
    let (created, store, _) = interfered(MAX_WRITE_ATTEMPTS - 1);
    assert!(created.is_ok(), "{created:?}");
    assert_eq!(store.writes(), MAX_WRITE_ATTEMPTS);
}

#[test]
fn a_clock_too_far_ahead_of_its_reference_is_refused() {
    let store = board();
    let ahead = create(
        &store,
        SetClock::with_reference("10:01:01", Some("10:00:00")),
    )
    .execute(&new_task("a"));
    assert!(
        matches!(ahead, Err(ProjectBoardError::ClockAhead { .. })),
        "{ahead:?}"
    );
    let close = create(
        &store,
        SetClock::with_reference("10:01:00", Some("10:00:00")),
    )
    .execute(&new_task("a"));
    assert!(close.is_ok(), "60 s ahead is allowed");
}

#[test]
fn a_board_not_bootstrapped_is_reported() {
    let store = MemoryBoardStore::default();
    let refused = create(&store, SetClock::at("10:00:00")).execute(&new_task("a"));
    assert_eq!(
        refused,
        Err(ProjectBoardError::Store(BoardStoreError::NoBoard))
    );
}

/// A parent or dependency whose file is set aside is reported as set
/// aside, not as unknown.
#[test]
fn a_quarantined_parent_or_dependency_is_reported_as_quarantined() {
    let store = board();
    for id in ["p", "d"] {
        create(&store, SetClock::at("10:00:00"))
            .execute(&new_task(id))
            .unwrap();
        store.quarantine(id, "malformed");
    }
    let child = create(&store, SetClock::at("10:01:00")).execute(&NewBoardTask {
        parent: Some(slug("p")),
        ..new_task("a")
    });
    assert_eq!(child, Err(set_aside_task("p")));
    let dependent = create(&store, SetClock::at("10:01:00")).execute(&NewBoardTask {
        depends_on: vec![slug("d")],
        ..new_task("a")
    });
    assert_eq!(dependent, Err(set_aside_task("d")));
}

/// The parent's file is spoiled between the read and the write: the
/// write is lost, and the create, made again, finds the parent set aside.
#[test]
fn a_parent_quarantined_between_the_read_and_the_write_is_reported() {
    let store = board();
    create(&store, SetClock::at("10:00:00"))
        .execute(&new_task("p"))
        .unwrap();
    store.before_next_write(|board| board.quarantine("p", "malformed"));
    let clock = SetClock::at("10:01:00");
    let child = create(&store, clock.clone()).execute(&NewBoardTask {
        parent: Some(slug("p")),
        ..new_task("a")
    });
    assert_eq!(child, Err(set_aside_task("p")));
    assert_eq!(*clock.pauses.lock().unwrap(), [2], "made again once");
    assert_eq!(store.writes(), 1, "only p was written");
}

/// b's file was quarantined and then deleted, so a still depends on, and
/// c is still a child of, a b the board no longer holds.
fn board_missing_b() -> MemoryBoardStore {
    let store = board();
    create(&store, SetClock::at("10:00:00"))
        .execute(&new_task("b"))
        .unwrap();
    create(&store, SetClock::at("10:01:00"))
        .execute(&NewBoardTask {
            depends_on: vec![slug("b")],
            ..new_task("a")
        })
        .unwrap();
    create(&store, SetClock::at("10:02:00"))
        .execute(&NewBoardTask {
            parent: Some(slug("b")),
            ..new_task("c")
        })
        .unwrap();
    store.quarantine("b", "malformed");
    store.delete_quarantined("b");
    store
}

/// A deleted task is made again under its id when that closes no cycle.
#[test]
fn a_deleted_task_is_made_again_when_that_closes_no_cycle() {
    let store = board_missing_b();
    let again = create(&store, SetClock::at("10:03:00")).execute(&new_task("b"));
    assert!(again.is_ok(), "{again:?}");
}

/// Tasks written straight to the board, as a store holding older or
/// hand-made tasks does.
fn plant(store: &MemoryBoardStore, tasks: Vec<TaskFields>) {
    let tasks: Vec<Task> = tasks
        .into_iter()
        .map(|fields| {
            Task::new(TaskFields {
                created: at("09:00:00"),
                updated: at("09:00:00"),
                ..fields
            })
            .unwrap()
        })
        .collect();
    let head = store.read().unwrap().head;
    store.write_tasks(&head, &tasks, "plant").unwrap();
}

fn closes(by: &str) -> Result<Task, ProjectBoardError> {
    Err(ProjectBoardError::ClosesCycle {
        id: slug("b"),
        by: slug(by),
    })
}

/// A new b depending on a, which depends on b, closes a dependency cycle;
/// so does b depending on e, which depends on a.
#[test]
fn a_new_task_that_would_close_a_dependency_cycle_is_refused() {
    let store = board_missing_b();
    create(&store, SetClock::at("10:03:00"))
        .execute(&NewBoardTask {
            depends_on: vec![slug("a")],
            ..new_task("e")
        })
        .unwrap();
    let writes = store.writes();
    let create_b = |depends_on: &[&str]| {
        create(&store, SetClock::at("10:04:00")).execute(&NewBoardTask {
            depends_on: depends_on.iter().map(|id| slug(id)).collect(),
            ..new_task("b")
        })
    };
    assert_eq!(create_b(&["a"]), closes("a"));
    assert_eq!(create_b(&["e"]), closes("a"));
    assert_eq!(store.writes(), writes);
}

/// A new b under c, whose parent is b, closes a parent cycle; so does b
/// under g, a child of c.
#[test]
fn a_new_task_that_would_close_a_parent_cycle_is_refused() {
    let store = board_missing_b();
    create(&store, SetClock::at("10:03:00"))
        .execute(&NewBoardTask {
            parent: Some(slug("c")),
            ..new_task("g")
        })
        .unwrap();
    let create_b = |parent: &str| {
        create(&store, SetClock::at("10:04:00")).execute(&NewBoardTask {
            parent: Some(slug(parent)),
            ..new_task("b")
        })
    };
    assert_eq!(create_b("c"), closes("c"));
    assert_eq!(create_b("g"), closes("c"));
}

/// Parents and dependencies are separate relations: b depending on c,
/// whose parent is b, is no cycle of either, and is made.
#[test]
fn a_dependency_on_a_task_naming_it_as_parent_closes_no_cycle() {
    let store = board_missing_b();
    let made = create(&store, SetClock::at("10:04:00")).execute(&NewBoardTask {
        depends_on: vec![slug("c")],
        ..new_task("b")
    });
    assert!(made.is_ok(), "{made:?}");
}

/// Parents and dependencies are separate relations: b under e, where e
/// depends on a and a depends on b, is no cycle of either, and is made.
#[test]
fn a_parent_then_a_dependency_chain_closes_no_cycle() {
    let store = board_missing_b();
    create(&store, SetClock::at("10:03:00"))
        .execute(&NewBoardTask {
            depends_on: vec![slug("a")],
            ..new_task("e")
        })
        .unwrap();
    let made = create(&store, SetClock::at("10:04:00")).execute(&NewBoardTask {
        parent: Some(slug("e")),
        ..new_task("b")
    });
    assert!(made.is_ok(), "{made:?}");
}

/// b depending on d, which leads nowhere near b, is made.
#[test]
fn an_unrelated_dependency_closes_no_cycle() {
    let store = board_missing_b();
    create(&store, SetClock::at("10:03:00"))
        .execute(&new_task("d"))
        .unwrap();
    let made = create(&store, SetClock::at("10:04:00")).execute(&NewBoardTask {
        depends_on: vec![slug("d")],
        ..new_task("b")
    });
    assert!(made.is_ok(), "{made:?}");
}

/// Two tasks would close the cycle (f and a both depend on b): the least
/// id is named, whatever order they are reached in.
#[test]
fn the_least_task_closing_the_cycle_is_named() {
    let store = board_missing_b();
    plant(
        &store,
        vec![TaskFields {
            depends_on: vec![slug("b")],
            ..fields("f")
        }],
    );
    for depends_on in [vec![slug("f"), slug("a")], vec![slug("a"), slug("f")]] {
        let refused = create(&store, SetClock::at("10:04:00")).execute(&NewBoardTask {
            depends_on: depends_on.clone(),
            ..new_task("b")
        });
        assert_eq!(refused, closes("a"), "{depends_on:?}");
    }
}

/// x and y depend on each other (planted by hand) and their parents form
/// a loop too: the walks pass through them, end, and find no cycle
/// through b.
#[test]
fn a_cycle_already_on_the_board_is_walked_through() {
    let store = board_missing_b();
    plant(
        &store,
        vec![
            TaskFields {
                parent: Some(slug("y")),
                depends_on: vec![slug("y")],
                ..fields("x")
            },
            TaskFields {
                parent: Some(slug("x")),
                depends_on: vec![slug("x")],
                ..fields("y")
            },
        ],
    );
    let made = create(&store, SetClock::at("10:04:00")).execute(&NewBoardTask {
        parent: Some(slug("x")),
        depends_on: vec![slug("x")],
        ..new_task("b")
    });
    assert!(made.is_ok(), "{made:?}");
}

#[test]
fn board_text_is_normalised_before_the_schema_reads_it() {
    let store = board();
    let created = create(&store, SetClock::at("10:00:00"))
        .execute(&NewBoardTask {
            title: "\tShip it ".into(),
            description: "Why. \r\nHow so.\r\n".into(),
            ..new_task("a")
        })
        .unwrap();
    assert_eq!(created.fields().title, "Ship it");
    assert_eq!(created.fields().description, "Why.\nHow so.");
}

/// An id the board's other artifacts already name (a plan, run, change or
/// review of a task by that id, or a discussion about it) is refused,
/// naming the least of them: a new task would take over what they say.
/// Each kind alone names it.
#[test]
fn an_id_other_artifacts_name_is_refused() {
    let about_x = Discussion::new(DiscussionFields {
        task: Some(slug("x")),
        ..board_artifacts::discussion("chat").fields().clone()
    })
    .unwrap();
    for (artifacts, least) in [
        (
            vec![BoardArtifact::Plan(board_artifacts::plan("x", 5))],
            IdUser::Plan(5),
        ),
        (
            vec![BoardArtifact::Run(board_artifacts::run("x", 1))],
            IdUser::Run(1),
        ),
        (
            vec![BoardArtifact::Change(board_artifacts::change("x", 4))],
            IdUser::Change(4),
        ),
        (
            vec![BoardArtifact::Review(board_artifacts::review("x", 3))],
            IdUser::Review(3),
        ),
        (
            vec![BoardArtifact::Discussion(about_x)],
            IdUser::Discussion(slug("chat")),
        ),
        (
            vec![
                BoardArtifact::Review(board_artifacts::review("x", 1)),
                BoardArtifact::Plan(board_artifacts::plan("x", 3)),
                BoardArtifact::Plan(board_artifacts::plan("x", 2)),
            ],
            IdUser::Plan(2),
        ),
    ] {
        let store = board();
        let head = store.read().unwrap().head;
        store.write(&head, &artifacts, "seed").unwrap();
        let writes = store.writes();
        assert_eq!(
            create(&store, SetClock::at("10:00:00")).execute(&new_task("x")),
            Err(ProjectBoardError::IdInUse {
                id: slug("x"),
                by: least.clone()
            })
        );
        assert_eq!(store.writes(), writes);
        assert!(
            create(&store, SetClock::at("10:00:00"))
                .execute(&new_task("y"))
                .is_ok(),
            "{least:?} names x alone"
        );
    }
}

/// While a file that may name the id is set aside, a create waits: a plan,
/// run, change or review of a task by that id, or any discussion (its task
/// is unknown). Creates are rare, so waiting for a mend costs little.
#[test]
fn a_create_waits_while_a_file_that_may_name_its_id_is_set_aside() {
    for (key, path) in [
        (ArtifactKey::Plan(slug("x"), 2), "plans/x.2.md"),
        (ArtifactKey::Run(slug("x"), 1), "runs/x.1.md"),
        (ArtifactKey::Change(slug("x"), 3), "changes/x.3.md"),
        (ArtifactKey::Review(slug("x"), 4), "reviews/x.4.md"),
        (ArtifactKey::Discussion(slug("chat")), "discussions/chat.md"),
    ] {
        let store = board();
        store.quarantine_key(ArtifactKey::Plan(slug("y"), 1), "plans/y.1.md", "malformed");
        assert!(
            create(&store, SetClock::at("10:00:00"))
                .execute(&new_task("w"))
                .is_ok(),
            "another task's file blocks nothing"
        );
        store.quarantine_key(key.clone(), path, "malformed");
        assert_eq!(
            create(&store, SetClock::at("10:00:00")).execute(&new_task("x")),
            Err(ProjectBoardError::Quarantined {
                key,
                path: path.into()
            })
        );
    }
}

/// A store whose writes each meet `per_write` lock races, as a clone
/// shared with other writers' fetches does.
struct Racing {
    store: MemoryBoardStore,
    races: std::sync::atomic::AtomicU64,
    per_write: u64,
}

impl ProjectBoardStore for Racing {
    fn read(
        &self,
    ) -> Result<crate::application::project_board::ports::BoardSnapshot, BoardStoreError> {
        self.store.client().read()
    }
    fn write(
        &self,
        expected: &crate::application::project_board::ports::BoardHead,
        artifacts: &[BoardArtifact],
        message: &str,
    ) -> Result<crate::application::project_board::ports::BoardWrite, BoardStoreError> {
        self.races
            .fetch_add(self.per_write, std::sync::atomic::Ordering::SeqCst);
        self.store.client().write(expected, artifacts, message)
    }
    fn clear_set_aside(
        &self,
        expected: &crate::application::project_board::ports::BoardHead,
        clear: &crate::application::project_board::ports::SetAsideClear,
        message: &str,
    ) -> Result<crate::application::project_board::ports::BoardWrite, BoardStoreError> {
        self.store
            .client()
            .clear_set_aside(expected, clear, message)
    }
    fn bootstrap(
        &self,
    ) -> Result<crate::application::project_board::ports::BoardBootstrap, BoardStoreError> {
        self.store.client().bootstrap()
    }
    fn writer(&self) -> crate::domain::project_board::entities::claim::Identity {
        self.store.client().writer()
    }
    fn lock_races(&self) -> u64 {
        self.races.load(std::sync::atomic::Ordering::SeqCst)
    }
}

/// #2663 review round 1: a write notes the lock races its store met during
/// it, never the ones met before.
#[test]
fn a_write_notes_the_lock_races_its_store_met() {
    let racing = Racing {
        store: board(),
        races: std::sync::atomic::AtomicU64::new(5),
        per_write: 2,
    };
    let creator = CreateBoardTask::new(Arc::new(racing), SetClock::at("10:00:00")).unwrap();
    let mut facts = crate::application::project_board::telemetry::OpFacts::default();
    creator
        .execute_noted(&new_task("a"), Some(&mut facts))
        .unwrap();
    assert_eq!(facts.lock_races, Some(2));
}
