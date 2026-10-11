use super::CreateBoardTask;
use crate::application::project_board::dto::{NewBoardTask, NewBoardTaskStatus, ProjectBoardError};
use crate::application::project_board::ports::{BoardStoreError, ProjectBoardStore};
use crate::application::project_board::use_cases::MAX_WRITE_ATTEMPTS;
use crate::application::project_board::use_cases::fixtures_tests::{
    SetClock, at, board, fields, new_task, slug,
};
use crate::domain::project_board::entities::task::{Task, TaskFields};
use crate::domain::project_board::entities::task_parts::PlanStep;
use crate::domain::project_board::value_objects::vocabulary::TaskStatus;
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
    board.write(&head, &[task], "other").unwrap();
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

/// A new task holds only what a plan sets: it is created a draft or
/// ready, with its plan, and no claim, runs, reviews, pull requests or
/// items, which `NewBoardTask` cannot carry.
#[test]
fn a_new_task_holds_only_what_a_plan_sets() {
    let store = board();
    let plan = vec![PlanStep {
        title: "Spike".into(),
        criteria: vec!["CAS proved".into()],
    }];
    let draft = create(&store, SetClock::at("10:00:00"))
        .execute(&NewBoardTask {
            status: NewBoardTaskStatus::Draft,
            description: "Why and how.".into(),
            plan: plan.clone(),
            ..new_task("a")
        })
        .unwrap();
    assert_eq!(
        draft.into_fields(),
        TaskFields {
            status: TaskStatus::Draft,
            description: "Why and how.".into(),
            plan,
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
    assert_eq!(over, Err(ProjectBoardError::Quarantined(slug("q"))));
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
    assert_eq!(child, Err(ProjectBoardError::Quarantined(slug("p"))));
    let dependent = create(&store, SetClock::at("10:01:00")).execute(&NewBoardTask {
        depends_on: vec![slug("d")],
        ..new_task("a")
    });
    assert_eq!(dependent, Err(ProjectBoardError::Quarantined(slug("d"))));
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
    assert_eq!(child, Err(ProjectBoardError::Quarantined(slug("p"))));
    assert_eq!(*clock.pauses.lock().unwrap(), [2], "made again once");
    assert_eq!(store.writes(), 1, "only p was written");
}

/// b's file was quarantined and then deleted, so a still depends on a b
/// the board no longer holds. A new b depending on a would close a cycle:
/// a new id any task already names is refused.
#[test]
fn a_new_id_another_task_already_names_is_refused() {
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
    let writes = store.writes();
    let cycle = create(&store, SetClock::at("10:03:00")).execute(&NewBoardTask {
        depends_on: vec![slug("a")],
        ..new_task("b")
    });
    assert_eq!(
        cycle,
        Err(ProjectBoardError::Referenced {
            id: slug("b"),
            by: slug("a")
        })
    );
    assert_eq!(store.writes(), writes);
}
