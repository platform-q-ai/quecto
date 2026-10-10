//! A valid task in any status, held by Ada from 09:00 to 11:00 when it may be.
use crate::domain::project_board::entities::claim::{Claim, Identity};
use crate::domain::project_board::entities::task::{Task, TaskFields};
use crate::domain::project_board::value_objects::slug::Slug;
use crate::domain::project_board::value_objects::timestamp::Timestamp;
use crate::domain::project_board::value_objects::vocabulary::{TaskKind, TaskStatus};

pub fn at(time: &str) -> Timestamp {
    Timestamp::parse(&format!("2026-10-10T{time}Z")).unwrap()
}

pub fn person(name: &str) -> Identity {
    Identity {
        name: name.into(),
        email: format!("{}@example.com", name.to_lowercase()),
    }
}

pub fn task(status: TaskStatus) -> Task {
    let claim = status.may_hold_claim().then(|| Claim {
        holder: person("Ada"),
        since: at("09:00:00"),
        expires: at("11:00:00"),
    });
    Task::new(TaskFields {
        id: Slug::parse("board-store").unwrap(),
        title: "Board store".into(),
        kind: TaskKind::Task,
        description: String::new(),
        status,
        parent: None,
        depends_on: vec![],
        claim,
        prs: vec![2482, 2483],
        created: at("08:00:00"),
        updated: at("09:00:00"),
    })
    .expect("fixture is valid")
}
