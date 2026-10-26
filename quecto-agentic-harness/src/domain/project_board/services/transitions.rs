//! Which status moves a task may make, and who may make them.
use super::super::entities::change::Change;
use super::super::entities::claim::Identity;
use super::super::entities::run::Run;
use super::super::entities::task::Task;
use super::super::value_objects::board_text::entered_line;
use super::super::value_objects::rule_error::BoardRuleError;
use super::super::value_objects::schema_error::SchemaError;
use super::super::value_objects::slug::Slug;
use super::super::value_objects::timestamp::Timestamp;
use super::super::value_objects::vocabulary::TaskStatus::{self, *};
use super::super::value_objects::vocabulary::{LifecycleAction, Resolution};
use super::claims::{held_by, held_live_by, held_or_lapsed_by, is_live, new_claim, settle, stamp};
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

/// What a move takes beside its actor (#2705 T4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Takes {
    Nothing,
    /// The mover's reason, kept on the task ([`MoveReason`]).
    Reason,
}

/// Every move `move` makes (#2705 T4 narrowed it to the moves people
/// make); a pair not listed is the lifecycle's ([`LIFECYCLE`]) or no move
/// at all. The other moves to done are the system's when the task's pull
/// requests merge ([`complete_merged`]) and [`done_without_run`].
#[rustfmt::skip] // One readable rule per row.
const TRANSITIONS: &[(TaskStatus, TaskStatus, Actor, ClaimEffect, Takes)] = &[
    (Draft,      Ready,      Actor::Anyone,          ClaimEffect::Keep,  Takes::Nothing),
    (Draft,      Archived,   Actor::Anyone,          ClaimEffect::Keep,  Takes::Nothing),
    (Ready,      Draft,      Actor::Anyone,          ClaimEffect::Keep,  Takes::Nothing),
    (Ready,      Claimed,    Actor::Anyone,          ClaimEffect::Take,  Takes::Nothing),
    (Ready,      Archived,   Actor::Anyone,          ClaimEffect::Keep,  Takes::Nothing),
    // Mark done: the holder finishes a reviewed task linking nothing.
    (Review,     Done,       Actor::Holder,          ClaimEffect::Clear, Takes::Nothing),
    (Done,       Archived,   Actor::Anyone,          ClaimEffect::Keep,  Takes::Nothing),
    // Archive a reviewed task: its holder's while the claim is live,
    // anyone's once nobody holds it. A run going holds it back
    // (`may_move_with`), so it never races a live run; a pull request or
    // change linked holds it as it holds review to done
    // ([`held_by_links`], as it holds a draft's or a ready one's
    // archive); the claim goes with it.
    (Review,     Archived,   Actor::HolderWhileLive, ClaimEffect::Clear, Takes::Nothing),
    // Release: the task goes back to the board. A task in progress with a
    // run going goes back by its run's end (#2705 T5: `move` names
    // `end_run`, [`run_ends_it`]); one with no run going (a legacy state:
    // no run, or only ended ones) is released by its holder.
    (Claimed,    Ready,      Actor::Releaser,        ClaimEffect::Clear, Takes::Nothing),
    (InProgress, Ready,      Actor::Releaser,        ClaimEffect::Clear, Takes::Nothing),
    // Back to ready from review says why.
    (Review,     Ready,      Actor::HolderWhileLive, ClaimEffect::Clear, Takes::Reason),
    (Blocked,    Ready,      Actor::HolderWhileLive, ClaimEffect::Clear, Takes::Nothing),
];

/// Every move the task's lifecycle makes, never `move` (#2705 T4), and the
/// action that makes it or is the way on, which `move`'s refusal names:
/// `start` is the only way to in progress; a run's end decides where a
/// task in progress goes (`run_outcomes::task_after_end`, #2705 T5);
/// work without a run is released, never blocked (blocking is a run's
/// outcome); and work done without a run is finished by
/// `done_without_run`. A pair in neither table is no move at all.
#[rustfmt::skip] // One readable rule per row.
const LIFECYCLE: &[(TaskStatus, TaskStatus, LifecycleAction)] = &[
    (Ready,      InProgress, LifecycleAction::Start),
    (Claimed,    InProgress, LifecycleAction::Start),
    (Review,     InProgress, LifecycleAction::Start),
    (Blocked,    InProgress, LifecycleAction::Start),
    (InProgress, Review,     LifecycleAction::EndRun),
    (InProgress, Blocked,    LifecycleAction::EndRun),
    (Claimed,    Blocked,    LifecycleAction::Release),
    (Ready,      Done,       LifecycleAction::DoneWithoutRun),
    (Claimed,    Done,       LifecycleAction::DoneWithoutRun),
];

