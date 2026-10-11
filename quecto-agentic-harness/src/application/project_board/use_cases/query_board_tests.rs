use super::QueryBoard;
use crate::application::project_board::ports::ProjectBoardStore;
use crate::application::project_board::use_cases::fixtures_tests::{at, board, fields, slug};
use crate::domain::project_board::entities::task::{Task, TaskFields};
use crate::domain::project_board::value_objects::vocabulary::TaskStatus;
use std::sync::Arc;

/// c (draft, child of a), b (ready, child of a, depends on a), a (ready),
/// d (done, depends on a and b), written out of order.
fn tasks() -> Vec<Task> {
    let task = |fields: TaskFields| {
        Task::new(TaskFields {
            created: at("08:00:00"),
            updated: at("08:00:00"),
            ..fields
        })
        .unwrap()
    };
    vec![
        task(TaskFields {
            status: TaskStatus::Draft,
            parent: Some(slug("a")),
            ..fields("c")
        }),
        task(TaskFields {
            parent: Some(slug("a")),
            depends_on: vec![slug("a")],
            ..fields("b")
        }),
        task(fields("a")),
        task(TaskFields {
            status: TaskStatus::Done,
            depends_on: vec![slug("a"), slug("b")],
            ..fields("d")
        }),
    ]
}

fn ids(tasks: Vec<Task>) -> Vec<String> {
    tasks
        .iter()
        .map(|task| task.fields().id.as_str().to_owned())
        .collect()
}

#[test]
fn listings_answer_ready_status_children_and_dependents_by_id() {
    let store = board();
    let head = store.read().unwrap().head;
    store.write(&head, &tasks(), "seed").unwrap();
    let query = QueryBoard::new(Arc::new(store));
    assert_eq!(ids(query.ready().unwrap()), ["a", "b"]);
    assert_eq!(ids(query.by_status(TaskStatus::Draft).unwrap()), ["c"]);
    assert_eq!(
        ids(query.by_status(TaskStatus::Archived).unwrap()),
        Vec::<String>::new()
    );
    assert_eq!(ids(query.children(&slug("a")).unwrap()), ["b", "c"]);
    assert_eq!(ids(query.dependents(&slug("a")).unwrap()), ["b", "d"]);
    assert_eq!(ids(query.dependents(&slug("b")).unwrap()), ["d"]);
}
