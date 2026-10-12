use super::UpdateDraftTask;
use crate::application::project_board::dto::{DraftUpdate, ProjectBoardError};
use crate::application::project_board::ports::ProjectBoardStore;
use crate::application::project_board::use_cases::fixtures_tests::{
    SetClock, at, board, fields, slug,
};
use crate::domain::project_board::entities::task::{Task, TaskFields};
use crate::domain::project_board::value_objects::rule_error::BoardRuleError;
use crate::domain::project_board::value_objects::vocabulary::TaskStatus;
use crate::infrastructure::test_support::memory_board_store::MemoryBoardStore;
use std::sync::Arc;

/// A board holding `tasks`, each `(id, status, parent, depends_on)`.
fn holding(tasks: &[(&str, TaskStatus, Option<&str>, &[&str])]) -> MemoryBoardStore {
    let store = board();
    let head = store.read().unwrap().head;
    let tasks: Vec<Task> = tasks
        .iter()
        .map(|(id, status, parent, depends_on)| {
            Task::new(TaskFields {
                status: *status,
                parent: parent.map(slug),
                depends_on: depends_on.iter().map(|id| slug(id)).collect(),
                created: at("09:00:00"),
                updated: at("09:00:00"),
                ..fields(id)
            })
            .unwrap()
        })
        .collect();
    store.write(&head, &tasks, "seed").unwrap();
    store
}

fn update(store: &MemoryBoardStore) -> UpdateDraftTask {
    UpdateDraftTask::new(Arc::new(store.client()), SetClock::at("10:00:00")).unwrap()
}

/// An update of draft `id` setting nothing yet.
fn of(id: &str) -> DraftUpdate {
    DraftUpdate {
        id: slug(id),
        title: None,
        description: None,
        parent: None,
        depends_on: None,
        expected_updated: None,
    }
}

fn references(id: &str, parent: Option<Option<&str>>, depends_on: Option<&[&str]>) -> DraftUpdate {
    DraftUpdate {
        parent: parent.map(|parent| parent.map(slug)),
        depends_on: depends_on.map(|ids| ids.iter().map(|id| slug(id)).collect()),
        ..of(id)
    }
}

fn retitle(id: &str) -> DraftUpdate {
    DraftUpdate {
        title: Some("New".into()),
        ..of(id)
    }
}

#[test]
fn a_draft_is_edited_and_written_now() {
    let store = holding(&[
        ("a", TaskStatus::Draft, None, &[]),
        ("b", TaskStatus::Ready, None, &[]),
    ]);
    let edit = DraftUpdate {
        title: Some("Retitled".into()),
        ..references("a", Some(Some("b")), Some(&["b"]))
    };
    let edited = update(&store).execute(&edit).unwrap();
    let fields = edited.fields();
    assert_eq!(
        (
            fields.title.as_str(),
            fields.parent.clone(),
            fields.depends_on.clone(),
            &fields.updated
        ),
        (
            "Retitled",
            Some(slug("b")),
            vec![slug("b")],
            &at("10:00:00")
        )
    );
    let on_board = store.read().unwrap().tasks;
    assert!(on_board.contains(&edited), "{on_board:?}");
}

#[test]
fn references_must_be_on_the_board_and_not_set_aside() {
    let store = holding(&[
        ("a", TaskStatus::Draft, None, &[]),
        ("x", TaskStatus::Ready, None, &[]),
    ]);
    store.quarantine("x", "malformed");
    assert_eq!(
        update(&store).execute(&references("a", None, Some(&["missing"]))),
        Err(ProjectBoardError::UnknownReference(slug("missing")))
    );
    assert_eq!(
        update(&store).execute(&references("a", Some(Some("x")), None)),
        Err(ProjectBoardError::Quarantined(slug("x")))
    );
}

#[test]
fn an_edit_never_closes_a_cycle() {
    // b depends on a, c's parent is a: a may not depend on b, nor take c as
    // its parent.
    let store = holding(&[
        ("a", TaskStatus::Draft, None, &[]),
        ("b", TaskStatus::Ready, None, &["a"]),
        ("c", TaskStatus::Ready, Some("a"), &[]),
    ]);
    assert_eq!(
        update(&store).execute(&references("a", None, Some(&["b"]))),
        Err(ProjectBoardError::ClosesCycle {
            id: slug("a"),
            by: slug("b")
        })
    );
    assert_eq!(
        update(&store).execute(&references("a", Some(Some("c")), None)),
        Err(ProjectBoardError::ClosesCycle {
            id: slug("a"),
            by: slug("c")
        })
    );
}

