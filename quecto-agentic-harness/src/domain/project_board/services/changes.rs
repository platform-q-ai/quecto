//! A task's changes (#2481's artifact model): the holder links a branch
//! and its pull request, and may unlink a wrong or abandoned one (kept,
//! marked); a task whose linked changes have all merged is done, whoever
//! holds it, and each change records its merge commit. Every rule takes
//! all of the task's changes as the board holds them.
use std::collections::BTreeMap;

use super::super::entities::change::{Change, ChangeFields, MAX_CHANGE_NUMBER};
use super::super::entities::claim::Identity;
use super::super::entities::run::Run;
use super::super::entities::task::Task;
use super::super::value_objects::rule_error::BoardRuleError;
use super::super::value_objects::schema_error::SchemaError;
use super::super::value_objects::timestamp::Timestamp;
use super::claims::held_live_by;
use super::transitions::{Completed, completable, finish};

/// A task completed by its changes merging: done, each linked change
/// recording its merge commit, and its run ended as lost if one was going.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangesMerged {
    pub completed: Completed,
    pub changes: Vec<Change>,
}

/// A new change of `task` linking `branch` (and its pull request `pr`),
/// linked by `actor`, the holder of a live claim, at `now`, numbered after
/// every change in `changes`.
///
/// # Errors
/// Not the holder of a live claim; a change of another task; a linked
/// change naming the same branch or pull request; no number left; or a
/// change its schema refuses.
pub fn link(
    task: &Task,
    changes: &[Change],
    branch: String,
    pr: Option<u64>,
    actor: &Identity,
    now: &Timestamp,
) -> Result<Change, BoardRuleError> {
    held_live_by(task, actor, now)?;
    of_task(task, changes)?;
    let same = |change: &&Change| {
        let fields = change.fields();
        change.is_linked() && (fields.branch == branch || (pr.is_some() && fields.pr == pr))
    };
    if let Some(linked) = changes.iter().find(same) {
        return Err(BoardRuleError::ChangeAlreadyLinked(linked.fields().number));
    }
    let last = changes
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

/// `change` of `task` unlinked by `actor`, the holder of a live claim, at
/// `now`: a wrong or abandoned change no longer delivers the task, and is
/// kept.
///
/// # Errors
/// Not the holder of a live claim; a change of another task, one already
/// unlinked, or one merged.
pub fn unlink(
    task: &Task,
    change: &Change,
    actor: &Identity,
    now: &Timestamp,
) -> Result<Change, BoardRuleError> {
    held_live_by(task, actor, now)?;
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
/// each merged pull request's merge commit, as the host reads it. Each
/// linked change records its merge commit; an unlinked one is left as it
/// is; the task's run going, if one of `runs` is, is ended as lost.
///
/// # Errors
/// A task not ready or held; a change of another task; no change linked;
/// linked changes not merged (no pull request, or one the host has not
/// merged); runs that contradict each other; a merge commit that is not a
/// full commit id.
pub fn complete(
    task: &Task,
    changes: &[Change],
    merged: &BTreeMap<u64, String>,
    runs: &[Run],
    now: &Timestamp,
) -> Result<ChangesMerged, BoardRuleError> {
    completable(task)?;
    of_task(task, changes)?;
    let linked: Vec<&Change> = changes.iter().filter(|change| change.is_linked()).collect();
    let commit = |change: &Change| change.fields().pr.and_then(|pr| merged.get(&pr));
    let unmerged: Vec<u32> = linked
        .iter()
        .filter(|change| commit(change).is_none())
        .map(|change| change.fields().number)
        .collect();
    match (linked.as_slice(), unmerged.as_slice()) {
        ([_, ..], []) => {}
        ([], _) => return Err(BoardRuleError::NoChanges),
        (_, open) => return Err(BoardRuleError::ChangesUnmerged(open.to_vec())),
    }
    let (completed, at) = finish(task, runs, now)?;
    let changes = linked
        .into_iter()
        .map(|change| {
            let mut next = change.fields().clone();
            next.merged = commit(change).cloned();
            next.updated = at.clone().max(after(change, now)?);
            Change::new(next).map_err(BoardRuleError::Invalid)
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(ChangesMerged { completed, changes })
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
