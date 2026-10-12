//! One valid artifact of each kind a board holds besides tasks (#2481,
//! M3): a draft plan, an open run, a linked change, an approved review and
//! an open discussion, for the store contracts and the board views' tests.
//! Each is named by what keys it, so tests can write many of a kind.
use crate::domain::project_board::entities::change::{Change, ChangeFields};
use crate::domain::project_board::entities::claim::Identity;
use crate::domain::project_board::entities::discussion::{Discussion, DiscussionFields, Message};
use crate::domain::project_board::entities::plan::{Plan, PlanFields, PlanItem};
use crate::domain::project_board::entities::review::{Review, ReviewFields, Round};
use crate::domain::project_board::entities::run::{Run, RunFields};
use crate::domain::project_board::value_objects::code_ref::{BranchName, CommitId, RepoName};
use crate::domain::project_board::value_objects::slug::Slug;
use crate::domain::project_board::value_objects::timestamp::Timestamp;
use crate::domain::project_board::value_objects::vocabulary::{
    DiscussionState, PlanState, ReviewVerdict,
};

fn at() -> Timestamp {
    Timestamp::parse("2026-10-10T09:00:00Z").expect("fixture time is valid")
}

fn slug(text: &str) -> Slug {
    Slug::parse(text).expect("fixture slug is valid")
}

fn ada() -> Identity {
    Identity {
        name: "Ada".into(),
        email: "ada@example.com".into(),
    }
}

/// Draft version `version` of task `task`'s plan, of one item.
pub fn plan(task: &str, version: u32) -> Plan {
    Plan::new(PlanFields {
        task: slug(task),
        version,
        state: PlanState::Draft,
        approach: "Spike, then build.\n".into(),
        items: vec![PlanItem {
            id: slug("build"),
            title: "Build it".into(),
            criteria: vec!["it is built".into()],
            depends_on: vec![],
        }],
        team: None,
        proposed_by: ada(),
        edited_by: None,
        accepted: None,
        created: at(),
        updated: at(),
    })
    .expect("fixture plan is valid")
}

/// Run `number` of task `task`, of its plan's version 1, going.
pub fn run(task: &str, number: u32) -> Run {
    Run::new(RunFields {
        task: slug(task),
        number,
        plan_version: 1,
        started: at(),
        ended: None,
        outcome: None,
        cost: None,
        items: vec![],
    })
    .expect("fixture run is valid")
}

/// Change `number` of task `task`: branch `feat/<task>-<number>` of the
/// board's repository, no pull request yet.
pub fn change(task: &str, number: u32) -> Change {
    Change::new(ChangeFields {
        task: slug(task),
        number,
        head_repo: RepoName::parse("platform-q-ai/quecto").expect("fixture repo is valid"),
        branch: BranchName::parse(&format!("feat/{task}-{number}"))
            .expect("fixture branch is valid"),
        pr: None,
        merged: None,
        linked_by: ada(),
        unlinked: None,
        created: at(),
        updated: at(),
    })
    .expect("fixture change is valid")
}

/// The review of change `change` of task `task`: one round, approved.
pub fn review(task: &str, change: u32) -> Review {
    Review::new(ReviewFields {
        task: slug(task),
        change,
        rounds: vec![Round {
            commit: CommitId::parse(&"a".repeat(40)).expect("fixture commit is valid"),
            reviewer: ada(),
            at: at(),
            verdict: ReviewVerdict::Approved,
            findings: vec![],
        }],
        created: at(),
        updated: at(),
    })
    .expect("fixture review is valid")
}

/// The open discussion `id`, of one message, on no task.
pub fn discussion(id: &str) -> Discussion {
    Discussion::new(DiscussionFields {
        id: slug(id),
        title: "How the store locks".into(),
        task: None,
        state: DiscussionState::Open,
        started_by: ada(),
        messages: vec![Message {
            author: ada(),
            at: at(),
            text: "Should the store take a lease?".into(),
        }],
        created: at(),
        updated: at(),
    })
    .expect("fixture discussion is valid")
}
