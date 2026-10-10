//! Which status moves a task may make, and who may make them.
use super::super::entities::claim::Identity;
use super::super::entities::task::Task;
use super::super::value_objects::rule_error::BoardRuleError;
use super::super::value_objects::timestamp::Timestamp;
use super::super::value_objects::vocabulary::TaskStatus::{self, *};
use super::claims::{held_by, new_claim, settle};
use std::collections::BTreeSet;

/// Who may make a move.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Actor {
    /// Anyone with access to the project.
    Anyone,
    /// Only the current claim's holder.
    Holder,
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
/// other move, review to done when the task's pull requests merge, is the
/// system's ([`complete_merged`]).
#[rustfmt::skip] // One readable rule per row.
const TRANSITIONS: &[(TaskStatus, TaskStatus, Actor, ClaimEffect)] = &[
    (Draft,      Ready,      Actor::Anyone, ClaimEffect::Keep),
    (Blocked,    Ready,      Actor::Anyone, ClaimEffect::Clear),
    (Ready,      Claimed,    Actor::Anyone, ClaimEffect::Take),
    (Claimed,    InProgress, Actor::Holder, ClaimEffect::Keep),
    (InProgress, Review,     Actor::Holder, ClaimEffect::Keep),
    (InProgress, Blocked,    Actor::Holder, ClaimEffect::Keep),
    (Review,     Done,       Actor::Holder, ClaimEffect::Clear),
    (Review,     InProgress, Actor::Holder, ClaimEffect::Keep),
    (Done,       Archived,   Actor::Anyone, ClaimEffect::Keep),
    // Release: the holder hands the task back.
    (Claimed,    Ready,      Actor::Holder, ClaimEffect::Clear),
    (InProgress, Ready,      Actor::Holder, ClaimEffect::Clear),
];

/// `task` moved to `to` by `actor` at `now`, if the table allows it.
pub fn move_task(
    task: &Task,
    to: TaskStatus,
    actor: &Identity,
    now: &Timestamp,
) -> Result<Task, BoardRuleError> {
    let _ = (task, actor, now, TRANSITIONS, held_by, new_claim, settle);
    let _ = (
        Actor::Anyone,
        Actor::Holder,
        ClaimEffect::Keep,
        ClaimEffect::Take,
        ClaimEffect::Clear,
    );
    Err(BoardRuleError::NotAllowed { from: Draft, to })
}

/// The system move behind "a merged PR moves its task to done": a task in
/// review whose linked pull requests are all in `merged` is done, whoever
/// holds it and whoever merged them.
pub fn complete_merged(
    task: &Task,
    merged: &BTreeSet<u64>,
    now: &Timestamp,
) -> Result<Task, BoardRuleError> {
    let _ = (task, merged, now);
    Err(BoardRuleError::NoPrs)
}

#[cfg(test)]
#[path = "transitions_tests.rs"]
mod tests;
