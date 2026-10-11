//! A task's changes (#2481's artifact model): the holder links a branch
//! and its pull request, and may unlink a wrong or abandoned one (kept,
//! marked); a task whose linked changes have all merged is done, whoever
//! holds it, and each change records its merge commit. Every rule takes
//! the changes as the board holds them, and refuses changes that
//! contradict each other, or that a clock running ahead stamped, rather
//! than build on them.
//!
//! What the host says is a parameter, read by the use case (M4) and never
//! taken from a caller: [`HostPrs`], the host's reading of the pull
//! requests a rule needs. A rule refuses a pull request it was given no
//! reading of (`pr_unread`) rather than guess its state. The link use case
//! also gives `link` every change on the board, the task's and every other
//! task's.
//!
//! A pull request is linked to one task on the board, for good (merged
//! ones stay linked); a branch, named by its repository and name, to one
//! open change, so its name is free again once that change merges or is
//! unlinked.
use std::collections::{BTreeMap, BTreeSet};

use super::super::entities::change::{Change, ChangeFields, MAX_CHANGE_NUMBER};
use super::super::entities::claim::Identity;
use super::super::entities::run::Run;
use super::super::entities::task::Task;
use super::super::value_objects::code_ref::{BranchName, CommitId, RepoName};
use super::super::value_objects::rule_error::BoardRuleError;
use super::super::value_objects::schema_error::SchemaError;
use super::super::value_objects::timestamp::Timestamp;
use super::claims::{held_live_by, within_slack};
use super::plans::Moment;
use super::transitions::{Completed, completable, finish};

/// Where a pull request stands, as the host reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PrState {
    Open,
    /// Closed without merging.
    Closed,
    /// Merged, as this merge commit.
    Merged(CommitId),
}

/// A pull request as the host reads it: the repository and branch it
/// comes from, and where it stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostPr {
    pub head_repo: RepoName,
    pub head_ref: BranchName,
    pub state: PrState,
}

/// The host's reading of pull requests, by number.
pub type HostPrs = BTreeMap<u64, HostPr>;

/// What a change links: a branch of a repository, and its pull request
/// once there is one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Head {
    pub repo: RepoName,
    pub branch: BranchName,
    pub pr: Option<u64>,
}

/// A task completed by its changes merging: done, each linked change
/// recording its merge commit, and its run ended as lost if one was going.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangesMerged {
    pub completed: Completed,
    pub changes: Vec<Change>,
}

/// A new change of `task` linking `head`, linked by `actor`, the holder of
/// a live claim, `when`, numbered after the task's changes. `board` is
/// every change on the board; `host` reads `head`'s pull request, which
/// must be open and come from `head`'s repository and branch.
///
/// # Errors
/// Not the holder of a live claim; the task's changes contradicting each
/// other or stamped ahead; a pull request the task links already, unread,
/// not open or from another head; a change anywhere on the board linking
/// the pull request, or the branch while open; no number left; or a
/// change its schema refuses.
pub fn link(
    task: &Task,
    board: &[Change],
    head: Head,
    host: &HostPrs,
    actor: &Identity,
    when: &Moment,
) -> Result<Change, BoardRuleError> {
    held_live_by(task, actor, &when.now)?;
    let own: Vec<Change> = board
        .iter()
        .filter(|change| change.fields().task == task.fields().id)
        .cloned()
        .collect();
    consistent(task, &own)?;
    not_ahead(&own, when)?;
    match head.pr {
        None => {}
        Some(pr) => linkable(task, pr, &head, host)?,
    }
    let taken = |change: &&Change| {
        let pr_taken = change.is_linked() && head.pr.is_some() && change.fields().pr == head.pr;
        let branch_taken = change.is_open() && change.head() == (&head.repo, &head.branch);
        pr_taken || branch_taken
    };
    match board.iter().find(taken) {
        None => {}
        Some(linked) => {
            return Err(BoardRuleError::ChangeAlreadyLinked {
                task: linked.fields().task.clone(),
                change: linked.fields().number,
            });
        }
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
        head_repo: head.repo,
        branch: head.branch,
        pr: head.pr,
        merged: None,
        linked_by: actor.clone(),
        unlinked: None,
        created: when.now.clone(),
        updated: when.now.clone(),
    })
    .map_err(BoardRuleError::Invalid)
}

