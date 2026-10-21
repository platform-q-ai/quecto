//! Which status moves a task may make, and who may make them.
use super::super::entities::change::Change;
use super::super::entities::claim::Identity;
use super::super::entities::run::Run;
use super::super::entities::task::Task;
use super::super::value_objects::rule_error::BoardRuleError;
use super::super::value_objects::slug::Slug;
use super::super::value_objects::timestamp::Timestamp;
use super::super::value_objects::vocabulary::Resolution;
use super::super::value_objects::vocabulary::TaskStatus::{self, *};
use super::claims::{held_by, held_live_by, is_live, new_claim, settle, stamp};
use super::runs::{holds_moves_to, lost_at, may_move_with, open_of};
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
    /// expired, so a stalled review or block can be put back, or a
    /// stalled review archived.
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
    // Archive a reviewed task: its holder's while the claim is live,
    // anyone's once nobody holds it. A run going holds it back
    // (`may_move_with`), so it never races a live run; a pull request or
    // change linked holds it as it holds review to done
    // ([`held_by_links`], as it holds a draft's or a ready one's
    // archive); the claim goes with it.
    (Review,     Archived,   Actor::HolderWhileLive, ClaimEffect::Clear),
    // Release: the task goes back to the board.
    (Claimed,    Ready,      Actor::Releaser,        ClaimEffect::Clear),
    (InProgress, Ready,      Actor::Releaser,        ClaimEffect::Clear),
    (Review,     Ready,      Actor::HolderWhileLive, ClaimEffect::Clear),
    (Blocked,    Ready,      Actor::HolderWhileLive, ClaimEffect::Clear),
];

/// What a move reads of the task's work: its runs, its changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MoveReads {
    pub runs: bool,
    pub changes: bool,
}

/// Whether the task's pull requests and changes linked hold the move
/// `from` -> `to` (allowlist): review to done (they finish it when they
/// merge), and draft, ready or review to archived, so a task released with
/// its links (review to ready, on to draft) is not archived with them
/// either. Only a done task is archived with what it links: merged, as
/// done needs.
fn held_by_links(from: TaskStatus, to: TaskStatus) -> bool {
    match (from, to) {
        (Review, Done) | (Draft | Ready | Review, Archived) => true,
        (Draft | Ready | Claimed | InProgress | Review | Blocked | Done | Archived, _) => false,
    }
}

/// What moving `task` to `to` by `actor` at `now` reads of the task's
/// work, once the table, the actor and the task's own fields allow it: a
/// move a run going holds back ([`holds_moves_to`]) reads the runs, a
/// move links hold ([`held_by_links`]) reads the changes (a change linked
/// holds it), any other reads nothing. A use case reads only these, so a move the
/// rule refuses anyway is refused as such, and reads nothing.
///
/// # Errors
/// What [`move_task`] refuses before it looks at runs or changes.
pub fn move_reads(
    task: &Task,
    to: TaskStatus,
    actor: &Identity,
    now: &Timestamp,
) -> Result<MoveReads, BoardRuleError> {
    allowed(task, to, actor, now)?;
    let from = task.fields().status;
    Ok(MoveReads {
        runs: holds_moves_to(to),
        changes: held_by_links(from, to),
    })
}

/// `task` moved to `to` by `actor` at `now`, if the table allows it, its
/// `runs` and `changes` (all of them, as the board holds them) let it
/// leave, and the `tasks` it relates to (every task the board holds) let
/// it move ([`relations_allow`], #2661). Only the runs and changes
/// [`move_reads`] names count: a caller may leave the rest empty.
pub fn move_task(
    task: &Task,
    to: TaskStatus,
    actor: &Identity,
    (runs, changes): (&[Run], &[Change]),
    tasks: &[Task],
    now: &Timestamp,
) -> Result<Task, BoardRuleError> {
    let effect = allowed(task, to, actor, now)?;
    let from = task.fields().status;
    let linked = numbers(task, changes, Change::is_linked)?;
    // Only review to done, and draft, ready or review to archived, are
    // held by a change linked.
    if held_by_links(from, to) && !linked.is_empty() {
        return Err(BoardRuleError::ChangesLinked(linked));
    }
    may_move_with(task, runs, to)?;
    relations_allow(task, to, tasks)?;
    let at = stamp(task, now)?;
    let mut next = task.fields().clone();
    next.status = to;
    next.resolution = resolved(to, next.resolution, !next.superseded_by.is_empty());
    next.claim = match effect {
        ClaimEffect::Keep => next.claim,
        ClaimEffect::Take => Some(new_claim(actor, &at)?),
        ClaimEffect::Clear => None,
    };
    assert!(
        next.claim.is_none() || to.may_hold_claim(),
        "{from:?} -> {to:?}: a task leaves its claim behind for a status that holds none"
    );
    settle(next, &at)
}

