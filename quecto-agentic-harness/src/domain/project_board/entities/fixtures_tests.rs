//! An entity's fixtures: a team, versions of a task's plan, and runs of
//! them, all of the task `board-store`.
use super::claim::Identity;
use super::plan::{Plan, PlanFields, PlanItem};
use super::run::{Run, RunFields};
use super::team::{Role, RoleLimits, Team};
use crate::domain::project_board::value_objects::slug::Slug;
use crate::domain::project_board::value_objects::timestamp::Timestamp;
use crate::domain::project_board::value_objects::vocabulary::{Effort, PlanState};

pub fn at(time: &str) -> Timestamp {
    Timestamp::parse(&format!("2026-10-10T{time}Z")).unwrap()
}

pub fn person(name: &str) -> Identity {
    Identity {
        name: name.into(),
        email: format!("{}@example.com", name.to_lowercase()),
    }
}

pub fn slug(text: &str) -> Slug {
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
        approach: "Spike, then build.".into(),
        items: vec![item("schema", &[]), item("api", &["schema"])],
        team: Some(team()),
        proposed_by: person("Ada"),
        edited_by: None,
        accepted: match state {
            PlanState::Accepted | PlanState::Superseded => Some((person("Ada"), at("08:30:00"))),
            PlanState::Draft | PlanState::Discarded => None,
        },
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
        environment: None,
    })
    .expect("fixture run is valid")
}

/// Change `number` of the fixture task linking branch `feat/b<number>` of
/// the board's repository and pull request `pr`, linked by Ada at 09:00.
pub fn change(number: u32, pr: Option<u64>) -> super::change::Change {
    super::change::Change::new(super::change::ChangeFields {
        task: slug("board-store"),
        number,
        head_repo: repo("platform-q-ai/quecto"),
        branch: branch(&format!("feat/b{number}")),
        pr,
        merged: None,
        linked_by: person("Ada"),
        unlinked: None,
        created: at("09:00:00"),
        updated: at("09:00:00"),
    })
    .expect("fixture change is valid")
}

/// A full commit id of `digit`s.
pub fn commit(digit: char) -> crate::domain::project_board::value_objects::code_ref::CommitId {
    crate::domain::project_board::value_objects::code_ref::CommitId::parse(
        &digit.to_string().repeat(40),
    )
    .unwrap()
}

/// A branch name.
pub fn branch(name: &str) -> crate::domain::project_board::value_objects::code_ref::BranchName {
    crate::domain::project_board::value_objects::code_ref::BranchName::parse(name).unwrap()
}

/// A repository's `owner/name`.
pub fn repo(name: &str) -> crate::domain::project_board::value_objects::code_ref::RepoName {
    crate::domain::project_board::value_objects::code_ref::RepoName::parse(name).unwrap()
}