#[test]
fn the_drafts_old_references_never_count_against_its_new_ones() {
    // a depends on b. Once a's edit drops that edge, b may depend on a:
    // the walk follows a's new references, never its old ones.
    let store = holding(&[
        ("b", TaskStatus::Draft, None, &[]),
        ("a", TaskStatus::Draft, None, &["b"]),
    ]);
    let dropped = update(&store)
        .execute(&references("a", None, Some(&[])))
        .unwrap();
    assert!(dropped.fields().depends_on.is_empty());
    let turned = update(&store).execute(&references("b", None, Some(&["a"])));
    assert!(turned.is_ok(), "{turned:?}");
}

#[test]
fn only_a_draft_on_the_board_is_edited_and_the_edit_refuses_first() {
    // r is ready and depends on a task that is gone: the edit's own
    // refusal comes before any reference is looked at.
    let store = holding(&[("r", TaskStatus::Ready, None, &["gone"])]);
    assert_eq!(
        update(&store).execute(&retitle("r")),
        Err(ProjectBoardError::Rule(BoardRuleError::NotDraft(
            TaskStatus::Ready
        )))
    );
    assert_eq!(
        update(&store).execute(&references("r", None, Some(&["missing"]))),
        Err(ProjectBoardError::Rule(BoardRuleError::NotDraft(
            TaskStatus::Ready
        ))),
        "a relation it sets is looked at only once the edit is allowed"
    );
    assert_eq!(
        update(&store).execute(&retitle("none")),
        Err(ProjectBoardError::NotFound(slug("none")))
    );
}

#[test]
fn a_relation_the_edit_leaves_alone_is_not_checked_again() {
    let store = holding(&[
        ("x", TaskStatus::Ready, None, &[]),
        ("a", TaskStatus::Draft, Some("x"), &["x"]),
    ]);
    store.quarantine("x", "malformed");
    let retitled = update(&store).execute(&retitle("a"));
    assert!(retitled.is_ok(), "{retitled:?}");
    assert_eq!(
        update(&store).execute(&references("a", None, Some(&["x"]))),
        Err(ProjectBoardError::Rule(BoardRuleError::NoEdit)),
        "setting what it holds changes nothing"
    );
}

#[test]
fn a_draft_changed_since_it_was_read_is_refused_as_stale() {
    let store = holding(&[("a", TaskStatus::Draft, None, &[])]);
    let read = store.read().unwrap().tasks[0].fields().updated.clone();
    let stale = DraftUpdate {
        expected_updated: Some(at("08:00:00")),
        ..retitle("a")
    };
    assert_eq!(
        update(&store).execute(&stale),
        Err(ProjectBoardError::Stale {
            id: slug("a"),
            expected: at("08:00:00"),
            updated: read.clone(),
        })
    );
    let fresh = DraftUpdate {
        expected_updated: Some(read),
        ..retitle("a")
    };
    assert!(update(&store).execute(&fresh).is_ok());
}

#[test]
fn board_text_is_normalised_before_the_schema_reads_it() {
    let store = holding(&[("a", TaskStatus::Draft, None, &[])]);
    let edit = DraftUpdate {
        title: Some(" New title\t".into()),
        description: Some("Why. \r\nHow so.\r\n".into()),
        ..of("a")
    };
    let edited = update(&store).execute(&edit).unwrap();
    assert_eq!(edited.fields().title, "New title");
    assert_eq!(edited.fields().description, "Why.\nHow so.");
}

#[test]
fn two_edits_in_one_second_are_told_apart() {
    let store = holding(&[("a", TaskStatus::Draft, None, &[])]);
    let first = update(&store).execute(&retitle("a")).unwrap();
    let read = first.fields().updated.clone();
    let second = DraftUpdate {
        title: Some("Second".into()),
        expected_updated: Some(read.clone()),
        ..of("a")
    };
    let second = update(&store).execute(&second).unwrap();
    assert_ne!(
        second.fields().updated,
        read,
        "the same second moves updated on"
    );
    let late = DraftUpdate {
        title: Some("Late".into()),
        expected_updated: Some(read),
        ..of("a")
    };
    assert!(matches!(
        update(&store).execute(&late),
        Err(ProjectBoardError::Stale { .. })
    ));
}
