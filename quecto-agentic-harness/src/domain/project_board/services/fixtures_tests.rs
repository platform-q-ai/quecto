//! A valid task in any status, with every part, held by Ada from 09:00 to
//! 11:00 when it may be; and versions of its plan and runs of them.
use crate::domain::project_board::entities::claim::{Claim, Identity};
use crate::domain::project_board::entities::plan::{Plan, PlanFields, PlanItem};
use crate::domain::project_board::entities::run::{Run, RunFields};
use crate::domain::project_board::entities::task::{Task, TaskFields};
use crate::domain::project_board::entities::task_parts::{Review, Role, RoleLimits, Team};
use crate::domain::project_board::value_objects::slug::Slug;
use crate::domain::project_board::value_objects::timestamp::Timestamp;
use crate::domain::project_board::value_objects::vocabulary::{
    Effort, PlanState, Severity, TaskKind, TaskStatus, Verdict,
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
        accepted_plan: Some(1),
        claim,
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

fn slug(text: &str) -> Slug {
    Slug::parse(text).unwrap()
}

/// The team a plan of the fixture task names.
pub fn team() -> Team {
    Team {
        roles: vec![Role {
            name: slug("worker"),
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
    }
}

/// Version `version` of the fixture task's plan, in `state`, written at
/// 08:30, with two items (`api` after `schema`).
pub fn plan(version: u32, state: PlanState) -> Plan {
    let item = |id: &str, depends_on: &[&str]| PlanItem {
        id: slug(id),
        title: format!("Item {id}"),
        criteria: vec![format!("{id} proved")],
        depends_on: depends_on.iter().map(|id| slug(id)).collect(),
    };
    Plan::new(PlanFields {
        task: slug("board-store"),
        version,
        state,
        approach: "Spike, then build.\n".into(),
        items: vec![item("schema", &[]), item("api", &["schema"])],
        team: Some(team()),
        created: at("08:30:00"),
        updated: at("08:30:00"),
    })
    .expect("fixture plan is valid")
}

/// Run `number` of the fixture task's plan `version`, going since 09:00.
pub fn open_run(number: u32, version: u32) -> Run {
    Run::new(RunFields {
        task: slug("board-store"),
        number,
        plan_version: version,
        started: at("09:00:00"),
        ended: None,
        outcome: None,
        cost: None,
        items: vec![],
    })
    .expect("fixture run is valid")
}
