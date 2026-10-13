//! Starting a task (S4, #2479): the one change that hands a task to a
//! swarm. A ready task is claimed by its starter; a task the starter holds
//! live is worked on as it is. Either way it moves to in progress and a run
//! of its accepted plan begins, all at one stamp, so the task and the run
//! are written by one commit.
//!
//! The transition table stays the authority on who may move a task: a start
//! asks it about each move it makes ([`move_reads`]) and composes them, as
//! two moves in a row would stamp the task twice.
use super::super::entities::claim::Identity;
use super::super::entities::plan::Plan;
use super::super::entities::run::Run;
use super::super::entities::task::Task;
use super::super::value_objects::rule_error::BoardRuleError;
use super::super::value_objects::timestamp::Timestamp;
use super::super::value_objects::vocabulary::TaskStatus::{self, *};
use super::claims::{held_live_by, new_claim, settle, stamp};
use super::runs::begin_run;
use super::transitions::move_reads;

/// A task started: in progress, held by its starter, and its run begun.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Started {
    pub task: Task,
    pub run: Run,
    /// Whether the start took the claim (the task was ready), so a start
    /// whose swarm never launched knows to hand the task back.
    pub took_claim: bool,
}

/// `task` started by `actor` at `now` on `plan`, its accepted plan (`None`
/// when the board holds no file of that version), after every run in
/// `runs`: a ready task is claimed by `actor`; a claimed, in-review or
/// blocked one that `actor` holds live moves to in progress; an in-progress
/// one stays. The task's change and the run share one stamp.
///
/// # Errors
/// A draft, done or archived task (`not_allowed`); a claim `actor` does not
/// hold live; no accepted plan, or its file missing, or another plan given;
/// a run still going, or runs that contradict each other; a stamp or a run
/// number the board cannot spell.
pub fn start(
    task: &Task,
    runs: &[Run],
    plan: Option<&Plan>,
    actor: &Identity,
    now: &Timestamp,
) -> Result<Started, BoardRuleError> {
    let from = task.fields().status;
    let took_claim = match from {
        Ready => {
            move_reads(task, Claimed, actor, now)?;
            true
        }
        Claimed | Review | Blocked => {
            move_reads(task, InProgress, actor, now)?;
            false
        }
        InProgress => {
            held_live_by(task, actor, now)?;
            false
        }
        Draft | Done | Archived => {
            return Err(BoardRuleError::NotAllowed {
                from,
                to: InProgress,
            });
        }
    };
    let plan = match (task.fields().accepted_plan, plan) {
        (None, _) => return Err(BoardRuleError::NoAcceptedPlan),
        (Some(version), None) => return Err(BoardRuleError::UnknownPlanVersion(version)),
        (Some(_), Some(plan)) => plan,
    };
    let at = stamp(task, now)?;
    let mut next = task.fields().clone();
    next.status = TaskStatus::InProgress;
    if took_claim {
        next.claim = Some(new_claim(actor, &at)?);
    }
    let started = settle(next, &at)?;
    let run = begin_run(&started, runs, plan, actor, &at)?;
    assert!(
        started.fields().updated == at && run.fields().started == at,
        "the task's change and its run share one stamp"
    );
    assert!(
        started
            .fields()
            .claim
            .as_ref()
            .is_some_and(|claim| claim.holder.is(actor)),
        "a started task is held by its starter"
    );
    Ok(Started {
        task: started,
        run,
        took_claim,
    })
}

#[cfg(test)]
#[path = "starts_tests.rs"]
mod tests;
