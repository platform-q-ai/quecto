use super::UpdateDraftTask;
use crate::application::project_board::dto::ProjectBoardError;
use crate::application::project_board::ports::ProjectBoardStore;
use crate::application::project_board::use_cases::fixtures_tests::{
    SetClock, at, board, fields, slug,
};
use crate::domain::project_board::entities::task::{Task, TaskFields};
use crate::domain::project_board::services::drafts::DraftEdit;
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

fn references(parent: Option<Option<&str>>, depends_on: Option<&[&str]>) -> DraftEdit {
    DraftEdit {
        parent: parent.map(|parent| parent.map(slug)),
        depends_on: depends_on.map(|ids| ids.iter().map(|id| slug(id)).collect()),
        ..DraftEdit::default()
    }
}

#[test]
fn a_draft_is_edited_and_written_now() {
    let store = holding(&[
        ("a", TaskStatus::Draft, None, &[]),
        ("b", TaskStatus::Ready, None, &[]),
    ]);
    let edit = DraftEdit {
        title: Some("Retitled".into()),
        ..references(Some(Some("b")), Some(&["b"]))
    };
    let edited = update(&store).execute(&slug("a"), &edit).unwrap();
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
        update(&store).execute(&slug("a"), &references(None, Some(&["missing"]))),
        Err(ProjectBoardError::UnknownReference(slug("missing")))
    );
    assert_eq!(
        update(&store).execute(&slug("a"), &references(Some(Some("x")), None)),
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
        update(&store).execute(&slug("a"), &references(None, Some(&["b"]))),
        Err(ProjectBoardError::ClosesCycle {
            id: slug("a"),
            by: slug("b")
        })
    );
    assert_eq!(
        update(&store).execute(&slug("a"), &references(Some(Some("c")), None)),
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
        .execute(&slug("a"), &references(None, Some(&[])))
        .unwrap();
    assert!(dropped.fields().depends_on.is_empty());
    let turned = update(&store).execute(&slug("b"), &references(None, Some(&["a"])));
    assert!(turned.is_ok(), "{turned:?}");
}

#[test]
fn only_a_draft_on_the_board_is_edited() {
    let store = holding(&[("r", TaskStatus::Ready, None, &[])]);
    let retitle = DraftEdit {
        title: Some("New".into()),
        ..DraftEdit::default()
    };
    assert_eq!(
        update(&store).execute(&slug("r"), &retitle),
        Err(ProjectBoardError::Rule(BoardRuleError::NotDraft(
            TaskStatus::Ready
        )))
    );
    assert_eq!(
        update(&store).execute(&slug("none"), &retitle),
        Err(ProjectBoardError::NotFound(slug("none")))
    );
}
