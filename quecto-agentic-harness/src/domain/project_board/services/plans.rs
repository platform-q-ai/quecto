//! A task's plan versions (#2481's artifact model): anyone proposes a new
//! version as a draft and edits a draft; accepting one makes it the task's
//! plan and supersedes the version accepted before (an older draft too: a
//! rollback); a draft may be discarded and is kept. An accepted plan is
//! never edited: a change is a new version. Accepting or discarding is the
//! claim holder's on a task someone holds, and anyone's while it is
//! unclaimed; nothing is accepted while a run of the task is going, and a
//! done or archived task's plans are settled.
//!
//! The rules own the invariants across artifacts: they take the task's
//! plans (and its open run) as they are on the board, and refuse plans
//! that contradict each other rather than build on them.
use super::super::entities::claim::Identity;
use super::super::entities::plan::{MAX_PLAN_VERSION, Plan, PlanFields, PlanItem};
use super::super::entities::run::Run;
use super::super::entities::task::Task;
use super::super::entities::team::Team;
use super::super::value_objects::rule_error::BoardRuleError;
use super::super::value_objects::schema_error::SchemaError;
use super::super::value_objects::timestamp::Timestamp;
use super::super::value_objects::vocabulary::{PlanState, TaskStatus};
use super::claims::{held_live_by, settle, stamp};

/// What a plan says: its approach, items and team.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PlanContent {
    /// Markdown.
    pub approach: String,
    pub items: Vec<PlanItem>,
    pub team: Option<Team>,
}

/// A task with its plans and its run going (or last run), as the board
/// holds them: what an acceptance is judged against.
#[derive(Debug, Clone, Copy)]
pub struct TaskPlans<'a> {
    pub task: &'a Task,
    pub versions: &'a [Plan],
    pub open: Option<&'a Run>,
}

/// An acceptance: the task naming the version, that version accepted, and
/// the version it supersedes, if one was accepted before.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Accepted {
    pub task: Task,
    pub plan: Plan,
    pub superseded: Option<Plan>,
}

/// A new draft version of `task`'s plan, after every version in
/// `versions`, proposed by `actor` at `now`.
///
/// # Errors
/// A done or archived task; plans that contradict each other; no version
/// left; or a plan its schema refuses.
pub fn propose(
    task: &Task,
    versions: &[Plan],
    content: PlanContent,
    actor: &Identity,
    now: &Timestamp,
) -> Result<Plan, BoardRuleError> {
    open_to_plans(task)?;
    consistent(task, versions)?;
    let last = versions
        .iter()
        .map(|plan| plan.fields().version)
        .max()
        .unwrap_or(0);
    if last >= MAX_PLAN_VERSION {
        return Err(BoardRuleError::Invalid(SchemaError::new(
            "version",
            format!("the task has reached {MAX_PLAN_VERSION} plan versions"),
        )));
    }
    Plan::new(PlanFields {
        task: task.fields().id.clone(),
        version: last + 1,
        state: PlanState::Draft,
        approach: content.approach,
        items: content.items,
        team: content.team,
        proposed_by: actor.clone(),
        accepted: None,
        created: now.clone(),
        updated: now.clone(),
    })
    .map_err(BoardRuleError::Invalid)
}

/// The draft `plan` of `task` saying `content` instead, stamped `now`.
///
/// # Errors
/// A done or archived task; a plan of another task, or not a draft; an
/// edit that changes nothing; or a plan its schema refuses.
pub fn edit_draft(
    task: &Task,
    plan: &Plan,
    content: PlanContent,
    now: &Timestamp,
) -> Result<Plan, BoardRuleError> {
    open_to_plans(task)?;
    of_task(task, std::slice::from_ref(plan))?;
    draft(plan)?;
    let fields = plan.fields();
    let unchanged = fields.approach == content.approach
        && fields.items == content.items
        && fields.team == content.team;
    if unchanged {
        return Err(BoardRuleError::NoEdit);
    }
    let mut next = fields.clone();
    next.approach = content.approach;
    next.items = content.items;
    next.team = content.team;
    restamped(next, plan, now)
}

