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
use super::super::entities::plan::{MAX_PLAN_VERSION, Plan, PlanEntry, PlanFields, PlanItem};
use super::super::entities::run::Run;
use super::super::entities::task::Task;
use super::super::entities::team::Team;
use super::super::value_objects::board_text::{entered_line, entered_text};
use super::super::value_objects::rule_error::BoardRuleError;
use super::super::value_objects::schema_error::SchemaError;
use super::super::value_objects::timestamp::Timestamp;
use super::super::value_objects::vocabulary::{PlanState, TaskStatus};
use super::claims::{after_last, held_live_by, not_before, settle, stamp, within_slack};
use super::runs::open_of;

/// What a plan says: its approach, handoff, task items and team; and how
/// its approach enters (#2705 T3).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PlanContent {
    /// Markdown.
    pub approach: String,
    /// Markdown; `None` for none.
    pub handoff: Option<String>,
    pub items: Vec<PlanItem>,
    pub team: Option<Team>,
    /// How a new approach enters: one an edit leaves as the draft holds
    /// it enters nothing.
    pub entry: PlanEntry,
}

/// When a change happens: the caller's `now`, and the latest time it may
/// build on, when an outside reference bounds the clock (a plan stamped
/// past it, plus the stamp's slack, came from a clock running ahead).
/// `latest: None` means there is no reference, and nothing is refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Moment {
    pub now: Timestamp,
    pub latest: Option<Timestamp>,
}

impl Moment {
    /// `plan` is no later than this moment may build on.
    ///
    /// # Errors
    /// A plan stamped past the latest time plus the stamp's slack.
    fn may_build_on(&self, plan: &Plan) -> Result<(), BoardRuleError> {
        let updated = &plan.fields().updated;
        match within_slack(updated, self.latest.as_ref()) {
            true => Ok(()),
            false => Err(BoardRuleError::PlanAhead {
                version: plan.fields().version,
                updated: updated.clone(),
            }),
        }
    }
}

/// A task with all its plans and runs, as the board holds them: what an
/// acceptance is judged against.
#[derive(Debug, Clone, Copy)]
pub struct TaskPlans<'a> {
    pub task: &'a Task,
    pub versions: &'a [Plan],
    pub runs: &'a [Run],
}

/// A draft of a task's plan as the board holds it (#2662): the task,
/// every version of its plan (the draft among them), and the draft.
#[derive(Debug, Clone, Copy)]
pub struct DraftOf<'a> {
    pub task: &'a Task,
    pub versions: &'a [Plan],
    pub plan: &'a Plan,
}

/// What an acceptance's caller read (#2481 M4c): the draft's `updated`,
/// so the content accepted is the content reviewed, and the version the
/// task named as accepted (`None`: none), so no acceptance the caller did
/// not see is superseded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AsRead<'a> {
    pub updated: &'a Timestamp,
    pub accepted: Option<u32>,
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
/// `versions`, proposed by `actor` at `when`: stamped no earlier than the
/// task or any of its plans within `when`'s bound (#2662).
///
/// # Errors
/// A done or archived task; plans that contradict each other; no version
/// left; or a plan its schema refuses.
pub fn propose_at(
    task: &Task,
    versions: &[Plan],
    content: PlanContent,
    actor: &Identity,
    when: &Moment,
) -> Result<Plan, BoardRuleError> {
    open_to_plans(task)?;
    consistent(task, versions)?;
    let now = &not_before(task, stamps_of(versions, when), &when.now);
    let last = versions
        .iter()
        .map(|plan| plan.fields().version)
        .max()
        .unwrap_or(0);
    entered(&content, None)?;
    match last < MAX_PLAN_VERSION {
        true => {}
        false => {
            return Err(BoardRuleError::InvalidPlan(SchemaError::new(
                "version",
                format!("the task has reached {MAX_PLAN_VERSION} plan versions"),
            )));
        }
    }
    Plan::new(PlanFields {
        task: task.fields().id.clone(),
        version: last + 1,
        state: PlanState::Draft,
        approach: content.approach,
        handoff: content.handoff,
        items: content.items,
        team: content.team,
        proposed_by: actor.clone(),
        edited_by: None,
        accepted: None,
        created: now.clone(),
        updated: now.clone(),
    })
    .map_err(BoardRuleError::InvalidPlan)
}

