//! A task's changes (#2481's artifact model): the holder links a branch
//! and its pull request, and may unlink a wrong or abandoned one (kept,
//! marked); a task whose linked changes have all merged is done, whoever
//! holds it, and each change records its merge commit. Every rule takes
//! the changes as the board holds them, and refuses changes that
//! contradict each other rather than choose among them.
//!
//! What the use cases must bring (M3/M4): `link` is given every change on
//! the board, so a branch or pull request is linked to one task only, and
//! checks at link time that the pull request's head is the change's
//! branch; `unlink` asks the host first and refuses a change whose pull
//! request has merged there (`change_merged`); `complete` is given each
//! merged pull request as the host reads it, never as a caller says.
use std::collections::{BTreeMap, BTreeSet};

use super::super::entities::change::{Change, ChangeFields, MAX_CHANGE_NUMBER};
use super::super::entities::claim::Identity;
use super::super::entities::run::Run;
use super::super::entities::task::Task;
use super::super::value_objects::code_ref::{BranchName, CommitId};
use super::super::value_objects::rule_error::BoardRuleError;
use super::super::value_objects::schema_error::SchemaError;
use super::super::value_objects::timestamp::Timestamp;
use super::claims::held_live_by;
use super::transitions::{Completed, completable, finish};

/// A pull request the host reads as merged: its merge commit, and the
/// branch it merged from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MergedPr {
    pub merge_commit: CommitId,
    pub head_ref: BranchName,
}

/// A task completed by its changes merging: done, each linked change
/// recording its merge commit, and its run ended as lost if one was going.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangesMerged {
    pub completed: Completed,
    pub changes: Vec<Change>,
}

/// A new change of `task` linking `branch` (and its pull request `pr`),
/// linked by `actor`, the holder of a live claim, at `now`, numbered after
/// the task's changes. `board` is every change on the board, the task's
/// and every other task's: a branch or pull request is linked to one task
/// only.
///
/// # Errors
/// Not the holder of a live claim; the task's changes contradicting each
/// other; a change anywhere on the board linking the same branch or pull
/// request; no number left; or a change its schema refuses.
pub fn link(
    task: &Task,
    board: &[Change],
    branch: BranchName,
    pr: Option<u64>,
    actor: &Identity,
    now: &Timestamp,
) -> Result<Change, BoardRuleError> {
    held_live_by(task, actor, now)?;
    let own: Vec<Change> = board
        .iter()
        .filter(|change| change.fields().task == task.fields().id)
        .cloned()
        .collect();
    consistent(task, &own)?;
    let same = |change: &&Change| {
        let fields = change.fields();
        change.is_linked() && (fields.branch == branch || (pr.is_some() && fields.pr == pr))
    };
    match board.iter().find(same) {
        None => {}
        Some(linked) => return Err(BoardRuleError::ChangeAlreadyLinked(linked.fields().number)),
    }
    let last = own
        .iter()
        .map(|change| change.fields().number)
        .max()
        .unwrap_or(0);
    match last < MAX_CHANGE_NUMBER {
        true => {}
        false => {
            return Err(BoardRuleError::Invalid(SchemaError::new(
                "number",
                format!("the task has reached {MAX_CHANGE_NUMBER} changes"),
            )));
        }
    }
    Change::new(ChangeFields {
        task: task.fields().id.clone(),
        number: last + 1,
        branch,
        pr,
        merged: None,
        linked_by: actor.clone(),
        unlinked: None,
        created: now.clone(),
        updated: now.clone(),
    })
    .map_err(BoardRuleError::Invalid)
}

/// `change` of `task`, one of its `changes`, unlinked by `actor`, the
/// holder of a live claim, at `now` (a second past its last change at the
/// earliest): a wrong or abandoned change no longer delivers the task, and
/// is kept. The use case refuses first a change whose pull request the
/// host reads as merged.
///
/// # Errors
/// Not the holder of a live claim; changes contradicting each other; a
/// change of another task, one already unlinked, or one merged.
pub fn unlink(
    task: &Task,
    changes: &[Change],
    change: &Change,
    actor: &Identity,
    now: &Timestamp,
) -> Result<Change, BoardRuleError> {
    held_live_by(task, actor, now)?;
    consistent(task, changes)?;
    of_task(task, std::slice::from_ref(change))?;
    let fields = change.fields();
    match (fields.merged.is_some(), change.is_linked()) {
        (false, true) => {}
        (true, _) => return Err(BoardRuleError::ChangeMerged(fields.number)),
        (false, false) => return Err(BoardRuleError::ChangeUnlinked(fields.number)),
    }
    let at = after(change, now)?;
    let mut next = fields.clone();
    next.unlinked = Some((actor.clone(), at.clone()));
    next.updated = at;
    Change::new(next).map_err(BoardRuleError::Invalid)
}