/// The lifecycle action that makes the move `from` -> `to`, if the
/// lifecycle makes it ([`LIFECYCLE`]).
pub fn lifecycle_action(from: TaskStatus, to: TaskStatus) -> Option<LifecycleAction> {
    LIFECYCLE
        .iter()
        .find(|(row_from, row_to, _)| (*row_from, *row_to) == (from, to))
        .map(|(_, _, action)| *action)
}

/// Why a reason given to a move that takes none is refused.
const REASON_NOT_TAKEN: &str = "is taken only by a move from review back to ready";
/// Why an empty reason is refused.
const REASON_EMPTY: &str = "must say why";

/// Why a move is made (#2705 T4), as its front end lets a mover say.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MoveReason {
    /// None given.
    Absent,
    /// The mover's reason, as it enters the board: one line.
    Given(String),
    /// None given, through a front end that lets a task move back to
    /// ready from review without one: the CLI, which the Pocket relay
    /// moves through until its task page sends a reason (as it lets the
    /// relay's free text in, `EntryPolicy::SectionedOrLegacy`).
    Unasked,
}

/// The reason a move keeps, when it takes one (`takes`), from what its
/// mover gave (`given`): a move back to ready from review keeps the reason
/// given, refuses none (but for the CLI's unasked one, which keeps none),
/// and any other move refuses one, as it would never keep it.
fn reason_kept(
    (from, to): (TaskStatus, TaskStatus),
    takes: Takes,
    given: &MoveReason,
) -> Result<Option<String>, BoardRuleError> {
    match (takes, given) {
        (Takes::Reason, MoveReason::Given(text)) => entered_reason(text).map(Some),
        (Takes::Reason, MoveReason::Unasked)
        | (Takes::Nothing, MoveReason::Absent | MoveReason::Unasked) => Ok(None),
        (Takes::Reason, MoveReason::Absent) => Err(BoardRuleError::ReasonRequired { from, to }),
        (Takes::Nothing, MoveReason::Given(_)) => Err(BoardRuleError::Invalid(SchemaError::new(
            "reason",
            REASON_NOT_TAKEN,
        ))),
    }
}

/// `text`, a reason as it enters the board: one normalised line that says
/// something (its length is the task's schema's to judge).
fn entered_reason(text: &str) -> Result<String, BoardRuleError> {
    entered_line("reason", text).map_err(BoardRuleError::Invalid)?;
    match text.is_empty() {
        false => Ok(text.to_string()),
        true => Err(BoardRuleError::Invalid(SchemaError::new(
            "reason",
            REASON_EMPTY,
        ))),
    }
}

/// What [`done_without_run`] reads of the task's work, once its rule
/// allows `actor` to finish it so at `now`: its runs (one going holds it)
/// and its changes (one not merged holds it). Its linked pull requests are
/// the caller's to read on the host ([`DoneWork::merged`]).
///
/// # Errors
/// What [`done_without_run`] refuses before it looks at runs, changes or
/// pull requests.
pub fn done_without_run_reads(
    task: &Task,
    actor: &Identity,
    now: &Timestamp,
) -> Result<MoveReads, BoardRuleError> {
    let from = task.fields().status;
    match from {
        Ready => {}
        Claimed => {
            held_live_by(task, actor, now)?;
        }
        Draft | InProgress | Review | Blocked | Done | Archived => {
            return Err(BoardRuleError::NotAllowed { from, to: Done });
        }
    }
    not_superseded(task)?;
    Ok(MoveReads {
        runs: true,
        changes: true,
    })
}

/// What a task done without a run is judged by: its `runs` and `changes`
/// (all of them, as the board holds them), the `tasks` it relates to
/// (every task the board holds), and which of its linked pull requests the
/// host reads as `merged`.
#[derive(Debug, Clone, Copy)]
pub struct DoneWork<'a> {
    pub runs: &'a [Run],
    pub changes: &'a [Change],
    pub tasks: &'a [Task],
    pub merged: &'a BTreeSet<u64>,
}