/// Whether the tasks `task` relates to on the board (`tasks`: every task
/// the board holds) let it move to `to` (#2661), an allowlist of the moves
/// they hold back:
/// - a claim waits for every task it depends on to release its
///   dependents ([`TaskStatus::releases_dependents`]: done or archived,
///   as `list_ready` lists it); one not on the board holds it too;
/// - an archive waits for every child task to be free of work: none
///   claimed, in progress, in review or blocked.
///
/// A start claims a ready task as a move to claimed does, so it asks the
/// same ([`super::starts::start`]).
///
/// # Errors
/// The tasks that hold it back, by id.
pub fn relations_allow(task: &Task, to: TaskStatus, tasks: &[Task]) -> Result<(), BoardRuleError> {
    let fields = task.fields();
    let status_of = |id: &Slug| {
        tasks
            .iter()
            .find(|other| other.fields().id == *id)
            .map(|other| other.fields().status)
    };
    match to {
        Claimed => {
            let (mut undone, mut missing) = (vec![], vec![]);
            for id in &fields.depends_on {
                match status_of(id) {
                    Some(status) if status.releases_dependents() => {}
                    Some(_) => undone.push(id.clone()),
                    None => missing.push(id.clone()),
                }
            }
            match (undone.as_slice(), missing.as_slice()) {
                ([], []) => Ok(()),
                (_, _) => Err(BoardRuleError::DependencyNotDone { undone, missing }),
            }
        }
        Archived => {
            let active: Vec<Slug> = tasks
                .iter()
                .filter(|other| other.fields().parent.as_ref() == Some(&fields.id))
                .filter(|child| match child.fields().status {
                    Claimed | InProgress | Review | Blocked => true,
                    Draft | Ready | Done | Archived => false,
                })
                .map(|child| child.fields().id.clone())
                .collect();
            match active.as_slice() {
                [] => Ok(()),
                [_, ..] => Err(BoardRuleError::ChildrenActive(active)),
            }
        }
        Draft | Ready | InProgress | Review | Blocked | Done => Ok(()),
    }
}

/// What a move does to the claim, when the table and its actor allow it,
/// a task others took over is not finished, and a task that links pull
/// requests is not finished by its holder.
fn allowed(
    task: &Task,
    to: TaskStatus,
    actor: &Identity,
    now: &Timestamp,
) -> Result<ClaimEffect, BoardRuleError> {
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
    // A task others took over is never done, whatever it links.
    match to {
        Done => not_superseded(task)?,
        Draft | Ready | Claimed | InProgress | Review | Blocked | Archived => {}
    }
    // The holder finishes a task only when it links no pull request and
    // no change: one that links them is done when they merge
    // (`complete_merged`, `changes::complete`). The holder may unlink one
    // deliberately (`remove_pr`, `changes::unlink`), and then finish. A
    // draft, ready or reviewed task is archived on the same terms, so an
    // archived task pins no pull request that has not merged. The changes
    // are `move_task`'s to judge.
    match (held_by_links(from, to), &task.fields().prs[..]) {
        (true, [_, ..]) => Err(BoardRuleError::PrsLinked(task.fields().prs.clone())),
        (true, []) | (false, _) => Ok(*effect),
    }
}

/// The system move behind "a merged PR moves its task to done": a task
/// that is ready (released with its pull requests linked) or held
/// (claimed, in progress, in review or blocked), whose linked pull
/// requests are all in `merged`, and which links no change still open
/// (`changes` is the task's, as the board holds them), is done, whoever
/// holds it and whoever merged them. Its claim is cleared, and its run
/// still going, if one of its `runs` is, is ended as lost at the
/// completion's stamp: it can no longer report its end to a done task, as
/// a run whose host died cannot.
pub fn complete_merged(
    task: &Task,
    merged: &BTreeSet<u64>,
    changes: &[Change],
    runs: &[Run],
    now: &Timestamp,
) -> Result<Completed, BoardRuleError> {
    completable(task)?;
    let open = numbers(task, changes, Change::is_open)?;
    match open.as_slice() {
        [] => {}
        open => return Err(BoardRuleError::ChangesUnmerged(open.to_vec())),
    }
    let fields = task.fields();
    let unmerged: Vec<u64> = fields
        .prs
        .iter()
        .copied()
        .filter(|pr| !merged.contains(pr))
        .collect();
    match (&fields.prs[..], &unmerged[..]) {
        ([_, ..], []) => {}
        ([], _) => return Err(BoardRuleError::NoPrs),
        (_, open) => return Err(BoardRuleError::PrsUnmerged(open.to_vec())),
    }
    finish(task, runs, now).map(|(completed, _)| completed)
}

