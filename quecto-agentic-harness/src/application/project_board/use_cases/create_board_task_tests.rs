use super::CreateBoardTask;
use crate::application::project_board::dto::ProjectBoardError;
use crate::application::project_board::ports::{BoardStoreError, ProjectBoardStore};
use crate::application::project_board::use_cases::MAX_WRITE_ATTEMPTS;
use crate::application::project_board::use_cases::fixtures_tests::{
    SetClock, at, board, fields, person, slug,
};
use crate::domain::project_board::entities::claim::Claim;
use crate::domain::project_board::entities::task::{Task, TaskFields};
use crate::domain::project_board::entities::task_parts::Run;
use crate::domain::project_board::value_objects::vocabulary::TaskStatus;
use crate::infrastructure::test_support::memory_board_store::MemoryBoardStore;
use std::sync::Arc;

fn create(store: &MemoryBoardStore, clock: Arc<SetClock>) -> CreateBoardTask {
    CreateBoardTask::new(Arc::new(store.client()), clock)
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
        .execute(fields("a"))
        .unwrap();
    assert_eq!(
        (&task.fields().created, &task.fields().updated),
        (&at("10:00:00"), &at("10:00:00"))
    );
    assert_eq!(store.read().unwrap().tasks, vec![task]);
}

#[test]
fn only_an_unclaimed_draft_or_ready_task_with_no_history_is_created() {
    let store = board();
    let claim = Claim {
        holder: person("Ada"),
        since: at("10:00:00"),
        expires: at("11:00:00"),
    };
    let run = Run {
        id: slug("r1"),
        started: at("09:00:00"),
        ended: None,
        outcome: None,
    };
    for (case, task) in [
        (
            "claimed",
            TaskFields {
                status: TaskStatus::Claimed,
                ..fields("a")
            },
        ),
        (
            "done",
            TaskFields {
                status: TaskStatus::Done,
                ..fields("a")
            },
        ),
        (
            "ready with a claim",
            TaskFields {
                claim: Some(claim),
                ..fields("a")
            },
        ),
        (
            "a linked PR",
            TaskFields {
                prs: vec![7],
                ..fields("a")
            },
        ),
        (
            "a run",
            TaskFields {
                runs: vec![run],
                ..fields("a")
            },
        ),
    ] {
        let refused = create(&store, SetClock::at("10:00:00")).execute(task);
        assert!(
            matches!(refused, Err(ProjectBoardError::NotCreatable(_))),
            "{case}: {refused:?}"
        );
    }
    assert_eq!(store.writes(), 0);
}

/// A parent or dependency must already be on the board, so no two tasks
/// can ever be created depending on each other.
#[test]
fn a_parent_or_dependency_not_on_the_board_is_refused() {
    let store = board();
    let a_on_b = TaskFields {
        depends_on: vec![slug("b")],
        ..fields("a")
    };
    let refused = create(&store, SetClock::at("10:00:00")).execute(a_on_b.clone());
    assert_eq!(refused, Err(ProjectBoardError::UnknownReference(slug("b"))));
    let orphan = create(&store, SetClock::at("10:00:00")).execute(TaskFields {
        parent: Some(slug("p")),
        ..fields("a")
    });
    assert_eq!(orphan, Err(ProjectBoardError::UnknownReference(slug("p"))));
    create(&store, SetClock::at("10:00:00"))
        .execute(fields("b"))
        .unwrap();
    create(&store, SetClock::at("10:01:00"))
        .execute(a_on_b)
        .unwrap();
}

#[test]
fn an_id_held_or_set_aside_is_refused() {
    let store = board();
    create(&store, SetClock::at("10:00:00"))
        .execute(fields("a"))
        .unwrap();
    let again = create(&store, SetClock::at("10:01:00")).execute(fields("a"));
    assert_eq!(again, Err(ProjectBoardError::Exists(slug("a"))));
    store.quarantine("q", "malformed");
    let over = create(&store, SetClock::at("10:01:00")).execute(fields("q"));
    assert_eq!(over, Err(ProjectBoardError::Quarantined(slug("q"))));
}

/// Another writer moves the board between the read and the write: the
/// create is built again against the fresh board and lands.
#[test]
fn a_lost_write_is_retried_against_the_fresh_board() {
    let store = board();
    store.before_next_write(|board| other_writes(board, "b"));
    create(&store, SetClock::at("10:00:00"))
        .execute(fields("a"))
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
    let refused = create(&store, SetClock::at("10:00:00")).execute(fields("a"));
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
    let created = create(&store, clock.clone()).execute(fields("a"));
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
    .execute(fields("a"));
    assert!(
        matches!(ahead, Err(ProjectBoardError::ClockAhead { .. })),
        "{ahead:?}"
    );
    let close = create(
        &store,
        SetClock::with_reference("10:01:00", Some("10:00:00")),
    )
    .execute(fields("a"));
    assert!(close.is_ok(), "60 s ahead is allowed");
}

#[test]
fn a_board_not_bootstrapped_is_reported() {
    let store = MemoryBoardStore::default();
    let refused = create(&store, SetClock::at("10:00:00")).execute(fields("a"));
    assert_eq!(
        refused,
        Err(ProjectBoardError::Store(BoardStoreError::NoBoard))
    );
}