/// `task` done without a run (#2705 T4): work finished with no run of its
/// plan (an answer, a fix made by hand), for `reason`, by `actor` at `now`.
/// A ready task is anyone's to finish so, a claimed one its holder's while
/// the claim is live. It is resolved `done_without_run`, keeps the reason
/// and leaves its claim. What it is judged by ([`DoneWork`]) must hold no
/// run going, no change and no linked pull request that has not merged
/// (the owner's choice, #2705 T4 review round 1: done without a run once
/// every linked pull request has merged), and let it go as a claim and an
/// archive would ([`relations_allow`]): every task it depends on done or
/// archived, and no child task held.
///
/// # Errors
/// Any other status (`not_allowed`); not the holder of a live claim; a
/// task others took over; a change (`changes_unmerged`) or a linked pull
/// request (`prs_open`) not merged; a run going; a task it depends on not
/// done, or a child task held; a reason that is empty or never normalised
/// as one line.
pub fn done_without_run(
    task: &Task,
    reason: &str,
    actor: &Identity,
    work: DoneWork<'_>,
    now: &Timestamp,
) -> Result<Task, BoardRuleError> {
    done_without_run_reads(task, actor, now)?;
    let reason = entered_reason(reason)?;
    let open = numbers(task, work.changes, Change::is_open)?;
    match open.as_slice() {
        [] => {}
        [_, ..] => return Err(BoardRuleError::ChangesUnmerged(open)),
    }
    let unmerged: Vec<u64> = (task.fields().prs.iter())
        .copied()
        .filter(|pr| !work.merged.contains(pr))
        .collect();
    match unmerged.as_slice() {
        [] => {}
        [_, ..] => return Err(BoardRuleError::PrsOpen(unmerged)),
    }
    may_move_with(task, work.runs, Done)?;
    // As a claim waits for the task's dependencies, and an archive for its
    // children to be free of work.
    relations_allow(task, Claimed, work.tasks)?;
    relations_allow(task, Archived, work.tasks)?;
    let at = stamp(task, now);
    let mut next = task.fields().clone();
    next.status = Done;
    next.resolution = Some(Resolution::DoneWithoutRun);
    next.reason = Some(reason);
    next.claim = None;
    let done = settle(next, &at)?;
    assert!(
        done.fields().reason.is_some() && done.fields().claim.is_none(),
        "a task done without a run says why and is held by nobody"
    );
    Ok(done)
}

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
    // Only a move with a row (and so no lifecycle action) gets here.
    debug_assert_eq!(lifecycle_action(from, to), None);
    Ok(MoveReads {
        runs: holds_moves_to(to),
        changes: held_by_links(from, to),
    })
}

/// `task` moved to `to` by `actor` at `now`, if the table allows it, its
/// `runs` and `changes` (all of them, as the board holds them) let it
/// leave, and the `tasks` it relates to (every task the board holds) let
/// it move ([`relations_allow`], #2661). Only the runs and changes
/// [`move_reads`] names count: a caller may leave the rest empty. No
/// reason is given ([`move_with_reason`]).
pub fn move_task(
    task: &Task,
    to: TaskStatus,
    actor: &Identity,
    works: (&[Run], &[Change]),
    tasks: &[Task],
    now: &Timestamp,
) -> Result<Task, BoardRuleError> {
    move_with_reason(task, (to, &MoveReason::Absent), actor, works, tasks, now)
}

