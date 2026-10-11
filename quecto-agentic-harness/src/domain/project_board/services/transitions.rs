//! Which status moves a task may make, and who may make them.
use super::super::entities::claim::Identity;
use super::super::entities::task::Task;
use super::super::value_objects::rule_error::BoardRuleError;
use super::super::value_objects::timestamp::Timestamp;
use super::super::value_objects::vocabulary::TaskStatus::{self, *};
use super::claims::{clock_keeps_up, held_by, held_live_by, is_live, new_claim, settle};
use std::collections::BTreeSet;

/// Who may make a move.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Actor {
    /// Anyone with access to the project.
    Anyone,
    /// Only the holder, while the claim is live (renew an expired one).
    Holder,
    /// Only the holder, live or expired: handing the task back.
    Releaser,
    /// The holder while the claim is live; anyone once it is absent or
    /// expired, so a stalled review or block can be put back.
    HolderWhileLive,
}

/// What a move does to the claim.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ClaimEffect {
    Keep,
    /// The mover takes a new two-hour claim.
    Take,
    Clear,
}

/// Every move a person may make; any pair not listed is refused. The one
/// other move, to done when the task's pull requests merge, is the
/// system's ([`complete_merged`]).
#[rustfmt::skip] // One readable rule per row.
const TRANSITIONS: &[(TaskStatus, TaskStatus, Actor, ClaimEffect)] = &[
    (Draft,      Ready,      Actor::Anyone,          ClaimEffect::Keep),
    (Draft,      Archived,   Actor::Anyone,          ClaimEffect::Keep),
    (Ready,      Draft,      Actor::Anyone,          ClaimEffect::Keep),
    (Ready,      Claimed,    Actor::Anyone,          ClaimEffect::Take),
    (Ready,      Archived,   Actor::Anyone,          ClaimEffect::Keep),
    (Claimed,    InProgress, Actor::Holder,          ClaimEffect::Keep),
    (Claimed,    Blocked,    Actor::Holder,          ClaimEffect::Keep),
    (InProgress, Review,     Actor::Holder,          ClaimEffect::Keep),
    (InProgress, Blocked,    Actor::Holder,          ClaimEffect::Keep),
    (Review,     Done,       Actor::Holder,          ClaimEffect::Clear),
    (Review,     InProgress, Actor::Holder,          ClaimEffect::Keep),
    (Blocked,    InProgress, Actor::Holder,          ClaimEffect::Keep),
    (Done,       Archived,   Actor::Anyone,          ClaimEffect::Keep),
    // Release: the task goes back to the board.
    (Claimed,    Ready,      Actor::Releaser,        ClaimEffect::Clear),
    (InProgress, Ready,      Actor::Releaser,        ClaimEffect::Clear),
    (Review,     Ready,      Actor::HolderWhileLive, ClaimEffect::Clear),
    (Blocked,    Ready,      Actor::HolderWhileLive, ClaimEffect::Clear),
];

/// `task` moved to `to` by `actor` at `now`, if the table allows it.
pub fn move_task(
    task: &Task,
    to: TaskStatus,
    actor: &Identity,
    now: &Timestamp,
) -> Result<Task, BoardRuleError> {
    clock_keeps_up(task, now)?;
    let from = task.fields().status;
    let (_, _, who, effect) = TRANSITIONS
        .iter()
        .find(|(row_from, row_to, _, _)| (*row_from, *row_to) == (from, to))
        .ok_or(BoardRuleError::NotAllowed { from, to })?;
    match who {
        Actor::Anyone => {}
        Actor::Holder => {
            held_live_by(task, actor, now)?;
        }
        Actor::Releaser => {
            held_by(task, actor)?;
        }
        Actor::HolderWhileLive => {
            if task
                .fields()
                .claim
                .as_ref()
                .is_some_and(|claim| is_live(claim, now))
            {
                held_by(task, actor)?;
            }
        }
    }
    runs_may_stay_open(task, to)?;
    let mut next = task.fields().clone();
    next.status = to;
    next.claim = match effect {
        ClaimEffect::Keep => next.claim,
        ClaimEffect::Take => Some(new_claim(actor, now)?),
        ClaimEffect::Clear => None,
    };
    settle(next, now)
}

/// The system move behind "a merged PR moves its task to done": a held
/// task (claimed, in progress, in review or blocked) whose linked pull
/// requests are all in `merged` is done, whoever holds it and whoever
/// merged them.
pub fn complete_merged(
    task: &Task,
    merged: &BTreeSet<u64>,
    now: &Timestamp,
) -> Result<Task, BoardRuleError> {
    clock_keeps_up(task, now)?;
    let fields = task.fields();
    let unmerged: Vec<u64> = fields
        .prs
        .iter()
        .copied()
        .filter(|pr| !merged.contains(pr))
        .collect();
    match (
        fields.status.may_hold_claim(),
        &fields.prs[..],
        &unmerged[..],
    ) {
        (true, [_, ..], []) => {}
        (true, [], _) => return Err(BoardRuleError::NoPrs),
        (true, _, open) => return Err(BoardRuleError::PrsOpen(open.to_vec())),
        (false, _, _) => {
            return Err(BoardRuleError::NotAllowed {
                from: fields.status,
                to: Done,
            });
        }
    }
    runs_may_stay_open(task, Done)?;
    let mut next = fields.clone();
    next.status = Done;
    next.claim = None;
    settle(next, now)
}

/// A run may stay open only while the task may hold a claim; leaving for
/// ready, done or archived waits until it ends.
fn runs_may_stay_open(task: &Task, to: TaskStatus) -> Result<(), BoardRuleError> {
    match task.fields().runs.iter().find(|run| run.is_open()) {
        None => Ok(()),
        Some(_) if to.may_hold_claim() => Ok(()),
        Some(run) => Err(BoardRuleError::RunOpen(run.id.clone())),
    }
}

#[cfg(test)]
#[path = "transitions_tests.rs"]
mod tests;