/// The draft `plan` of `task` (`of`, with every version of its plan)
/// saying `content` instead, edited by `actor` (anyone: drafts are shared)
/// at `when`, stamped no earlier than any of the task's plans (#2662).
/// With `expected`, the draft must still be as the caller read it (its
/// `updated`), so a newer edit is never overwritten.
///
/// # Errors
/// A done or archived task; a plan of another task, or not a draft, or
/// changed since it was read, or stamped ahead of `when`'s bound; an edit
/// that changes nothing; or a plan its schema refuses.
pub fn edit_draft(
    of: DraftOf<'_>,
    content: PlanContent,
    expected: Option<&Timestamp>,
    actor: &Identity,
    when: &Moment,
) -> Result<Plan, BoardRuleError> {
    let DraftOf {
        task,
        versions,
        plan,
    } = of;
    assert!(
        versions.contains(plan),
        "a draft is one of the task's plans as the board holds them"
    );
    let now = &when.now;
    open_to_plans(task)?;
    of_task(task, std::slice::from_ref(plan))?;
    of_task(task, versions)?;
    draft(plan)?;
    as_read(plan, expected)?;
    when.may_build_on(plan)?;
    let fields = plan.fields();
    entered(&content, Some(&fields.approach))?;
    let changes = fields.approach != content.approach
        || fields.handoff != content.handoff
        || fields.items != content.items
        || fields.team != content.team;
    match changes {
        true => {}
        false => return Err(BoardRuleError::NoEdit),
    }
    let mut next = fields.clone();
    next.approach = content.approach;
    next.handoff = content.handoff;
    next.items = content.items;
    next.team = content.team;
    // Drafts are anyone's: the last editor is recorded, the proposer kept.
    next.edited_by = Some(actor.clone());
    restamped(
        next,
        plan,
        &not_before(task, stamps_of(versions, when), now),
    )
}

/// `content`'s text is as it enters the board, normalised: its approach
/// and handoff as markdown, each task item's title and criteria as lines;
/// and its approach within its entry's cap (#2705 T3) unless it is the
/// approach `held` (an edit's draft holds it already: a legacy approach
/// is kept, never forced to be rewritten).
fn entered(content: &PlanContent, held: Option<&str>) -> Result<(), BoardRuleError> {
    let invalid = BoardRuleError::InvalidPlan;
    entered_text("approach", &content.approach).map_err(invalid)?;
    if held != Some(content.approach.as_str()) {
        content.entry.approach(&content.approach).map_err(invalid)?;
    }
    if let Some(handoff) = &content.handoff {
        entered_text("handoff", handoff).map_err(invalid)?;
    }
    for (index, item) in content.items.iter().enumerate() {
        entered_line(&format!("items/{index}/title"), &item.title).map_err(invalid)?;
        for (at, criterion) in item.criteria.iter().enumerate() {
            entered_line(&format!("items/{index}/criteria/{at}"), criterion).map_err(invalid)?;
        }
    }
    Ok(())
}