/// [`move_task`], for the `reason` its mover gives (#2705 T4): a move back
/// to ready from review keeps it on the task, and any other move clears
/// the one the task had, but for an archive of a task done without a run,
/// which keeps it with its resolution.
///
/// # Errors
/// As [`move_task`]; a move the lifecycle makes (`use_lifecycle`, naming
/// its action); a move back to ready from review given no reason
/// (`reason_required`); a reason given to any other move, or one empty or
/// never normalised as one line (`invalid`, field `reason`).
pub fn move_with_reason(
    task: &Task,
    (to, reason): (TaskStatus, &MoveReason),
    actor: &Identity,
    (runs, changes): (&[Run], &[Change]),
    tasks: &[Task],
    now: &Timestamp,
) -> Result<Task, BoardRuleError> {
    let (effect, takes) = allowed(task, to, actor, now)?;
    let from = task.fields().status;
    let kept = reason_kept((from, to), takes, reason)?;
    let linked = numbers(task, changes, Change::is_linked)?;
    // Only review to done, and draft, ready or review to archived, are
    // held by a change linked.
    if held_by_links(from, to) && !linked.is_empty() {
        return Err(BoardRuleError::ChangesLinked(linked));
    }
    may_move_with(task, runs, to).map_err(|error| run_ends_it(from, to, error))?;
    relations_allow(task, to, tasks)?;
    let at = stamp(task, now);
    let mut next = task.fields().clone();
    next.status = to;
    next.resolution = resolved(to, next.resolution, !next.superseded_by.is_empty());
    // A task done without a run is archived with its reason, as with its
    // resolution; every other move keeps only the reason it takes.
    next.reason = match (kept, to, next.resolution) {
        (Some(reason), _, _) => Some(reason),
        (None, Archived, Some(Resolution::DoneWithoutRun)) => next.reason,
        (None, _, _) => None,
    };
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

/// `error`, a move `from` -> `to` held back by a run, as the move answers
/// it: a task in progress whose run is going leaves by the run's end
/// (#2705 T5), so its release names `end_run`; any other is as it is.
fn run_ends_it(from: TaskStatus, to: TaskStatus, error: BoardRuleError) -> BoardRuleError {
    match (from, to, error) {
        (InProgress, Ready, BoardRuleError::RunOpen(_)) => BoardRuleError::UseLifecycle {
            from,
            to,
            action: LifecycleAction::EndRun,
        },
        (_, _, error) => error,
    }
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

/// What a move does to the claim and what it takes, when the table and its
/// actor allow it, a task others took over is not finished, and a task
/// that links pull requests is not finished by its holder. A move the
/// lifecycle makes is refused naming its action, whoever asks.
fn allowed(
    task: &Task,
    to: TaskStatus,
    actor: &Identity,
    now: &Timestamp,
) -> Result<(ClaimEffect, Takes), BoardRuleError> {
    let from = task.fields().status;
    let row = TRANSITIONS
        .iter()
        .find(|(row_from, row_to, ..)| (*row_from, *row_to) == (from, to));
    let (_, _, who, effect, takes) = match (row, lifecycle_action(from, to)) {
        (Some(row), None) => row,
        (None, Some(action)) => return Err(BoardRuleError::UseLifecycle { from, to, action }),
        (None, None) => return Err(BoardRuleError::NotAllowed { from, to }),
        (Some(_), Some(action)) => {
            unreachable!("{from:?} -> {to:?} is both a move and the lifecycle's {action:?}")
        }
    };
    match who {
        Actor::Anyone => {}
        Actor::Holder => {
            held_live_by(task, actor, now)?;
        }
        Actor::Releaser => {
            held_or_lapsed_by(task, actor, now)?;
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
        (true, []) | (false, _) => Ok((*effect, *takes)),
    }
}

/// The system move behind "a merged PR moves its task to done": a task
/// in review (the only status it completes, #2705 T4), whose linked pull
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

/// A task in review may be completed; any other may not (#2705 T4: a
/// merged pull request completes a task only from review, where its run
/// moves it). A use case asks before it reads the task's runs and
/// changes, so a completion refused anyway reads nothing.
///
/// # Errors
/// A task in any other status (`use_lifecycle`, naming done_without_run, for
/// a ready or claimed one; `not_allowed` for the rest), or one others took
/// over.
pub fn completable(task: &Task) -> Result<(), BoardRuleError> {
    match task.fields().status {
        Review => not_superseded(task),
        // A ready or claimed task is pointed to done_without_run, as `move` is.
        from @ (Draft | Ready | Claimed | InProgress | Blocked | Done | Archived) => {
            Err(match lifecycle_action(from, Done) {
                Some(action) => BoardRuleError::UseLifecycle {
                    from,
                    to: Done,
                    action,
                },
                None => BoardRuleError::NotAllowed { from, to: Done },
            })
        }
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
    let at = stamp(task, now);
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

#[cfg(test)]
#[path = "transitions_holder_tests.rs"]
mod holder_tests;

#[cfg(test)]
#[path = "transitions_lifecycle_tests.rs"]
mod lifecycle_tests;

#[cfg(test)]
#[path = "transitions_done_tests.rs"]
mod done_tests;
