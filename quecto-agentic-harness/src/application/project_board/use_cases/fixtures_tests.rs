//! Fixtures of the project board's use case tests: a set clock, a board
//! with clients, and new tasks' fields.
use std::sync::{Arc, Mutex};

use crate::application::project_board::ports::{BoardClock, BoardClockError, ProjectBoardStore};
use crate::domain::project_board::entities::claim::Identity;
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

pub fn person(name: &str) -> Identity {
    Identity {
        name: name.into(),
        email: format!("{}@example.com", name.to_lowercase()),
    }
}

/// A clock reading a set time, with an optional outside reference; it
/// counts the pauses between attempts instead of waiting.
pub struct SetClock {
    pub now: Mutex<Timestamp>,
    pub reference: Option<Timestamp>,
    pub pauses: Mutex<Vec<usize>>,
}

impl SetClock {
    pub fn at(time: &str) -> Arc<Self> {
        Self::with_reference(time, None)
    }

    pub fn with_reference(time: &str, reference: Option<&str>) -> Arc<Self> {
        Arc::new(Self {
            now: Mutex::new(at(time)),
            reference: reference.map(at),
            pauses: Mutex::new(Vec::new()),
        })
    }
}

impl BoardClock for SetClock {
    fn now(&self) -> Result<Timestamp, BoardClockError> {
        Ok(self.now.lock().unwrap().clone())
    }

    fn reference(&self) -> Option<Timestamp> {
        self.reference.clone()
    }

    fn pause(&self, attempt: usize) {
        self.pauses.lock().unwrap().push(attempt);
    }
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
