//! The pointers a task makes to other tasks (its parent and dependencies)
//! as a change is about to write them (not its `references`, which point
//! anywhere and are the domain's to judge): each a task on the board, none set aside, and no
//! cycle closed through the board, by parent or by dependency.
use std::collections::{BTreeMap, BTreeSet};

use super::board_view;
use crate::application::project_board::dto::ProjectBoardError;
use crate::application::project_board::ports::BoardSnapshot;
use crate::domain::project_board::entities::task::TaskFields;
use crate::domain::project_board::value_objects::slug::Slug;

/// The pointers task `id` is to make.
pub(super) struct Pointers<'a> {
    pub id: &'a Slug,
    pub parent: Option<&'a Slug>,
    pub depends_on: &'a [Slug],
}

/// The parent and every dependency are tasks on the board (one whose file
/// is set aside is reported as such), and they close no cycle through the
/// board. Task `id`'s own entry on the board, if any, is the one being
/// replaced, so it is not walked.
pub(super) fn pointers_hold(
    board: &BoardSnapshot,
    refs: &Pointers<'_>,
) -> Result<(), ProjectBoardError> {
    // A pointer to itself is the task schema's to refuse, in its own words
    // (#2659): only pointers to other tasks are looked up.
    let others = refs.parent.into_iter().chain(refs.depends_on);
    for reference in others.filter(|reference| *reference != refs.id) {
        match board_view::task(board, reference) {
            Ok(_) => {}
            Err(ProjectBoardError::NotFound(_)) => {
                return Err(ProjectBoardError::UnknownReference(reference.clone()));
            }
            Err(refused) => return Err(refused),
        }
    }
    match cycle_closer(board, refs) {
        None => Ok(()),
        Some(by) => Err(ProjectBoardError::ClosesCycle {
            id: refs.id.clone(),
            by,
        }),
    }
}

/// The task that would close a cycle through task `id`. Parents and
/// dependencies are separate relations, each checked on its own:
/// - a dependency cycle: a task reached by following `depends_on` from
///   `id`'s dependencies whose own `depends_on` names `id`;
/// - a parent cycle: a task up `id`'s parent chain whose parent is `id`.
///
/// The least such id, so every store order answers alike.
fn cycle_closer(board: &BoardSnapshot, refs: &Pointers<'_>) -> Option<Slug> {
    let on_board: BTreeMap<&Slug, &TaskFields> = board
        .tasks
        .iter()
        .map(|task| (&task.fields().id, task.fields()))
        .filter(|(id, _)| *id != refs.id)
        .collect();
    let by_dependency = dependency_closers(&on_board, refs);
    let by_parent = parent_closer(&on_board, refs);
    by_dependency.into_iter().chain(by_parent).min().cloned()
}

/// Board tasks reached through `depends_on` from `id`'s dependencies that
/// depend on `id`. Each task is expanded once, so the walk ends within the
/// board's size, cycles on the board included.
fn dependency_closers<'b>(
    on_board: &BTreeMap<&'b Slug, &'b TaskFields>,
    refs: &Pointers<'_>,
) -> Vec<&'b Slug> {
    let mut expanded: BTreeSet<&Slug> = BTreeSet::new();
    let mut next: Vec<&Slug> = refs.depends_on.iter().collect();
    let mut closers = Vec::new();
    while let Some(id) = next.pop() {
        if let Some((&id, fields)) = on_board.get_key_value(id)
            && expanded.insert(id)
        {
            if fields.depends_on.contains(refs.id) {
                closers.push(id);
            }
            next.extend(&fields.depends_on);
        }
    }
    assert!(
        expanded.len() <= on_board.len(),
        "each task is expanded once"
    );
    closers
}

/// The board task up `id`'s parent chain whose parent is `id`. The chain
/// is followed while it stays on the board and new, so it ends within the
/// board's size, a parent loop on the board included.
fn parent_closer<'b>(
    on_board: &BTreeMap<&'b Slug, &'b TaskFields>,
    refs: &Pointers<'_>,
) -> Option<&'b Slug> {
    let mut seen: BTreeSet<&Slug> = BTreeSet::new();
    let mut at = refs.parent;
    while let Some((&id, fields)) = at.and_then(|id| on_board.get_key_value(id)) {
        match (seen.insert(id), fields.parent.as_ref()) {
            (true, Some(parent)) if parent == refs.id => return Some(id),
            (true, parent) => at = parent,
            (false, _) => break,
        }
    }
    assert!(seen.len() <= on_board.len(), "each task is seen once");
    None
}
