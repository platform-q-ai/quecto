//! Puts a new task on the board: only what a plan sets ([`NewBoardTask`]), a
//! draft or ready task with no claim, runs, reviews, pull requests or
//! items yet, whose parent and dependencies are already on the board,
//! stamped with the time it was created. It never closes a cycle: not
//! through new references, which must exist, nor through a task still
//! naming the id of one that was deleted. Blocking: a lost write pauses the calling thread before
//! it is tried again.
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use super::board_change::{checked_writer, commit};
use crate::application::project_board::dto::{NewBoardTask, ProjectBoardError};
use crate::application::project_board::ports::{BoardClock, BoardSnapshot, ProjectBoardStore};
use crate::domain::project_board::entities::task::{Task, TaskFields};
use crate::domain::project_board::value_objects::slug::Slug;
use crate::domain::project_board::value_objects::timestamp::Timestamp;
use crate::domain::project_board::value_objects::vocabulary::TaskStatus;

pub struct CreateBoardTask {
    store: Arc<dyn ProjectBoardStore>,
    clock: Arc<dyn BoardClock>,
}

impl CreateBoardTask {
    /// # Errors
    /// The store writes as an invalid identity.
    pub fn new(
        store: Arc<dyn ProjectBoardStore>,
        clock: Arc<dyn BoardClock>,
    ) -> Result<Self, ProjectBoardError> {
        checked_writer(&*store)?;
        Ok(Self { store, clock })
    }

    /// The task `new` describes, created and updated now.
    ///
    /// # Errors
    /// A parent or dependency not on the board, or set aside; an id a
    /// task already names; the schema's refusal; an id the board holds or
    /// has set aside; the clock's; or the store's.
    pub fn execute(&self, new: &NewBoardTask) -> Result<Task, ProjectBoardError> {
        let message = format!("board: create {}", new.id.as_str());
        let tasks = commit(&*self.store, &*self.clock, &message, |board, when| {
            free(board, new)?;
            Ok(vec![
                Task::new(created_at(new, &when.now)).map_err(ProjectBoardError::Invalid)?,
            ])
        })?;
        Ok(tasks.into_iter().next().expect("a create writes its task"))
    }
}

/// The new task's fields, created and updated `now`, with no claim, runs,
/// reviews, pull requests or items yet.
fn created_at(new: &NewBoardTask, now: &Timestamp) -> TaskFields {
    let fields = TaskFields {
        id: new.id.clone(),
        title: new.title.clone(),
        kind: new.kind,
        description: new.description.clone(),
        status: new.status.into(),
        parent: new.parent.clone(),
        depends_on: new.depends_on.clone(),
        plan: new.plan.clone(),
        items: vec![],
        team: new.team.clone(),
        claim: None,
        runs: vec![],
        reviews: vec![],
        prs: vec![],
        created: now.clone(),
        updated: now.clone(),
    };
    assert!(
        matches!(fields.status, TaskStatus::Draft | TaskStatus::Ready),
        "a task is created a draft or ready"
    );
    fields
}

/// The id is free (neither held nor set aside); the parent and every
/// dependency are tasks on the board (one whose file is set aside is
/// reported as such); and the new task closes no cycle.
fn free(board: &BoardSnapshot, new: &NewBoardTask) -> Result<(), ProjectBoardError> {
    let held = |id: &Slug| board.tasks.iter().any(|task| task.fields().id == *id);
    let set_aside = |id: &Slug| {
        board
            .quarantined
            .iter()
            .any(|file| file.id.as_ref() == Some(id))
    };
    if held(&new.id) {
        return Err(ProjectBoardError::Exists(new.id.clone()));
    }
    if set_aside(&new.id) {
        return Err(ProjectBoardError::Quarantined(new.id.clone()));
    }
    for reference in new.parent.iter().chain(&new.depends_on) {
        match (held(reference), set_aside(reference)) {
            (true, _) => {}
            (false, true) => return Err(ProjectBoardError::Quarantined(reference.clone())),
            (false, false) => return Err(ProjectBoardError::UnknownReference(reference.clone())),
        }
    }
    match cycle_closer(board, new) {
        None => Ok(()),
        Some(by) => Err(ProjectBoardError::ClosesCycle {
            id: new.id.clone(),
            by,
        }),
    }
}

/// The task that would close a cycle through the new task: one reached
/// from its parent and dependencies, following each board task's own
/// parent and dependencies, that names the new id (whose earlier task was
/// deleted). The least such id, so every store order answers alike.
fn cycle_closer(board: &BoardSnapshot, new: &NewBoardTask) -> Option<Slug> {
    let on_board: BTreeMap<&Slug, &TaskFields> = board
        .tasks
        .iter()
        .map(|task| (&task.fields().id, task.fields()))
        .collect();
    let mut reached: BTreeSet<&Slug> = BTreeSet::new();
    let mut next: Vec<&Slug> = new.parent.iter().chain(&new.depends_on).collect();
    while let Some(id) = next.pop() {
        // Each id is followed once, and only when the board holds it.
        if let (true, Some(fields)) = (reached.insert(id), on_board.get(id)) {
            next.extend(fields.parent.iter().chain(&fields.depends_on));
        }
    }
    reached
        .into_iter()
        .filter_map(|id| on_board.get(id))
        .filter(|fields| {
            fields.parent.as_ref() == Some(&new.id) || fields.depends_on.contains(&new.id)
        })
        .map(|fields| fields.id.clone())
        .min()
}

#[cfg(test)]
#[path = "create_board_task_tests.rs"]
mod tests;
