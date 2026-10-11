//! Fixtures of the project board's use case tests: a board with clients,
//! and new tasks' fields.
use crate::application::project_board::ports::ProjectBoardStore;
use crate::domain::project_board::entities::task::TaskFields;
use crate::domain::project_board::value_objects::slug::Slug;
use crate::domain::project_board::value_objects::timestamp::Timestamp;
use crate::domain::project_board::value_objects::vocabulary::{TaskKind, TaskStatus};
use crate::infrastructure::test_support::memory_board_store::MemoryBoardStore;

pub fn at(time: &str) -> Timestamp {
    Timestamp::parse(&format!("2026-10-10T{time}Z")).unwrap()
}

pub fn slug(text: &str) -> Slug {
    Slug::parse(text).unwrap()
}

/// A bootstrapped, empty board.
pub fn board() -> MemoryBoardStore {
    let store = MemoryBoardStore::default();
    store.bootstrap().unwrap();
    store
}

/// A new task's fields: ready, unclaimed, with no parts.
pub fn fields(id: &str) -> TaskFields {
    TaskFields {
        id: slug(id),
        title: format!("Task {id}"),
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
        created: at("00:00:00"),
        updated: at("00:00:00"),
    }
}