/// Version `version` of the `board` task's plan accepted by `actor` at
/// `now`: the task names it, and the version accepted before is superseded. With
/// `expected`, the draft must still be as the caller read it (its
/// `updated`), so the content accepted is the content reviewed.
///
/// # Errors
/// A done or archived task; plans that contradict each other; no version
/// of that number, or one that is not a draft, or changed since it was
/// read; an actor who may not decide for the task; or a run of the task
/// still going (`open`), or a run of another task.
pub fn accept(
    board: &TaskPlans<'_>,
    version: u32,
    expected: Option<&Timestamp>,
    actor: &Identity,
    now: &Timestamp,
) -> Result<Accepted, BoardRuleError> {
    let TaskPlans {
        task,
        versions,
        open,
    } = *board;
    open_to_plans(task)?;
    consistent(task, versions)?;
    let chosen = versions
        .iter()
        .find(|plan| plan.fields().version == version)
        .ok_or(BoardRuleError::UnknownPlanVersion(version))?;
    draft(chosen)?;
    if let Some(expected) = expected {
        let updated = &chosen.fields().updated;
        match updated == expected {
            true => {}
            false => {
                return Err(BoardRuleError::PlanChanged {
                    version,
                    updated: updated.clone(),
                });
            }
        }
    }
    decides(task, actor, now)?;
    if let Some(run) = open {
        match run.fields().task == task.fields().id {
            true => {}
            false => return Err(BoardRuleError::NotThisTasks),
        }
        if run.is_open() {
            return Err(BoardRuleError::RunOpen(run.fields().number));
        }
    }
    let at = stamp(task, now)?;
    let mut named = task.fields().clone();
    named.accepted_plan = Some(version);
    let task = settle(named, &at)?;
    let mut accepted = chosen.fields().clone();
    accepted.state = PlanState::Accepted;
    accepted.accepted = Some((actor.clone(), at.clone()));
    let plan = restamped(accepted, chosen, &at)?;
    let superseded = versions
        .iter()
        .find(|plan| plan.fields().state == PlanState::Accepted)
        .map(|before| moved(before, PlanState::Superseded, &at))
        .transpose()?;
    assert!(
        task.fields().accepted_plan == Some(plan.fields().version)
            && plan.fields().state == PlanState::Accepted
            && superseded
                .as_ref()
                .is_none_or(|before| before.fields().state == PlanState::Superseded),
        "an acceptance leaves the task naming its one accepted plan"
    );
    Ok(Accepted {
        task,
        plan,
        superseded,
    })
}

/// The draft `plan` of `task` discarded by `actor` at `now`; it is kept.
///
/// # Errors
/// A plan of another task, or not a draft; or an actor who may not decide
/// for the task.
pub fn discard(
    task: &Task,
    plan: &Plan,
    actor: &Identity,
    now: &Timestamp,
) -> Result<Plan, BoardRuleError> {
    open_to_plans(task)?;
    of_task(task, std::slice::from_ref(plan))?;
    draft(plan)?;
    decides(task, actor, now)?;
    moved(plan, PlanState::Discarded, now)
}

/// The task's plans as the board holds them agree with each other and
/// with the task: every one is the task's, no version is listed twice, at
/// most one is accepted, and it is the one the task names.
fn consistent(task: &Task, versions: &[Plan]) -> Result<(), BoardRuleError> {
    of_task(task, versions)?;
    let numbers: std::collections::BTreeSet<u32> =
        versions.iter().map(|plan| plan.fields().version).collect();
    if numbers.len() != versions.len() {
        return Err(BoardRuleError::PlansInconsistent(
            "a version is listed twice".into(),
        ));
    }
    let accepted: Vec<u32> = versions
        .iter()
        .filter(|plan| plan.fields().state == PlanState::Accepted)
        .map(|plan| plan.fields().version)
        .collect();
    match (accepted.as_slice(), task.fields().accepted_plan) {
        ([], None) => Ok(()),
        ([version], Some(named)) if *version == named => Ok(()),
        (accepted, named) => Err(BoardRuleError::PlansInconsistent(format!(
            "versions {accepted:?} are accepted; the task names {named:?}"
        ))),
    }
}

/// A done or archived task's plans are settled; any other's may change.
fn open_to_plans(task: &Task) -> Result<(), BoardRuleError> {
    match task.fields().status {
        TaskStatus::Draft
        | TaskStatus::Ready
        | TaskStatus::Claimed
        | TaskStatus::InProgress
        | TaskStatus::Review
        | TaskStatus::Blocked => Ok(()),
        status @ (TaskStatus::Done | TaskStatus::Archived) => {
            Err(BoardRuleError::TaskFinished(status))
        }
    }
}

fn of_task(task: &Task, versions: &[Plan]) -> Result<(), BoardRuleError> {
    match versions
        .iter()
        .all(|plan| plan.fields().task == task.fields().id)
    {
        true => Ok(()),
        false => Err(BoardRuleError::NotThisTasks),
    }
}

fn draft(plan: &Plan) -> Result<(), BoardRuleError> {
    match plan.fields().state {
        PlanState::Draft => Ok(()),
        state => Err(BoardRuleError::PlanNotDraft(state)),
    }
}

/// The holder of a live claim decides for a held task; anyone decides for
/// one nobody holds.
fn decides(task: &Task, actor: &Identity, now: &Timestamp) -> Result<(), BoardRuleError> {
    match task.fields().claim {
        Some(_) => held_live_by(task, actor, now).map(|_| ()),
        None => Ok(()),
    }
}

fn moved(plan: &Plan, state: PlanState, now: &Timestamp) -> Result<Plan, BoardRuleError> {
    let mut next = plan.fields().clone();
    next.state = state;
    restamped(next, plan, now)
}

/// `next`, updated at `now` or a second past `plan`'s own `updated`,
/// whichever is later, so every change moves it strictly forward.
fn restamped(mut next: PlanFields, plan: &Plan, now: &Timestamp) -> Result<Plan, BoardRuleError> {
    let after = plan
        .fields()
        .updated
        .plus_seconds(1)
        .map_err(|error| BoardRuleError::Invalid(error.at("updated")))?;
    next.updated = now.max(&after).clone();
    Plan::new(next).map_err(BoardRuleError::Invalid)
}

#[cfg(test)]
#[path = "plans_tests.rs"]
mod tests;
