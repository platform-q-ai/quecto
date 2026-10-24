//! Starting a task (S4, #2479): the one change that hands a task to a
//! swarm. A ready task is claimed by its starter; a task the starter holds
//! has its claim renewed. Either way it moves to in progress and a run of
//! its accepted plan begins, all at one stamp, so the task and the run are
//! written by one commit, and the run begins with a whole claim window.
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
use super::claims::{new_claim, renew, settle, stamp};
use super::runs::begin_run;
use super::transitions::{move_reads, relations_allow};

/// A task started: in progress, held by its starter, and a run of its
/// accepted plan begun. Only [`start`] builds one, so holding one proves
/// its plan is the task's accepted plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Started {
    task: Task,
    run: Run,
    plan: Plan,
    from: TaskStatus,
}

impl Started {
    /// The task as the start wrote it.
    pub fn task(&self) -> &Task {
        &self.task
    }

    /// The run the start began.
    pub fn run(&self) -> &Run {
        &self.run
    }

    /// The accepted plan the run runs.
    pub fn plan(&self) -> &Plan {
        &self.plan
    }

    /// The status it started from, for the caller's compensation.
    pub fn from(&self) -> TaskStatus {
        self.from
    }

    /// Whether the start took the claim: the task was ready.
    pub fn took_claim(&self) -> bool {
        self.from == Ready
    }

    /// The task and the run, to write in one commit.
    pub fn into_writes(self) -> (Task, Run) {
        (self.task, self.run)
    }
}

/// `task` started by `actor` at `now` on `plan`, its accepted plan (`None`
/// when the board holds no file of that version), after every run in
/// `runs`: a ready task is claimed by `actor`; a claimed, in-progress,
/// in-review or blocked one that `actor` holds has its claim renewed (an
/// expired one too, as [`renew`] allows the holder) and moves to, or stays
/// in, progress. The task's change, its claim and the run share one stamp,
/// at which the claim's liveness is judged.
///
/// # Errors
/// A draft, done or archived task (`not_allowed`); a ready task a task it
/// depends on (of `tasks`, every task the board holds) holds back
/// (`dependency_not_done`); a claim `actor` does not
/// hold, live or not (`not_holder`, `no_claim`); no accepted plan, or its
/// file missing, or another plan given; a run still going, or runs that
/// contradict each other; a stamp or a run number the board cannot spell.
pub fn start(
    task: &Task,
    tasks: &[Task],
    runs: &[Run],
    plan: Option<&Plan>,
    actor: &Identity,
    now: &Timestamp,
) -> Result<Started, BoardRuleError> {
    let from = task.fields().status;
    let at = stamp(task, now)?;
    // The task's fields once held by `actor` at the stamp: a new claim on
    // a ready task (set with the move, as a ready task holds none), or
    // the holder's renewed one.
    let mut next = match from {
        Ready => {
            move_reads(task, Claimed, actor, &at)?;
            // Claimed as a move to claimed is: once its dependencies are done.
            relations_allow(task, Claimed, tasks)?;
            let mut next = task.fields().clone();
            next.claim = Some(new_claim(actor, &at)?);
            next
        }
        Claimed | Review | Blocked => {
            let renewed = renew(task, actor, now)?;
            // Guards the table's row: a held status moves to in progress
            // only by its holder's row, never by this rule's say-so.
            move_reads(&renewed, InProgress, actor, &at)?;
            renewed.into_fields()
        }
        InProgress => renew(task, actor, now)?.into_fields(),
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
    next.status = TaskStatus::InProgress;
    let started = settle(next, &at)?;
    let run = begin_run(&started, runs, plan, actor, &at)?;
    let fields = started.fields();
    assert!(
        fields.updated == at && run.fields().started == at,
        "the task's change and its run share one stamp"
    );
    assert!(
        fields
            .claim
            .as_ref()
            .is_some_and(|claim| claim.holder.is(actor) && claim.since == at),
        "a started task is held by its starter, its claim taken or renewed at the stamp"
    );
    Ok(Started {
        task: started,
        run,
        plan: plan.clone(),
        from,
    })
}

/// A start undone (#2705 T4): `task`, in progress, back in the review or
/// blocked status its start moved it from, by `actor` at `now`.
pub fn unstart(
    task: &Task,
    from: TaskStatus,
    runs: &[Run],
    actor: &Identity,
    now: &Timestamp,
) -> Result<Task, BoardRuleError> {
    let _ = (runs, actor, now);
    Err(BoardRuleError::NotAllowed {
        from: task.fields().status,
        to: from,
    })
}

#[cfg(test)]
#[path = "starts_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "starts_undo_tests.rs"]
mod undo_tests;
