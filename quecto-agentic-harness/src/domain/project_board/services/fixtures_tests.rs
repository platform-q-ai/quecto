//! A valid task in any status, with every part, held by Ada from 09:00 to
//! 11:00 when it may be; and versions of its plan and runs of them.
use crate::domain::project_board::entities::claim::{Claim, Identity};
pub use crate::domain::project_board::entities::fixtures_tests::{
    branch, change, commit, open_run, plan, repo,
};
use crate::domain::project_board::entities::task::{Task, TaskFields};
use crate::domain::project_board::value_objects::slug::Slug;
use crate::domain::project_board::value_objects::timestamp::Timestamp;
use crate::domain::project_board::value_objects::vocabulary::{Resolution, TaskKind, TaskStatus};

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
        accepted_plan: Some(1),
        claim,
        prs: vec![2482, 2483],
        github_issue: None,
        // A done task is completed.
        resolution: match status {
            TaskStatus::Done => Some(Resolution::Completed),
            _ => None,
        },
        superseded_by: vec![],
        references: vec![],
        created: at("08:00:00"),
        updated: at("09:00:00"),
        description_form: Default::default(),
    })
    .expect("fixture is valid")
}

/// `task` with no claim (a review or blocked task may have none).
pub fn without_claim(task: Task) -> Task {
    let mut fields = task.into_fields();
    fields.claim = None;
    Task::new(fields).expect("review and blocked tasks may drop their claim")
}

/// A moment from a client whose clock ran far ahead.
pub fn far_future() -> Timestamp {
    Timestamp::parse("2036-01-01T00:00:00Z").unwrap()
}

/// `task` as last written at `updated` (by a fast clock, say).
pub fn last_written(task: Task, updated: &Timestamp) -> Task {
    let mut fields = task.into_fields();
    fields.updated = updated.clone();
    Task::new(fields).expect("a later write keeps the task valid")
}

/// The task with no linked pull requests: work its holder may finish.
pub fn without_prs(task: Task) -> Task {
    let mut fields = task.into_fields();
    fields.prs.clear();
    Task::new(fields).unwrap()
}