/// `task` done because every change it links has merged: `merged` names
/// each merged pull request as the host reads it. Each linked change
/// records its merge commit; an unlinked one is left as it is; the task's
/// run going, if one of `runs` is, is ended as lost.
///
/// # Errors
/// A task not ready or held; changes contradicting each other; no change
/// linked; linked changes not merged (no pull request, or one the host has
/// not merged); a pull request merged from another branch than its change
/// names; a change that already records a merge; runs that contradict each
/// other.
pub fn complete(
    task: &Task,
    changes: &[Change],
    merged: &BTreeMap<u64, MergedPr>,
    runs: &[Run],
    now: &Timestamp,
) -> Result<ChangesMerged, BoardRuleError> {
    completable(task)?;
    consistent(task, changes)?;
    let linked: Vec<&Change> = changes.iter().filter(|change| change.is_linked()).collect();
    let read = |change: &Change| change.fields().pr.and_then(|pr| merged.get(&pr));
    let unmerged: Vec<u32> = linked
        .iter()
        .filter(|change| read(change).is_none())
        .map(|change| change.fields().number)
        .collect();
    match (linked.as_slice(), unmerged.as_slice()) {
        ([_, ..], []) => {}
        ([], _) => return Err(BoardRuleError::NoChanges),
        (_, open) => return Err(BoardRuleError::ChangesUnmerged(open.to_vec())),
    }
    for change in &linked {
        let fields = change.fields();
        match (fields.merged.is_none(), read(change)) {
            (true, Some(pr)) if pr.head_ref == fields.branch => {}
            (false, _) => return Err(BoardRuleError::ChangeMerged(fields.number)),
            (true, _) => return Err(BoardRuleError::ChangeHeadMismatch(fields.number)),
        }
    }
    let (completed, at) = finish(task, runs, now)?;
    let changes = linked
        .into_iter()
        .map(|change| {
            let mut next = change.fields().clone();
            next.merged = read(change).map(|pr| pr.merge_commit.clone());
            next.updated = at.clone().max(after(change, now)?);
            Change::new(next).map_err(BoardRuleError::Invalid)
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(ChangesMerged { completed, changes })
}

/// The task's changes as the board holds them agree: every one is the
/// task's, no number is listed twice, and no branch or pull request is
/// linked twice.
fn consistent(task: &Task, changes: &[Change]) -> Result<(), BoardRuleError> {
    of_task(task, changes)?;
    let numbers: BTreeSet<u32> = changes
        .iter()
        .map(|change| change.fields().number)
        .collect();
    let linked: Vec<&Change> = changes.iter().filter(|change| change.is_linked()).collect();
    let branches: BTreeSet<&BranchName> = linked
        .iter()
        .map(|change| &change.fields().branch)
        .collect();
    let prs: Vec<u64> = linked
        .iter()
        .filter_map(|change| change.fields().pr)
        .collect();
    let distinct_prs: BTreeSet<u64> = prs.iter().copied().collect();
    let why = match (
        numbers.len() == changes.len(),
        branches.len() == linked.len(),
        distinct_prs.len() == prs.len(),
    ) {
        (true, true, true) => return Ok(()),
        (false, _, _) => "a change number is listed twice",
        (true, false, _) => "a branch is linked by two changes",
        (true, true, false) => "a pull request is linked by two changes",
    };
    Err(BoardRuleError::ChangesInconsistent(why.into()))
}

/// Every change in `changes` is `task`'s.
fn of_task(task: &Task, changes: &[Change]) -> Result<(), BoardRuleError> {
    match changes
        .iter()
        .all(|change| change.fields().task == task.fields().id)
    {
        true => Ok(()),
        false => Err(BoardRuleError::NotThisTasks),
    }
}

/// `now`, or a second past `change`'s last change if that is later.
fn after(change: &Change, now: &Timestamp) -> Result<Timestamp, BoardRuleError> {
    let next = change
        .fields()
        .updated
        .plus_seconds(1)
        .map_err(|error| BoardRuleError::Invalid(error.at("updated")))?;
    Ok(now.clone().max(next))
}

#[cfg(test)]
#[path = "changes_tests.rs"]
mod tests;