/// Pull request `pr` may be linked to `task` as `head`: the task does not
/// link it the old way, and the host reads it open, from `head`.
fn linkable(task: &Task, pr: u64, head: &Head, host: &HostPrs) -> Result<(), BoardRuleError> {
    match task.fields().prs.contains(&pr) {
        false => {}
        true => return Err(BoardRuleError::PrAlreadyLinked(pr)),
    }
    let read = host.get(&pr).ok_or(BoardRuleError::PrUnread(pr))?;
    let same_head = (&read.head_repo, &read.head_ref) == (&head.repo, &head.branch);
    match (&read.state, same_head) {
        (PrState::Open, true) => Ok(()),
        (PrState::Open, false) => Err(BoardRuleError::PrHeadMismatch(pr)),
        (PrState::Closed | PrState::Merged(_), _) => Err(BoardRuleError::PrNotOpen(pr)),
    }
}

/// Change `number` of `task`, one of its `changes`, unlinked by `actor`,
/// the holder of a live claim, `when` (a second past its last change at
/// the earliest): a wrong or abandoned change no longer delivers the task,
/// and is kept. `host` reads its pull request, if it has one: a merged one
/// stays linked.
///
/// # Errors
/// Not the holder of a live claim; changes contradicting each other or
/// stamped ahead; no change of that number; one already unlinked, or
/// merged (as recorded, or as the host reads its pull request); a pull
/// request unread.
pub fn unlink(
    task: &Task,
    changes: &[Change],
    number: u32,
    host: &HostPrs,
    actor: &Identity,
    when: &Moment,
) -> Result<Change, BoardRuleError> {
    held_live_by(task, actor, &when.now)?;
    consistent(task, changes)?;
    not_ahead(changes, when)?;
    let change = changes
        .iter()
        .find(|change| change.fields().number == number)
        .ok_or(BoardRuleError::NoSuchChange(number))?;
    let fields = change.fields();
    match (fields.merged.is_some(), change.is_linked()) {
        (false, true) => {}
        (true, _) => return Err(BoardRuleError::ChangeMerged(number)),
        (false, false) => return Err(BoardRuleError::ChangeUnlinked(number)),
    }
    let read = fields
        .pr
        .map(|pr| {
            host.get(&pr)
                .map(|read| &read.state)
                .ok_or(BoardRuleError::PrUnread(pr))
        })
        .transpose()?;
    match read {
        None | Some(PrState::Open | PrState::Closed) => {}
        Some(PrState::Merged(_)) => return Err(BoardRuleError::ChangeMerged(number)),
    }
    let at = after(change, &when.now)?;
    let mut next = fields.clone();
    next.unlinked = Some((actor.clone(), at.clone()));
    next.updated = at;
    Change::new(next).map_err(BoardRuleError::Invalid)
}

