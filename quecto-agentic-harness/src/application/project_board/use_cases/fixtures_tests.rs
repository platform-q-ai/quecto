//! Fixtures of the project board's use case tests: a set clock, a board
//! with clients, and new tasks' fields.
use std::sync::{Arc, Mutex};

use crate::application::project_board::dto::{NewBoardTask, NewBoardTaskStatus};
use crate::application::project_board::ports::{
    BoardClock, BoardClockError, ProjectBoardStore, ReferenceStatus,
};
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
/// counts the pauses between attempts instead of waiting, and its
/// monotonic time moves on by `tick` each time it is read.
pub struct SetClock {
    pub now: Mutex<Timestamp>,
    pub reference: Option<Timestamp>,
    pub pauses: Mutex<Vec<usize>>,
    pub elapsed: Mutex<std::time::Duration>,
    pub tick: Mutex<std::time::Duration>,
}

impl SetClock {
    pub fn at(time: &str) -> Arc<Self> {
        Self::with_reference(time, None)
    }

    pub fn with_reference(time: &str, reference: Option<&str>) -> Arc<Self> {
        Self::reading(at(time), reference.map(at))
    }

    /// A clock reading `now`, with `reference`, on any day.
    pub fn reading(now: Timestamp, reference: Option<Timestamp>) -> Arc<Self> {
        Arc::new(Self {
            now: Mutex::new(now),
            reference,
            pauses: Mutex::new(Vec::new()),
            elapsed: Mutex::new(std::time::Duration::ZERO),
            tick: Mutex::new(std::time::Duration::ZERO),
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

    fn reference_status(&self) -> ReferenceStatus {
        match self.reference {
            Some(_) => ReferenceStatus::Referenced,
            None => ReferenceStatus::NoReference("set without one".into()),
        }
    }

    fn pause(&self, attempt: usize) {
        self.pauses.lock().unwrap().push(attempt);
    }

    fn elapsed(&self) -> std::time::Duration {
        let mut elapsed = self.elapsed.lock().unwrap();
        let now = *elapsed;
        *elapsed += *self.tick.lock().unwrap();
        now
    }
}

/// A bootstrapped, empty board.
pub fn board() -> MemoryBoardStore {
    let store = MemoryBoardStore::default();
    store.bootstrap().unwrap();
    store
}

/// A new ready task.
pub fn new_task(id: &str) -> NewBoardTask {
    NewBoardTask {
        id: slug(id),
        title: format!("Task {id}"),
        kind: TaskKind::Task,
        description: String::new(),
        status: NewBoardTaskStatus::Ready,
        parent: None,
        depends_on: vec![],
    }
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
        accepted_plan: None,
        claim: None,
        prs: vec![],
        created: at("00:00:00"),
        updated: at("00:00:00"),
    }
}
