//! Editing a draft in place: its title, description, plan, parent and
//! dependencies. A draft has no holder (a claim starts when it is claimed,
//! after it is ready), so anyone with access to the board edits it; a task
//! past its draft changes only through its moves.
use super::super::entities::task::Task;
use super::super::entities::task_parts::PlanStep;
use super::super::value_objects::rule_error::BoardRuleError;
use super::super::value_objects::slug::Slug;
use super::super::value_objects::timestamp::Timestamp;
use super::super::value_objects::vocabulary::TaskStatus;
use super::claims::{settle, stamp};

/// What an edit sets; `None` keeps the field as it is. `parent:
/// Some(None)` takes the parent away.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DraftEdit {
    pub title: Option<String>,
    pub description: Option<String>,
    pub plan: Option<Vec<PlanStep>>,
    pub parent: Option<Option<Slug>>,
    pub depends_on: Option<Vec<Slug>>,
}

/// The draft with `edit` applied, stamped `now`.
///
/// # Errors
/// A task that is not a draft, an edit that changes nothing (sets nothing,
/// or sets each field to what it already holds), or a result the schema
/// refuses.
pub fn edit_draft(task: &Task, edit: &DraftEdit, now: &Timestamp) -> Result<Task, BoardRuleError> {
    let status = task.fields().status;
    if status != TaskStatus::Draft {
        return Err(BoardRuleError::NotDraft(status));
    }
    let mut next = task.fields().clone();
    if let Some(title) = &edit.title {
        next.title = title.clone();
    }
    if let Some(description) = &edit.description {
        next.description = description.clone();
    }
    if let Some(plan) = &edit.plan {
        next.plan = plan.clone();
    }
    if let Some(parent) = &edit.parent {
        next.parent = parent.clone();
    }
    if let Some(depends_on) = &edit.depends_on {
        next.depends_on = depends_on.clone();
    }
    match next == *task.fields() {
        true => Err(BoardRuleError::NoEdit),
        false => settle(next, &stamp(task, now)),
    }
}

#[cfg(test)]
#[path = "drafts_tests.rs"]
mod tests;
