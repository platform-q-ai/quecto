//! A valid task in any status, with every part, held by Ada from 09:00 to
//! 11:00 when it may be.
use crate::domain::project_board::entities::claim::{Claim, Identity};
use crate::domain::project_board::entities::task::{Task, TaskFields};
use crate::domain::project_board::entities::task_parts::{
    Item, PlanStep, Review, Role, RoleLimits, Run, Team,
};
use crate::domain::project_board::value_objects::slug::Slug;
use crate::domain::project_board::value_objects::timestamp::Timestamp;
use crate::domain::project_board::value_objects::vocabulary::{
    Effort, ItemState, RunOutcome, Severity, TaskKind, TaskStatus, Verdict,
};

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
        plan: vec![PlanStep {
            title: "Spike".into(),
            criteria: vec!["CAS proved".into()],
        }],
        items: vec![Item {
            id: Slug::parse("plumbing").unwrap(),
            title: "Plumbing".into(),
            criteria: vec![],
            depends_on: vec![],
            state: ItemState::Done,
            owner: None,
            evidence: vec![],
        }],
        team: Some(Team {
            roles: vec![Role {
                name: Slug::parse("worker").unwrap(),
                model: "gpt-5.5".into(),
                effort: Effort::High,
                limits: RoleLimits {
                    members: 2,
                    turns: None,
                },
            }],
            budget: None,
            deadline: None,
            image: None,
        }),
        claim,
        runs: vec![Run {
            id: Slug::parse("r1").unwrap(),
            started: at("08:30:00"),
            ended: Some(at("08:45:00")),
            outcome: Some(RunOutcome::Succeeded),
        }],
        reviews: vec![Review {
            id: Slug::parse("r1-h1").unwrap(),
            round: 1,
            reviewer: person("Bob"),
            at: at("08:50:00"),
            severity: Severity::High,
            location: Some("src/store.rs:42".into()),
            finding: "Lease not checked".into(),
            verdict: Verdict::Fixed,
            proving_test: None,
        }],
        prs: vec![2482, 2483],
        created: at("08:00:00"),
        updated: at("09:00:00"),
    })
    .expect("fixture is valid")
}

/// `task` with a run still going since 09:00.
pub fn with_open_run(task: Task) -> Task {
    let mut fields = task.into_fields();
    fields.runs.push(Run {
        id: Slug::parse("r2").unwrap(),
        started: at("09:00:00"),
        ended: None,
        outcome: None,
    });
    Task::new(fields).expect("an open run while the task may hold a claim")
}

/// `task` with no claim (a review or blocked task may have none).
pub fn without_claim(task: Task) -> Task {
    let mut fields = task.into_fields();
    fields.claim = None;
    Task::new(fields).expect("review and blocked tasks may drop their claim")
}