/// Version `version` of the `board` task's plan accepted by `actor` at
/// `now`: the task names it, and the version accepted before is
/// superseded. With `expected`, the draft must still be as the caller read
/// it (its `updated`), so the content accepted is the content reviewed,
/// and the task must still name the accepted version the caller saw.
/// The task, both plans and the acceptance share one stamp, later than
/// each one's last change (a proposer's clock may have run ahead).
///
/// # Errors
/// A done or archived task; plans or runs that contradict each other; no
/// version of that number, or one that is not a draft, or changed since it
/// was read; another accepted version than the caller saw; a plan stamped
/// ahead of `when`'s bound; an actor who may not
/// decide for the task; a run of the task still going, or a run of another
/// task.
pub fn accept(
    board: &TaskPlans<'_>,
    version: u32,
    expected: Option<AsRead<'_>>,
    actor: &Identity,
    when: &Moment,
) -> Result<Accepted, BoardRuleError> {
    let now = &when.now;
    let TaskPlans {
        task,
        versions,
        runs,
    } = *board;
    open_to_plans(task)?;
    consistent(task, versions)?;
    let chosen = versions
        .iter()
        .find(|plan| plan.fields().version == version)
        .ok_or(BoardRuleError::UnknownPlanVersion(version))?;
    draft(chosen)?;
    if let Some(read) = expected {
        as_read(chosen, Some(read.updated))?;
        as_accepted(task, read.accepted)?;
    }
    decides(task, actor, now)?;
    match open_of(task, runs)? {
        None => {}
        Some(run) => return Err(BoardRuleError::RunOpen(run.fields().number)),
    }
    let before = versions
        .iter()
        .find(|plan| plan.fields().state == PlanState::Accepted);
    // Neither plan the acceptance stamps past may drag the task ahead.
    for plan in std::iter::once(chosen).chain(before) {
        when.may_build_on(plan)?;
    }
    // One stamp, after the task's, the chosen plan's and the superseded
    // plan's last changes.
    let mut at = stamp(task, now)?;
    for plan in std::iter::once(chosen).chain(before) {
        at = after_last(&plan.fields().updated, &at)?;
    }
    // Never before any of the task's plans within the bound (#2662).
    at = not_before(task, stamps_of(versions, when), &at);
    let mut named = task.fields().clone();
    named.accepted_plan = Some(version);
    let task = settle(named, &at)?;
    let mut accepted = chosen.fields().clone();
    accepted.state = PlanState::Accepted;
    accepted.accepted = Some((actor.clone(), at.clone()));
    accepted.updated = at.clone();
    let plan = Plan::new(accepted).map_err(BoardRuleError::InvalidPlan)?;
    let superseded = before
        .map(|before| {
            let mut next = before.fields().clone();
            next.state = PlanState::Superseded;
            next.updated = at.clone();
            Plan::new(next).map_err(BoardRuleError::InvalidPlan)
        })
        .transpose()?;
    let result: Vec<&Plan> = versions
        .iter()
        .filter(|plan| plan.fields().version != version)
        .filter(|plan| plan.fields().state != PlanState::Accepted)
        .chain(std::iter::once(&plan))
        .chain(superseded.as_ref())
        .collect();
    let accepted_now: Vec<u32> = result
        .iter()
        .filter(|plan| plan.fields().state == PlanState::Accepted)
        .map(|plan| plan.fields().version)
        .collect();
    assert_eq!(
        (accepted_now, task.fields().accepted_plan),
        (vec![version], Some(version)),
        "an acceptance leaves the task naming its one accepted plan"
    );
    Ok(Accepted {
        task,
        plan,
        superseded,
    })
}

/// The draft `plan` of `task` (`of`, with every version of its plan)
/// discarded by `actor` at `when`, stamped no earlier than any of the
/// task's plans (#2662); it is kept, as discarded.
/// With `expected`, the draft must still be as the caller read it (its
/// `updated`), so the draft discarded is the one reviewed (#2481 M4c).
///
/// # Errors
/// A done or archived task; a plan of another task, or not a draft, or
/// changed since it was read, or stamped ahead of `when`'s bound; or an
/// actor who may not decide for the task.
pub fn discard(
    of: DraftOf<'_>,
    expected: Option<&Timestamp>,
    actor: &Identity,
    when: &Moment,
) -> Result<Plan, BoardRuleError> {
    let DraftOf {
        task,
        versions,
        plan,
    } = of;
    assert!(
        versions.contains(plan),
        "a draft is one of the task's plans as the board holds them"
    );
    let now = &when.now;
    open_to_plans(task)?;
    of_task(task, std::slice::from_ref(plan))?;
    of_task(task, versions)?;
    draft(plan)?;
    as_read(plan, expected)?;
    when.may_build_on(plan)?;
    decides(task, actor, now)?;
    moved(
        plan,
        PlanState::Discarded,
        &not_before(task, stamps_of(versions, when), now),
    )
}