/// `task` done because everything it links has merged, as `host` reads
/// it: every linked change, and every pull request the task links the
/// old way. Each linked change records its merge commit; an unlinked one
/// is left as it is; the task's run going, if one of `runs` is, is ended
/// as lost.
///
/// # Errors
/// A task not ready or held; changes contradicting each other or stamped
/// ahead; a pull request linked the old way not merged; no change linked;
/// linked changes not merged (no pull request, or one the host has not
/// merged); a pull request merged from another head than its change
/// names; a change that already records a merge; runs that contradict
/// each other.
pub fn complete(
    task: &Task,
    changes: &[Change],
    host: &HostPrs,
    runs: &[Run],
    when: &Moment,
) -> Result<ChangesMerged, BoardRuleError> {
    completable(task)?;
    consistent(task, changes)?;
    not_ahead(changes, when)?;
    let merged = |pr: &u64| match host.get(pr).map(|read| &read.state) {
        Some(PrState::Merged(_)) => true,
        None | Some(PrState::Open | PrState::Closed) => false,
    };
    let old_open: Vec<u64> = task
        .fields()
        .prs
        .iter()
        .filter(|pr| !merged(pr))
        .copied()
        .collect();
    match old_open.as_slice() {
        [] => {}
        open => return Err(BoardRuleError::PrsUnmerged(open.to_vec())),
    }
    let linked: Vec<&Change> = changes.iter().filter(|change| change.is_linked()).collect();
    let unmerged: Vec<u32> = linked
        .iter()
        .filter(|change| !change.fields().pr.is_some_and(|pr| merged(&pr)))
        .map(|change| change.fields().number)
        .collect();
    match (linked.as_slice(), unmerged.as_slice()) {
        ([_, ..], []) => {}
        ([], _) => return Err(BoardRuleError::NoChanges),
        (_, open) => return Err(BoardRuleError::ChangesUnmerged(open.to_vec())),
    }
    let mut commits = Vec::with_capacity(linked.len());
    for change in &linked {
        let fields = change.fields();
        let read = fields.pr.and_then(|pr| host.get(&pr));
        match (fields.merged.is_none(), read) {
            (
                true,
                Some(HostPr {
                    head_repo,
                    head_ref,
                    state: PrState::Merged(commit),
                }),
            ) if (head_repo, head_ref) == change.head() => commits.push(commit.clone()),
            (false, _) => return Err(BoardRuleError::ChangeMerged(fields.number)),
            (true, _) => return Err(BoardRuleError::ChangeHeadMismatch(fields.number)),
        }
    }
    assert_eq!(
        commits.len(),
        linked.len(),
        "a merge commit per linked change"
    );
    let (completed, at) = finish(task, runs, &when.now)?;
    let changes = linked
        .into_iter()
        .zip(commits)
        .map(|(change, commit)| {
            let mut next = change.fields().clone();
            next.merged = Some(commit);
            next.updated = at.clone().max(after(change, &when.now)?);
            Change::new(next).map_err(BoardRuleError::Invalid)
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(ChangesMerged { completed, changes })
}

/// The task's changes as the board holds them agree: every one is the
/// task's, no number is listed twice, no pull request is linked twice, and
/// no branch is open twice.
fn consistent(task: &Task, changes: &[Change]) -> Result<(), BoardRuleError> {
    of_task(task, changes)?;
    let numbers: BTreeSet<u32> = changes
        .iter()
        .map(|change| change.fields().number)
        .collect();
    let prs: Vec<u64> = changes
        .iter()
        .filter(|change| change.is_linked())
        .filter_map(|change| change.fields().pr)
        .collect();
    let distinct_prs: BTreeSet<u64> = prs.iter().copied().collect();
    let open: Vec<&Change> = changes.iter().filter(|change| change.is_open()).collect();
    let heads: BTreeSet<(&RepoName, &BranchName)> =
        open.iter().map(|change| change.head()).collect();
    let why = match (
        numbers.len() == changes.len(),
        distinct_prs.len() == prs.len(),
        heads.len() == open.len(),
    ) {
        (true, true, true) => return Ok(()),
        (false, _, _) => "a change number is listed twice",
        (true, false, _) => "a pull request is linked by two changes",
        (true, true, false) => "a branch is open in two changes",
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

/// No change in `changes` was stamped past `when`'s latest time plus the
/// stamp's slack.
fn not_ahead(changes: &[Change], when: &Moment) -> Result<(), BoardRuleError> {
    let ahead = changes
        .iter()
        .find(|change| !within_slack(&change.fields().updated, when.latest.as_ref()));
    match ahead {
        None => Ok(()),
        Some(change) => Err(BoardRuleError::ChangeAhead {
            change: change.fields().number,
            updated: change.fields().updated.clone(),
        }),
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