/// The numbers of the `changes` that are `kept`, every one of `changes`
/// being `task`'s.
fn numbers(
    task: &Task,
    changes: &[Change],
    kept: fn(&Change) -> bool,
) -> Result<Vec<u32>, BoardRuleError> {
    match changes
        .iter()
        .all(|change| change.fields().task == task.fields().id)
    {
        true => Ok(changes
            .iter()
            .filter(|change| kept(change))
            .map(|change| change.fields().number)
            .collect()),
        false => Err(BoardRuleError::NotThisTasks),
    }
}

/// A task that is ready (released with what delivers it linked) or held
/// may be completed; any other may not. A use case asks before it reads
/// the task's runs and changes, so a completion refused anyway reads
/// nothing.
///
/// # Errors
/// A draft, done or archived task, or one others took over.
pub fn completable(task: &Task) -> Result<(), BoardRuleError> {
    match task.fields().status {
        Ready | Claimed | InProgress | Review | Blocked => not_superseded(task),
        from @ (Draft | Done | Archived) => Err(BoardRuleError::NotAllowed { from, to: Done }),
    }
}

/// `task` is not superseded: a task others took over is archived as
/// superseded, never done.
fn not_superseded(task: &Task) -> Result<(), BoardRuleError> {
    match task.fields().superseded_by.as_slice() {
        [] => Ok(()),
        by => Err(BoardRuleError::Superseded { by: by.to_vec() }),
    }
}

/// `task` done at its stamp for `now`, its claim cleared and its run
/// going, if one of `runs` is, ended as lost at that stamp; with the stamp.
pub(super) fn finish(
    task: &Task,
    runs: &[Run],
    now: &Timestamp,
) -> Result<(Completed, Timestamp), BoardRuleError> {
    let fields = task.fields();
    let at = stamp(task, now)?;
    let open = open_of(task, runs)?;
    let run = open.map(|run| lost_at(run, &at)).transpose()?;
    assert_eq!(
        open.is_some(),
        run.as_ref().is_some_and(|run| !run.is_open()),
        "the run going, and only it, is ended"
    );
    let mut next = fields.clone();
    next.status = Done;
    assert!(
        fields.superseded_by.is_empty(),
        "a task others took over is never completed"
    );
    next.resolution = resolved(Done, next.resolution, false);
    next.claim = None;
    let task = settle(next, &at)?;
    assert_eq!(
        task.fields().resolution,
        Some(Resolution::Completed),
        "a completed task is resolved so"
    );
    Ok((Completed { task, run }, at))
}

/// A task's resolution once it moves `to` a status: one done is
/// completed; one archived keeps what it had (its done one's), or is
/// `superseded` when others took it over (`superseded`), or has none; any
/// other has none, as it had none (only a done or archived task is
/// resolved, and neither moves back).
fn resolved(to: TaskStatus, had: Option<Resolution>, superseded: bool) -> Option<Resolution> {
    match (to, had, superseded) {
        (Done, _, _) => Some(Resolution::Completed),
        (Archived, Some(had), _) => Some(had),
        (Archived, None, true) => Some(Resolution::Superseded),
        (Archived, None, false) => None,
        (Draft | Ready | Claimed | InProgress | Review | Blocked, _, _) => {
            assert!(
                had.is_none(),
                "an unresolved task moves among the open statuses"
            );
            None
        }
    }
}

/// A task completed: done, and its run ended as lost if one was going.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Completed {
    pub task: Task,
    pub run: Option<Run>,
}

#[cfg(test)]
#[path = "transitions_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "transitions_archive_tests.rs"]
mod archive_tests;

#[cfg(test)]
#[path = "transitions_relations_tests.rs"]
mod relations_tests;