/// The task's plans as the board holds them agree with each other and
/// with the task: every one is the task's, no version is listed twice, at
/// most one is accepted, and it is the one the task names.
fn consistent(task: &Task, versions: &[Plan]) -> Result<(), BoardRuleError> {
    of_task(task, versions)?;
    let numbers: std::collections::BTreeSet<u32> =
        versions.iter().map(|plan| plan.fields().version).collect();
    match numbers.len() == versions.len() {
        true => {}
        false => {
            return Err(BoardRuleError::PlansInconsistent(
                "a version is listed twice".into(),
            ));
        }
    }
    let accepted: Vec<u32> = versions
        .iter()
        .filter(|plan| plan.fields().state == PlanState::Accepted)
        .map(|plan| plan.fields().version)
        .collect();
    match (accepted.as_slice(), task.fields().accepted_plan) {
        ([], None) => Ok(()),
        ([version], Some(named)) if *version == named => Ok(()),
        (accepted, named) => {
            let listed: Vec<String> = accepted.iter().map(u32::to_string).collect();
            let accepted = match listed.as_slice() {
                [] => "no version is accepted".to_string(),
                [one] => format!("version {one} is accepted"),
                [..] => format!("versions {} are accepted", listed.join(", ")),
            };
            let named = match named {
                Some(version) => format!("the task names version {version}"),
                None => "the task names none".to_string(),
            };
            Err(BoardRuleError::PlansInconsistent(format!(
                "{accepted}; {named}"
            )))
        }
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

/// The task's accepted plan as the caller saw it (#2481 M4c review round
/// 1): the version it named then (`None`: it named none) is the one it
/// names now, so an acceptance never supersedes one its caller did not
/// see, and a rollback is a knowing choice.
///
/// # Errors
/// The task names another version, or none, or one where it named none.
pub fn as_accepted(task: &Task, expected: Option<u32>) -> Result<(), BoardRuleError> {
    let accepted = task.fields().accepted_plan;
    match accepted == expected {
        true => Ok(()),
        false => Err(BoardRuleError::AcceptedPlanChanged { accepted }),
    }
}

/// `plan` as the caller read it: unchanged since `expected`, when given.
fn as_read(plan: &Plan, expected: Option<&Timestamp>) -> Result<(), BoardRuleError> {
    let updated = &plan.fields().updated;
    match expected {
        None => Ok(()),
        Some(expected) if expected == updated => Ok(()),
        Some(_) => Err(BoardRuleError::PlanChanged {
            version: plan.fields().version,
            updated: updated.clone(),
        }),
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

/// When each of `versions` (the task's plans, as the board holds them)
/// last changed, within `when`'s bound: what a plan's change is stamped no
/// earlier than. A plan stamped past the bound (a clock that ran ahead, or
/// a hand edit) is left out, so it never drags a new stamp ahead of it.
fn stamps_of<'a>(versions: &'a [Plan], when: &'a Moment) -> impl Iterator<Item = &'a Timestamp> {
    versions
        .iter()
        .map(|plan| &plan.fields().updated)
        .filter(|updated| within_slack(updated, when.latest.as_ref()))
}

fn moved(plan: &Plan, state: PlanState, now: &Timestamp) -> Result<Plan, BoardRuleError> {
    let mut next = plan.fields().clone();
    next.state = state;
    restamped(next, plan, now)
}

/// `next`, updated at `now` or a second past `plan`'s own `updated`,
/// whichever is later, so every change moves it strictly forward.
fn restamped(mut next: PlanFields, plan: &Plan, now: &Timestamp) -> Result<Plan, BoardRuleError> {
    next.updated = after_last(&plan.fields().updated, now)?;
    Plan::new(next).map_err(BoardRuleError::InvalidPlan)
}

#[cfg(test)]
#[path = "plans_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "plans_limits_tests.rs"]
mod limits_tests;

#[cfg(test)]
#[path = "plans_discard_tests.rs"]
mod discard_tests;

#[cfg(test)]
#[path = "plans_pin_tests.rs"]
mod pin_tests;
